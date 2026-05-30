# Hydra Architecture Design Document

**Version**: 0.1.0-draft  
**Date**: 2026-05-30  
**Status**: Draft — open for review  

---

## 1. Design Principles

| # | Principle | Rationale |
|---|-----------|-----------|
| P1 | **All agents share the same trait** | Orchestrator, Worker, Reviewer implement the same `Agent` trait. No special-cased control flow. Capabilities differ via channel topology and tool sets, not via trait hierarchy. |
| P2 | **Single direction of control** | ResourceManager → Agent (spawn/send_command). Agent → ResourceManager (events only, via one-way broadcast). No Agent directly mutates another Agent's state or holds another Agent's receiver. |
| P3 | **Workspace isolation by default** | Every ExecutionAgent runs in its own git worktree. Two Agents never share a working directory. |
| P4 | **LLM-driven decisions where possible** | Orchestrator uses a strong model for scheduling decisions instead of hard-coded rules. Rules are only safety floors. |
| P5 | **Replaceable internals** | QualityGate, Provider, Tool, GitWorktreeManager are all trait-based. Swap implementations without touching orchestration logic. Test with fakes. |

---

## 2. System Topology

```
                         ┌──────────────────────────┐
                         │     ResourceManager       │  ← Pure resource router
                         │  (branches / worktrees    │
                         │   IDs / event fan-out)   │
                         │                          │
                         │  agents: AgentRegistry   │
                         │  event_rx: Receiver      │  ← receives all events
                         │  event_bus: Sender       │  ← fans out to subscribers
                         │  control_senders: Map    │  ← routes commands to agents
                         └──────────┬───────────────┘
                                    │
              ┌─────────┬──────────┼──────────┬─────────┐
              │ spawn   │ send_    │ subscribe │  gc /   │
              │ agent   │ command  │  events   │ cleanup │
              ▼         ▼          ▼           ▼         ▼
         ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐
         │Agent #1│ │Agent #2│ │Agent #3│ │Agent #4│ │Agent #5│ ...
         │Worker  │ │Worker  │ │Orchest.│ │Reviewer│ │Custom  │
         └───┬────┘ └───┬────┘ └───┬────┘ └───┬────┘ └───┬────┘
             │          │          │          │          │
             │  event_tx (AgentEvent)         │          │
             └──────────┼───────────────────────┘          │
                        │                                    │
                        ▼                                    │
              ┌───────────────────┐                        │
              │   ResourceManager │                        │
              │   event_rx        │                        │
              └─────────┬─────────┘                        │
                        │ fan-out                           │
            ┌───────────┼────────────┐                     │
            │           │            │                     │
            ▼           ▼            ▼                     │
     ┌──────────┐ ┌──────────┐ ┌──────────┐               │
     │Subscriber│ │Subscriber│ │Subscriber│               │
     │  (TUI)   │ │  (REST)  │ │  (WS)    │               │
     └──────────┘ └──────────┘ └──────────┘               │
                                                             │
                                          ┌─────────────────┘
                                          ▼
                               ┌──────────────────────┐
                               │   GitWorktreeSafety   │  ← from ISO-Framework
                               │  (create / delete /   │
                               │   gc / conflict guard)│
                               └──────────────────────┘
```

---

## 3. Core Abstractions

### 3.1 Agent Trait (universal interface)

```rust
/// Every autonomous unit in the system implements this trait.
/// AgentKind distinguishes behaviour; the lifecycle is identical.
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> AgentId;
    fn kind(&self) -> AgentKind;
    fn state(&self) -> &AgentState;          // read-only snapshot
    fn branch(&self) -> Option<&str>;        // git branch, if any
    fn worktree(&self) -> Option<&Path>;     // worktree path, if any

    /// Main loop. Runs until Completed, Killed, or Failed.
    /// ResourceHandle is cloned into the agent so it can be passed across spawned tasks.
    async fn run(&mut self, resources: ResourceHandle) -> anyhow::Result<AgentOutcome>;

    /// Handle a command from ResourceManager. Returns true if the agent
    /// acknowledged and will act on it; false if rejected.
    async fn on_command(&mut self, cmd: AgentCommand) -> AgentResponse;
}
```

**Design rationale**: A single trait eliminates the "orchestrator is special" anti-pattern.
Any new agent type (Tester, Deployer, Reviewer) plugs in without changing ResourceManager.

### 3.2 LlmProvider (trait abstraction)

```rust
/// Abstraction over all LLM backends. Implementations: OpenAI, Anthropic, Ollama, etc.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a completion request. Returns the model's raw text response.
    async fn complete(&self, req: CompletionRequest) -> anyhow::Result<CompletionResponse>;

    /// Stream completion tokens (for real-time UI updates). Optional.
    async fn stream(&self, req: CompletionRequest) -> anyhow::Result<impl Stream<Item = String>>;
}

pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: usize,
    pub tools: Vec<ToolDef>,          // optional, for function calling
    pub tool_choice: ToolChoice,
}

pub struct CompletionResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
}
```

**Mock example**:
```rust
struct FakeProvider;
#[async_trait]
impl LlmProvider for FakeProvider {
    async fn complete(&self, _req: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        Ok(CompletionResponse { content: "mocked".into(), tool_calls: vec![], usage: Default::default() })
    }
}
```

### 3.3 GitWorktreeManager (trait abstraction)

```rust
/// Abstraction over git worktree lifecycle. Default: ISO-Framework adapter.
#[async_trait]
pub trait GitWorktreeManager: Send + Sync {
    async fn create(&self, branch: &str, path: &Path, base: &str) -> anyhow::Result<WorktreeHandle>;
    async fn delete(&self, handle: &WorktreeHandle) -> anyhow::Result<()>;
    async fn list(&self) -> anyhow::Result<Vec<WorktreeInfo>>;
    async fn gc(&self) -> anyhow::Result<GcReport>;
}

pub struct WorktreeHandle {
    pub branch: String,
    pub path: PathBuf,
}
```

**Mock example**:
```rust
struct FakeGit;
#[async_trait]
impl GitWorktreeManager for FakeGit {
    async fn create(&self, branch: &str, path: &Path, _base: &str) -> anyhow::Result<WorktreeHandle> {
        Ok(WorktreeHandle { branch: branch.into(), path: path.to_path_buf() })
    }
    // ...
}
```

### 3.4 AgentId and AgentKind

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct AgentId(pub u64);  // globally unique, monotonically increasing

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AgentKind {
    Execution,       /// Modifies code, runs tools, produces diffs
    Orchestrator,    /// Spawns other agents, makes scheduling decisions
    Reviewer,        /// Inspects code / diffs, produces quality scores
    Custom(String),  // extension point
}

/// Opaque handle passed to agents so they can interact with ResourceManager.
/// Agents hold an Arc<ResourceHandle>; ResourceManager holds the underlying channels.
#[derive(Clone)]
pub struct ResourceHandle {
    pub event_tx: mpsc::UnboundedSender<AgentEvent>,
    pub control_tx: mpsc::UnboundedSender<(AgentId, AgentCommand)>,
    pub git: Arc<dyn GitWorktreeManager>,
    pub providers: Arc<RwLock<ProviderRegistry>>,
    pub tool_registry: Arc<RwLock<ToolRegistry>>,
}

/// Response from an agent after receiving a command.
#[derive(Debug, Clone, Serialize)]
pub enum AgentResponse {
    Ack,
    Reject { reason: String },
}
```

### 3.5 AgentState (shared, observable)

```rust
#[derive(Debug, Clone, Serialize)]
pub struct AgentState {
    pub id: AgentId,
    pub kind: AgentKind,
    pub status: AgentStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize)]
pub enum AgentStatus {
    Created,
    Running { turn: usize, max_turns: usize },
    Paused,
    Completed { outcome: AgentOutcome },
    Killed { reason: String, at: Timestamp },
    Failed { error: String },
}
```

`AgentState` is stored as `Arc<RwLock<AgentState>>`. ResourceManager's AgentRegistry holds clones of the same Arc for observation. **The agent itself is the sole writer** to its own state (updating status, turn count, timestamps). Subscribers and ResourceManager hold read-only snapshots or `Weak` references. No component other than the owning agent calls `write()` on its state.

### 3.6 AgentEvent (unified event stream)

```rust
#[derive(Debug, Clone, Serialize)]
pub enum AgentEvent {
    // Lifecycle (all agents)
    Started   { agent_id: AgentId, kind: AgentKind },
    Completed { agent_id: AgentId, outcome: AgentOutcome },
    Killed    { agent_id: AgentId, reason: String },
    Failed    { agent_id: AgentId, error: String },

    // ExecutionAgent
    Turn      { agent_id: AgentId, turn: TurnEvent },
    ToolCall  { agent_id: AgentId, tool: String, success: bool },

    // Orchestrator
    Decision  { agent_id: AgentId, decisions: Vec<HarnessCommand> },
    TaskSpawned { agent_id: AgentId, child_id: AgentId, desc: String },

    // Reviewer
    ReviewResult { agent_id: AgentId, score: f64, verdict: String },
}
```

### 3.7 AgentCommand (control interface)

```rust
#[derive(Debug, Clone, Serialize)]
pub enum AgentCommand {
    // Universal
    Kill    { reason: String },
    Pause,
    Resume,

    // ExecutionAgent
    InjectHint { text: String },
    SwitchModel { provider: String, model: String },

    // Orchestrator
    SubmitTask { description: String },
    SetPolicy  { policy: Policy },

    // Reviewer
    ReviewTarget { branch: String, scope: Vec<String> },
}
```

### 3.8 AgentOutcome (return value from run())

```rust
#[derive(Debug, Clone, Serialize)]
pub struct AgentOutcome {
    pub agent_id: AgentId,
    pub success: bool,
    pub edited_files: Vec<String>,
    pub summary: String,
    pub metrics: AgentMetrics,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AgentMetrics {
    pub turns_used: usize,
    pub edits_made: usize,
    pub tokens_consumed: usize,
    pub duration_ms: u64,
    pub compile_passed: bool,
    pub tests_passed: bool,
}
```

---

## 4. ResourceManager (the router)

ResourceManager is the **only** component that directly addresses agents by ID.
All other components interact through channels or read-only snapshots.

```rust
pub struct ResourceManager {
    agents:         Arc<RwLock<AgentRegistry>>,
    event_rx:       mpsc::UnboundedReceiver<AgentEvent>,
    event_bus:      mpsc::UnboundedSender<AgentEvent>,  // cloned from event_rx's half
    subscribers:    Arc<RwLock<Vec<mpsc::UnboundedSender<AgentEvent>>>>,
    control_senders: Arc<RwLock<HashMap<AgentId, mpsc::UnboundedSender<AgentCommand>>>>,
    next_id:        AtomicU64,
    // Shared infrastructure (cloned into ResourceHandle for each agent)
    git:            Arc<dyn GitWorktreeManager>,
    providers:      Arc<RwLock<ProviderRegistry>>,
    tool_registry:  Arc<RwLock<ToolRegistry>>,
}

impl ResourceManager {
    /// Spawn a new agent. Returns AgentId and a handle for observation.
    pub fn spawn(&self, kind: AgentKind, spec: AgentSpec) -> Result<AgentHandle> {
        let id = AgentId(self.next_id.fetch_add(1, Ordering::SeqCst));

        // Create agent with cloned ResourceHandle (agent owns its channels)
        let handle = self.build_handle(id, kind, spec)?;

        // Register the agent's control sender so send_command can route to it
        self.control_senders.write().unwrap().insert(id, handle.control_tx.clone());

        // Start the agent's run loop in a background task
        let rm_handle = self.spawn_handle.clone();
        let event_tx = self.event_bus.clone();
        tokio::spawn(async move {
            let mut agent = handle.agent;
            let outcome = agent.run(handle.resources).await;
            event_tx.send(AgentEvent::Completed { agent_id: id, outcome }).ok();
        });

        Ok(AgentHandle { id, state: /* ... */ })
    }

    /// Send a command to a specific agent.
    pub fn send_command(&self, id: AgentId, cmd: AgentCommand) -> Result<()> {
        let senders = self.control_senders.read().unwrap();
        senders.get(&id)
            .ok_or_anyhow!("agent {} not found or already terminated", id)?
            .send(cmd)
            .map_err(|_| anyhow!("agent {} control channel closed", id))
    }

    /// Subscribe to all agent events. Returns a receiver.
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<AgentEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.subscribers.write().unwrap().push(tx);
        rx
    }

    /// Get a read-only snapshot of an agent's state.
    pub fn snapshot(&self, id: AgentId) -> Option<AgentSnapshot>;

    /// Get snapshots of all agents.
    pub fn snapshots(&self) -> Vec<AgentSnapshot>;

    /// Trigger cleanup for a completed/killed agent.
    pub fn cleanup(&self, id: AgentId) -> Result<()>;
}
```

**What ResourceManager does NOT do:**
- Does not decide which agent to spawn
- Does not decide which agent to kill
- Does not hold any Agent's Conversation or TurnRunner
- Does not parse LLM output

---

## 5. Agent Implementations

### 5.1 ExecutionAgent (the worker)

Responsible for: reading files, editing code, running tools, reporting results.

```rust
pub struct ExecutionAgent {
    id:          AgentId,
    branch:      String,
    worktree:    PathBuf,
    conversation: Conversation,        // independent LLM context
    runner:      TurnRunner,           // LLM call loop
    tool_ctx:    ToolContext,          // cwd = worktree
    tool_reg:    ToolRegistry,         // scoped to assigned files
    state:       Arc<RwLock<AgentState>>,
    event_tx:    mpsc::UnboundedSender<AgentEvent>,
    control_rx:  mpsc::UnboundedReceiver<AgentCommand>,
}
```

**Tool scoping** (from atomcode's `ScopedReadFile` pattern):  
ExecutionAgent only has tools relevant to its assigned files. If assigned `Service.java`,
`read_file` is wrapped to reject any path other than `Service.java`.

### 5.2 OrchestratorAgent (the scheduler)

Responsible for: parsing user tasks, spawning ExecutionAgents, evaluating outcomes,
deciding promote/kill/retry.

```rust
pub struct OrchestratorAgent {
    id:             AgentId,
    conversation:   Conversation,       // its own LLM context
    runner:         TurnRunner,
    state:          Arc<RwLock<AgentState>>,
    event_rx:       mpsc::UnboundedReceiver<AgentEvent>,  // all events
    control_rx:     mpsc::UnboundedReceiver<AgentCommand>,
    resources:      ResourceHandle,     // to spawn new agents
    policy:         Arc<RwLock<Policy>>,
}
```

**Key difference from ExecutionAgent**: its `ToolRegistry` contains meta-tools:
- `inspect_agent(agent_id)` → AgentSnapshot
- `spawn_execution(task, files, model)` → AgentId
- `kill_agent(agent_id, reason)` → bool
- `promote_branch(agent_id, to_branch)` → bool
- `declare_complete(summary)` → terminates itself

It does NOT have `edit_file`, `bash`, `read_file`. It orchestrates, it doesn't execute.

### 5.3 ReviewerAgent (optional, pluggable)

Responsible for: inspecting a branch's diff, running tests, producing a quality score.

```rust
pub struct ReviewerAgent {
    id:          AgentId,
    branch:      String,
    worktree:    PathBuf,
    conversation: Conversation,
    runner:      TurnRunner,
    tool_ctx:    ToolContext,
    state:       Arc<RwLock<AgentState>>,
    event_tx:    mpsc::UnboundedSender<AgentEvent>,
    control_rx:  mpsc::UnboundedReceiver<AgentCommand>,
}
```

Can be LLM-based (Claude reviewing code) or rule-based (clippy + test suite).

---

## 6. Event Flow

```
┌──────────────────────────────────────────────────────────────┐
│  ExecutionAgent #1 (modifying Service.java)                  │
│  event_tx ────────────────────────────────────────┐          │
└───────────────────────────────────────────────────┼──────────┘
                                                    │
                                                    ▼
                                          ┌──────────────────┐
                                          │   Event Bus       │
                                          │   (fan-out to     │
                                          │    all subscribers)│
                                          └────────┬─────────┘
                                                   │
                      ┌────────────────────────────┼────────────────────────┐
                      │                            │                        │
                      ▼                            ▼                        ▼
               ┌──────────────┐          ┌──────────────────┐      ┌──────────────┐
               │ Orchestrator  │          │   REST Server     │      │  TUI / CLI    │
               │ Agent         │          │   (daemon)        │      │               │
               │               │          │                   │      │               │
               │ Receives ALL  │          │ Serves snapshots  │      │ Shows worker  │
               │ events via    │          │ to VSCode ext +   │      │ status in     │
               │ event_rx      │          │ Web Dashboard     │      │ terminal      │
               └───────┬───────┘          └──────────────────┘      └──────────────┘
                       │
                       │  Decision: kill #2, promote #1
                       │
                       ▼
               ┌──────────────┐
               │ send_command  │
               │ (via Resource │
               │  Manager)     │
               └───────┬───────┘
                       │
                       ▼
               ┌──────────────┐
               │  Resource     │
               │  Manager      │
               │  routes to    │
               │  target agent │
               └──────────────┘
```

**Critical invariant**: Event flow is **one-way broadcast**. No agent receives another agent's event_rx.
ResourceManager holds all receivers and fans out. Agents only hold `event_tx` (sender).

---

## 7. Git Worktree Isolation

Fork **ISO-Framework** (snehith01001110/ISO-Framework) as the foundation.

```
repo-root/
├── .git/
├── src/
├── ...
└── .worktrees/                    ← managed by ISO-Framework
    ├── wt-agent-1/                ← ExecutionAgent #1
    │   └── (on branch agent/1)
    ├── wt-agent-2/                ← ExecutionAgent #2
    │   └── (on branch agent/2)
    └── wt-reviewer-3/             ← ReviewerAgent #3
        └── (on branch agent/3)
```

Each worktree is an independent checkout with its own branch.  
Agents read/write only within their own worktree.  
ISO-Framework guarantees:
- No deletion of unmerged branches
- No orphaned worktrees (GC)
- Crash-safe state files
- No nested worktree creation

---

## 8. Crate Structure

```
hydra/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── hydra-core/             # ⭐ central crate, all others depend on this
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── agent.rs        # Agent trait, AgentId, AgentKind, AgentState
│   │   │   ├── event.rs        # AgentEvent, AgentCommand, AgentResponse
│   │   │   ├── outcome.rs      # AgentOutcome, AgentMetrics
│   │   │   ├── resource.rs     # ResourceManager, ResourceHandle, AgentRegistry
│   │   │   ├── execution.rs    # ExecutionAgent
│   │   │   ├── orchestrator.rs # OrchestratorAgent
│   │   │   ├── reviewer.rs     # ReviewerAgent (optional)
│   │   │   ├── provider/       # LlmProvider trait + implementations
│   │   │   │   ├── mod.rs
│   │   │   │   ├── openai.rs   # OpenAI / DeepSeek / 硅基流动
│   │   │   │   ├── anthropic.rs # Claude
│   │   │   │   └── ollama.rs    # Local models
│   │   │   ├── tools/          # Tool trait + built-in tools
│   │   │   │   ├── mod.rs
│   │   │   │   ├── edit.rs
│   │   │   │   ├── bash.rs
│   │   │   │   ├── read.rs
│   │   │   │   └── ...
│   │   │   └── git.rs          # GitWorktreeManager (ISO-Framework adapter)
│   │   └── tests/
│   │
│   ├── hydra-daemon/           # HTTP + WebSocket server
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── api.rs          # axum REST handlers
│   │   │   ├── ws.rs           # WebSocket event stream
│   │   │   └── snapshot.rs     # AgentState → JSON serialization
│   │   └── tests/
│   │
│   ├── hydra-cli/              # Command-line interface
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── commands.rs     # serve, run, workers, open, promote, kill
│   │   │   └── tui.rs          # terminal UI (optional, based on tuix)
│   │   └── tests/
│   │
│   ├── hydra-telemetry/        # Logging, tracing, datalog
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── datalog.rs      # per-agent turn logging (markdown)
│   │       └── trace.rs        # OpenTelemetry integration
│   │
│   └── hydra-workspace/        # ⭐ fork of ISO-Framework
│       ├── Cargo.toml
│       ├── src/
│       │   ├── lib.rs
│       │   ├── manager.rs      # ISO-Framework Manager wrapper
│       │   ├── adapter.rs      # Hydra types → ISO-Framework types
│       │   └── safety.rs       # SafetyPolicy (dry-run, protected branches)
│       └── tests/
│
├── extensions/
│   └── vscode/                 # VSCode extension
│       ├── package.json
│       └── src/
│           ├── workerMonitor.ts
│           ├── stateApi.ts
│           └── commands.ts
│
└── dashboard/
    └── index.html              # Web dashboard (served by hydra-daemon)
```

### Dependency graph

```
hydra-cli ──────────────────┐
hydra-daemon ───────────────┤
extensions/vscode (HTTP) ───┤
dashboard (WS) ─────────────┤
                            ▼
                    ┌───────────────┐
                    │  hydra-core   │
                    └───────┬───────┘
                            │
              ┌─────────────┼─────────────┐
              │             │             │
              ▼             ▼             ▼
        hydra-workspace  providers/   tools/
        (ISO-Framework   (OpenAI/     (edit/bash/
         fork)            Anthropic/   read/...)
                           Ollama)
```

**Rule**: hydra-core depends on nothing above it. All other crates depend on hydra-core.
hydra-workspace depends only on hydra-core types + ISO-Framework library.

---

## 9. Interface Contracts (detailed)

### 9.1 ResourceManager ↔ Agent

```
ResourceManager → Agent:
  AgentCommand::Kill { reason }
  AgentCommand::Pause
  AgentCommand::Resume
  AgentCommand::InjectHint { text }
  AgentCommand::SwitchModel { provider, model }
  AgentCommand::SubmitTask { description }
  AgentCommand::SetPolicy { policy }
  AgentCommand::ReviewTarget { branch, scope }

Agent → ResourceManager:
  AgentEvent::Started / Completed / Killed / Failed
  AgentEvent::Turn { turn: TurnEvent }
  AgentEvent::ToolCall { tool, success }
  AgentEvent::Decision { decisions: Vec<HarnessCommand> }
  AgentEvent::TaskSpawned { child_id, desc }
  AgentEvent::ReviewResult { score, verdict }
```

### 9.2 ExecutionAgent ↔ ToolRegistry

```
ExecutionAgent → ToolRegistry:
  registry.get("edit_file") → &dyn Tool
  registry.get("bash") → &dyn Tool
  registry.get("read_file") → &dyn Tool (scoped)

ToolRegistry → ExecutionAgent:
  ToolResult { call_id, output, success, duration }
  (returned from tool.execute(), added to conversation)
```

### 9.3 OrchestratorAgent ↔ ResourceManager

```
OrchestratorAgent → ResourceManager:
  resources.spawn(AgentKind::Execution, spec) → AgentId
  resources.send_command(child_id, AgentCommand::Kill { ... })
  resources.snapshot(child_id) → Option<AgentSnapshot>
  resources.snapshots() → Vec<AgentSnapshot>

ResourceManager → OrchestratorAgent:
  AgentEvent (all events from all agents, via event_rx)
```

### 9.4 StateServer ↔ Clients

```
REST:
  GET    /api/v1/agents              → Vec<AgentSnapshot>
  GET    /api/v1/agents/:id         → AgentSnapshot
  GET    /api/v1/agents/:id/events  → Vec<AgentEvent>
  POST   /api/v1/agents/:id/open    → { worktree_path: String }
  POST   /api/v1/agents/:id/kill    → { success: bool }
  POST   /api/v1/agents/:id/promote → { success: bool, branch: String }
  GET    /api/v1/worktrees           → Vec<WorktreeInfo>
  GET    /api/v1/branches            → Vec<BranchInfo>

WebSocket:
  Server → Client: AgentEvent (real-time)
  Client → Server: { action: "kill", agent_id: N }
                  { action: "open_vscode", agent_id: N }
                  { action: "subscribe" }
```

### 9.5 HydraWorkspace ↔ ISO-Framework

```
Hydra types:
  AgentId, AgentKind, AgentState
  AgentSpec (branch, worktree, base_branch)

ISO-Framework types (wrapped, never leaked to core):
  iso_code::Manager
  iso_code::Handle
  iso_code::CreateOptions
  iso_code::DeleteOptions
  iso_code::GcOptions

Adapter layer (hydra-workspace/src/adapter.rs):
  AgentId → branch name:  format!("agent/{}", id.0)
  AgentSpec → CreateOptions
  AgentOutcome → determines whether to delete or retain worktree
```

---

## 10. Failure Modes and Guarantees

| Failure | Guarantee | Mechanism |
|---------|-----------|-----------|
| Agent panics | Event emitted, worktree retained for inspection | tokio::spawn + JoinHandle::await |
| Agent hangs (no progress) | Killed after configurable timeout | Orchestrator's idle detection (rule floor) |
| Worktree corruption | No data loss on main branch | ISO-Framework: never deletes unmerged branches |
| Orphaned worktrees | GC on cleanup | ISO-Framework::gc() |
| Network failure (LLM) | Retry with backoff, then kill | TurnRunner retry logic (from atomcode) |
| Git merge conflict | Agent killed, conflict preserved | ISO-Framework conflict detection |
| ResourceManager panic | Agents continue running (they hold event_tx sender) | Agents send events via sender; RM owns event_rx but agents don't depend on RM after spawn |

---

## 11. Extension Points

| Extension | How |
|-----------|-----|
| New AgentKind | Implement `Agent` trait, register in `ResourceManager::spawn()` |
| New Provider | Implement `LlmProvider`, register in `ProviderRegistry` |
| New Tool | Implement `Tool`, register in `ToolRegistry` |
| New Event Subscriber | Call `resource_manager.subscribe()`, process `AgentEvent` stream |
| New Frontend | Connect to StateServer's REST + WS — no core changes needed |
| New Quality Policy | Implement `Policy` trait, pass to `AgentCommand::SetPolicy` |

---

## 12. Implementation Sequence

```
Phase 0 (1 week): Foundations
  - Fork ISO-Framework → hydra-workspace
  - Define Agent trait + AgentId + AgentState + AgentEvent
  - Define ResourceManager (minimal: spawn + send_command + subscribe)
  - Define AgentCommand + AgentOutcome

Phase 1 (1 week): ExecutionAgent
  - Wrap SubAgentTask → ExecutionAgent
  - Tool scoping (ScopedReadFile pattern)
  - ToolContext with worktree cwd
  - TurnEvent → AgentEvent mapping

Phase 2 (1 week): OrchestratorAgent
  - Orchestrator with meta-tools (inspect/spawn/kill/promote)
  - System prompt + tool definitions
  - Basic policy (rule-based quality gate as fallback)
  - Subtask decomposition (SubtaskDriver integration)

Phase 3 (1 week): Visibility
  - StateServer (REST + WS)
  - hydra-cli: serve, run, workers, open
  - AgentSnapshot serialization
  - VSCode extension: tree view + open worktree

Phase 4 (1 week): Polish
  - QualityGate trait (replace rule-based with LLM-based)
  - Multi-model fallback
  - Error handling + telemetry
  - Documentation
```

---

## 13. Risks and Open Questions

| # | Risk | Mitigation | Status |
|---|------|-----------|--------|
| R1 | ISO-Framework API changes before we fork | Fork immediately at a stable commit | Open |
| R2 | Orchestrator LLM makes bad kill decisions | Rule-based safety floors (min_turns_before_kill) | Design phase |
| R3 | Token cost of Orchestrator context growing | Context budget + event summarization | Design phase |
| R4 | Windows git worktree support | ISO-Framework claims cross-platform; verify on WSL | Open |
| R5 | Concurrent git operations (two agents committing simultaneously) | Sequential commit via ResourceManager lock | Design phase |
