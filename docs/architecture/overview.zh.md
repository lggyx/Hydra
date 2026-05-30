# Hydra 架构设计文档

**版本**: 0.1.0-draft  
**日期**: 2026-05-30  
**状态**: 草案 —— 开放评审  

---

## 1. 设计原则

| # | 原则 | 理由 |
|---|------|------|
| P1 | **所有 Agent 平等** | Orchestrator、Worker、Reviewer 共享同一个 `Agent` trait。不做特殊化控制流。 |
| P2 | **单向控制** | ResourceManager → Agent（spawn/send_command）。Agent → ResourceManager 仅通过事件广播。没有任何 Agent 直接修改另一个 Agent 的状态。 |
| P3 | **默认工作区隔离** | 每个 ExecutionAgent 运行在独立的 git worktree 中。两个 Agent 绝不共享工作目录。 |
| P4 | **能用 LLM 决策的地方就用 LLM** | Orchestrator 用强模型做调度决策，而非硬编码规则。规则仅作为安全下限。 |
| P5 | **内部可替换** | QualityGate、Provider、ToolRegistry 都是 trait 化的。替换实现无需改动编排逻辑。 |

---

## 2. 系统拓扑

```
                          ┌──────────────────────┐
                          │     ResourceManager    │  ← 纯资源路由器
                          │  (分支 / worktree /   │
                          │   ID / 事件扇出)       │
                          └──────────┬───────────┘
                                     │
               ┌─────────┬──────────┼──────────┬─────────┐
               │ spawn   │ send_    │ subscribe │  gc /   │
               │ agent   │ command  │  events   │ cleanup │
               ▼         ▼          ▼           ▼         ▼
          ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐
          │Agent #1│ │Agent #2│ │Agent #3│ │Agent #4│ │Agent #5│ ...
          │执行者  │ │执行者  │ │编排者  │ │审查者  │ │自定义  │
          └───┬────┘ └───┬────┘ └───┬────┘ └───┬────┘ └───┬────┘
              │          │          │          │          │
              │  AgentEvent (广播)    │          │
              └──────────┬─────────────────┘          │
                         │                            │
                         ▼                            │
               ┌─────────────────────┐               │
               │   事件订阅者          │               │
               │  TUI │ REST │ WS    │               │
               │  CLI │ VSCode │ ... │               │
               └─────────────────────┘               │
                                                       │
                                     ┌─────────────────┘
                                     ▼
                          ┌──────────────────────┐
                          │   GitWorktreeSafety   │  ← 来自 ISO-Framework
                          │  (创建 / 删除 /       │
                          │   GC / 冲突保护)       │
                          └──────────────────────┘
```

---

## 3. 核心抽象

### 3.1 Agent trait（统一接口）

```rust
/// 系统中每个自治单元都实现此 trait。
/// AgentKind 区分行为；生命周期完全相同。
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> AgentId;
    fn kind(&self) -> AgentKind;
    fn state(&self) -> &AgentState;          // 只读快照
    fn branch(&self) -> Option<&str>;        // git 分支（如果有）
    fn worktree(&self) -> Option<&Path>;     // worktree 路径（如果有）

    /// 主循环。运行直到 Completed、Killed 或 Failed。
    async fn run(&mut self, resources: ResourceHandle) -> anyhow::Result<AgentOutcome>;

    /// 处理来自 ResourceManager 的指令。返回 true 表示 Agent
    /// 已确认并将执行该指令；false 表示拒绝。
    async fn on_command(&mut self, cmd: AgentCommand) -> AgentResponse;
}
```

**设计理由**: 单一 trait 消除了"Orchestrator 是特殊的"反模式。任何新的 Agent 类型（Tester、Deployer、Reviewer）都可以直接接入，无需修改 ResourceManager。

### 3.2 LlmProvider（trait 抽象）

所有 LLM 后端的抽象。实现：OpenAI、Anthropic、Ollama 等。

```rust
/// 所有 LLM 后端的抽象。实现：OpenAI、Anthropic、Ollama 等。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 发送补全请求。返回模型的原始文本响应。
    async fn complete(&self, req: CompletionRequest) -> anyhow::Result<CompletionResponse>;

    /// 流式输出补全 token（用于实时 UI 更新）。可选。
    async fn stream(&self, req: CompletionRequest) -> anyhow::Result<impl Stream<Item = String>>;
}

pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: usize,
    pub tools: Vec<ToolDef>,          // 可选，用于 function calling
    pub tool_choice: ToolChoice,
}

pub struct CompletionResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
}
```

**Mock 示例**:
```rust
struct FakeProvider;
#[async_trait]
impl LlmProvider for FakeProvider {
    async fn complete(&self, _req: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        Ok(CompletionResponse { content: "mocked".into(), tool_calls: vec![], usage: Default::default() })
    }
}
```

### 3.3 GitWorktreeManager（trait 抽象）

Git worktree 生命周期的抽象。默认实现：ISO-Framework 适配层。

```rust
/// Git worktree 生命周期的抽象。默认：ISO-Framework 适配层。
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

**Mock 示例**:
```rust
struct FakeGit;
#[async_trait]
impl GitWorktreeManager for FakeGit {
    async fn create(&self, _b: &str, _p: &Path, _base: &str) -> anyhow::Result<WorktreeHandle> {
        Ok(WorktreeHandle { branch: "main".into(), path: PathBuf::from("/tmp/fake") })
    }
    async fn delete(&self, _h: &WorktreeHandle) -> anyhow::Result<()> { Ok(()) }
    async fn list(&self) -> anyhow::Result<Vec<WorktreeInfo>> { Ok(vec![]) }
    async fn gc(&self) -> anyhow::Result<GcReport> { Ok(GcReport { reclaimed: 0 }) }
}
```

### 3.4 AgentId 和 AgentKind

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct AgentId(pub u64);  // 全局唯一，单调递增

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AgentKind {
    Execution,       /// 修改代码、运行工具、产出 diff
    Orchestrator,    /// 生成其他 Agent、做出调度决策
    Reviewer,        /// 检查代码/diff、产出质量分数
    Custom(String),  // 扩展点
}
```

### 3.5 AgentState（共享、可观测）

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

`AgentState` 以 `Arc<RwLock<>>` 形式存储在 ResourceManager 的 AgentRegistry 中，订阅者持有克隆的 Arc 只读观察。**Agent 自身是其状态的唯一写入者**（更新状态、轮次计数、时间戳）。订阅者和 ResourceManager 仅持有只读快照或 `Weak` 引用。除持有 Agent 外，没有任何组件对其状态调用 `write()`。

### 3.6 AgentEvent（统一事件流）

```rust
#[derive(Debug, Clone, Serialize)]
pub enum AgentEvent {
    // 通用生命周期（所有 Agent）
    Started   { agent_id: AgentId, kind: AgentKind },
    Completed { agent_id: AgentId, outcome: AgentOutcome },
    Killed    { agent_id: AgentId, reason: String },
    Failed    { agent_id: AgentId, error: String },

    // ExecutionAgent 特有
    Turn      { agent_id: AgentId, turn: TurnEvent },
    ToolCall  { agent_id: AgentId, tool: String, success: bool },

    // Orchestrator
    Decision  { agent_id: AgentId, decisions: Vec<HarnessCommand> },
    TaskSpawned { agent_id: AgentId, child_id: AgentId, desc: String },

    // Reviewer
    ReviewResult { agent_id: AgentId, score: f64, verdict: String },
}
```

### 3.7 AgentCommand（控制接口）

```rust
#[derive(Debug, Clone, Serialize)]
pub enum AgentCommand {
    // 通用
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

### 3.8 AgentOutcome（run() 返回值）

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

## 4. ResourceManager（路由器）

ResourceManager 是**唯一**持有 Agent 可变引用的组件。其他所有模块都是只读观察。

```rust
pub struct ResourceManager {
    agents:         Arc<RwLock<AgentRegistry>>,
    event_bus:      mpsc::UnboundedSender<AgentEvent>,
    subscribers:    Arc<RwLock<Vec<mpsc::UnboundedSender<AgentEvent>>>>,
    next_id:        AtomicU64,
    // 共享基础设施（克隆给每个 AgentHandle）
    git:            Arc<dyn GitWorktreeManager>,    // ← 来自 ISO-Framework
    providers:      Arc<RwLock<ProviderRegistry>>,
    tool_registry:  Arc<RwLock<ToolRegistry>>,
}

impl ResourceManager {
    /// 生成新 Agent。返回 AgentId 和用于观察的句柄。
    pub fn spawn(&self, kind: AgentKind, spec: AgentSpec) -> Result<AgentHandle>;

    /// 向指定 Agent 发送指令。
    pub fn send_command(&self, id: AgentId, cmd: AgentCommand) -> Result<()>;

    /// 订阅所有 Agent 事件。返回 receiver。
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<AgentEvent>;

    /// 获取某个 Agent 的只读快照。
    pub fn snapshot(&self, id: AgentId) -> Option<AgentSnapshot>;

    /// 获取所有 Agent 的快照。
    pub fn snapshots(&self) -> Vec<AgentSnapshot>;

    /// 清理已结束的 Agent 资源。
    pub fn cleanup(&self, id: AgentId) -> Result<()>;
}
```

**ResourceManager 不做什么：**
- 不决定生成哪个 Agent
- 不决定终止哪个 Agent
- 不持有任何 Agent 的 Conversation 或 TurnRunner
- 不解析 LLM 输出

---

## 5. Agent 实现

### 5.1 ExecutionAgent（执行者）

职责：读取文件、编辑代码、运行工具、汇报结果。

```rust
pub struct ExecutionAgent {
    id:          AgentId,
    branch:      String,
    worktree:    PathBuf,
    conversation: Conversation,        // 独立的 LLM 上下文
    runner:      TurnRunner,           // LLM 调用循环
    tool_ctx:    ToolContext,          // cwd = worktree
    tool_reg:    ToolRegistry,         // 限制在分配的文件范围内
    state:       Arc<RwLock<AgentState>>,
    event_tx:    mpsc::UnboundedSender<AgentEvent>,
    control_rx:  mpsc::UnboundedReceiver<AgentCommand>,
}
```

**工具范围限制**（参考 hydra 的 `ScopedReadFile` 模式）：  
ExecutionAgent 只拥有与其分配文件相关的工具。如果分配的是 `Service.java`，`read_file` 会被包装以拒绝任何非 `Service.java` 的路径。

### 5.2 OrchestratorAgent（调度者）

职责：解析用户任务、生成 ExecutionAgent、评估结果、决定 promote/kill/retry。

```rust
pub struct OrchestratorAgent {
    id:             AgentId,
    conversation:   Conversation,       // 它自己的 LLM 上下文
    runner:         TurnRunner,
    state:          Arc<RwLock<AgentState>>,
    event_rx:       mpsc::UnboundedReceiver<AgentEvent>,  // 所有事件
    control_rx:     mpsc::UnboundedReceiver<AgentCommand>,
    resources:      ResourceHandle,     // 用于生成新 Agent
    policy:         Arc<RwLock<Policy>>,
}
```

**与 ExecutionAgent 的关键区别**：它的 `ToolRegistry` 包含元工具：
- `inspect_agent(agent_id)` → AgentSnapshot
- `spawn_execution(task, files, model)` → AgentId
- `kill_agent(agent_id, reason)` → bool
- `promote_branch(agent_id, to_branch)` → bool
- `declare_complete(summary)` → 终止自身

它**没有** `edit_file`、`bash`、`read_file`。它负责编排，不负责执行。

### 5.3 ReviewerAgent（可选，可插拔）

职责：检查分支的 diff、运行测试、产出质量分数。

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

可以是 LLM 驱动（Claude 审查代码）或规则驱动（clippy + 测试套件）。

---

## 6. 事件流

```
┌──────────────────────────────────────────────────────────────┐
│  ExecutionAgent #1（修改 Service.java）                       │
│  event_tx ────────────────────────────────────────┐          │
└───────────────────────────────────────────────────┼──────────┘
                                                    │
                                                    ▼
                                          ┌──────────────────┐
                                          │   EventBus        │
                                          │   (扇出到所有      │
                                          │    订阅者)         │
                                          └────────┬─────────┘
                                                 │
                       ┌────────────────────────────┼────────────────────────┐
                       │                            │                        │
                       ▼                            ▼                        ▼
                ┌──────────────┐          ┌──────────────────┐      ┌──────────────┐
                │ Orchestrator  │          │   REST Server     │      │  TUI / CLI    │
                │ Agent         │          │   (daemon)        │      │               │
                │               │          │                   │      │               │
                │ 通过 event_rx  │          │ 向 VSCode 扩展 +  │      │ 在终端显示    │
                │ 接收所有事件   │          │ Web Dashboard     │      │ worker 状态   │
                └───────┬───────┘          └──────────────────┘      └──────────────┘
                        │
                        │  决策：kill #2，promote #1
                        │
                        ▼
                ┌──────────────┐
                │ send_command  │
                │ (通过 Resource │
                │  Manager)     │
                └───────┬───────┘
                        │
                        ▼
                ┌──────────────┐
                │  Resource     │
                │  Manager      │
                │  路由到目标    │
                │  Agent        │
                └──────────────┘
```

**关键不变式**：事件流是**单向广播**。没有任何 Agent 持有另一个 Agent 的 event_rx。ResourceManager 持有所有 receiver 并负责扇出。Agent 只持有 `event_tx`（sender）。

---

## 7. Git Worktree 隔离

Fork **ISO-Framework**（snehith01001110/ISO-Framework）作为基础。

```
repo-root/
├── .git/
├── src/
├── ...
└── .worktrees/                    ← 由 ISO-Framework 管理
    ├── wt-agent-1/                ← ExecutionAgent #1
    │   └── (在分支 agent/1 上)
    ├── wt-agent-2/                ← ExecutionAgent #2
    │   └── (在分支 agent/2 上)
    └── wt-reviewer-3/             ← ReviewerAgent #3
        └── (在分支 agent/3 上)
```

每个 worktree 是独立的 checkout，拥有自己的分支。  
Agent 只能读写自己 worktree 内的文件。  
ISO-Framework 保证：
- 不删除未合并的分支
- 不留下孤立的 worktree（GC）
- crash-safe 状态文件
- 不创建嵌套 worktree

---

## 8. Crate 结构

```
hydra/
├── Cargo.toml                  # workspace 根
├── crates/
│   ├── hydra-core/             # ⭐ 核心 crate，其他所有 crate 依赖它
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── agent.rs        # Agent trait、AgentId、AgentKind、AgentState
│   │   │   ├── event.rs        # AgentEvent、AgentCommand、AgentResponse
│   │   │   ├── outcome.rs      # AgentOutcome、AgentMetrics
│   │   │   ├── resource.rs     # ResourceManager、ResourceHandle、AgentRegistry
│   │   │   ├── execution.rs    # ExecutionAgent
│   │   │   ├── orchestrator.rs # OrchestratorAgent
│   │   │   ├── reviewer.rs     # ReviewerAgent（可选）
│   │   │   ├── provider/       # LlmProvider trait + 实现
│   │   │   │   ├── mod.rs
│   │   │   │   ├── openai.rs   # OpenAI / DeepSeek
│   │   │   │   ├── anthropic.rs # Claude
│   │   │   │   └── ollama.rs    # 本地模型
│   │   │   ├── tools/          # Tool trait + 内置工具
│   │   │   │   ├── mod.rs
│   │   │   │   ├── edit.rs
│   │   │   │   ├── bash.rs
│   │   │   │   ├── read.rs
│   │   │   │   └── ...
│   │   │   └── git.rs          # GitWorktreeManager（ISO-Framework 适配层）
│   │   └── tests/
│   │
│   ├── hydra-daemon/           # HTTP + WebSocket 服务
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── api.rs          # axum REST handlers
│   │   │   ├── ws.rs           # WebSocket 事件流
│   │   │   └── snapshot.rs     # AgentState → JSON 序列化
│   │   └── tests/
│   │
│   ├── hydra-cli/              # 命令行入口
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── commands.rs     # serve、run、workers、open、promote、kill
│   │   │   └── tui.rs          # 终端 UI（可选，基于 tuix）
│   │   └── tests/
│   │
│   ├── hydra-telemetry/        # 日志、追踪、datalog
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── datalog.rs      # 每 Agent 每轮对话日志（markdown）
│   │       └── trace.rs        # OpenTelemetry 集成
│   │
│   └── hydra-workspace/        # ⭐ ISO-Framework fork（worktree 安全）
│       ├── Cargo.toml
│       ├── src/
│       │   ├── lib.rs
│       │   ├── manager.rs      # HydraWorkspaceManager 封装 ISO-Framework Manager
│       │   ├── adapter.rs      # Hydra 类型 ↔ ISO-Framework 类型
│       │   └── safety.rs       # Hydra 专属安全策略
│       └── tests/
│
├── extensions/
│   └── vscode/                 # VSCode 扩展
│       ├── package.json
│       └── src/
│           ├── workerMonitor.ts
│           ├── stateApi.ts
│           └── commands.ts
│
└── dashboard/
    └── index.html              # Web 看板（由 hydra-daemon 托管）
```

### 依赖图

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

**规则**：hydra-core 不依赖任何上层 crate。所有其他 crate 依赖 hydra-core。  
hydra-workspace 只依赖 hydra-core 类型 + ISO-Framework 库。

---

## 9. 接口契约详解

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
  AgentEvent::Subscribe（Agent 请求事件流）
```

### 9.2 ExecutionAgent ↔ ToolRegistry

```
ExecutionAgent → ToolRegistry:
  registry.get("edit_file") → &dyn Tool
  registry.get("bash") → &dyn Tool
  registry.get("read_file") → &dyn Tool（已做范围限制）

ToolRegistry → ExecutionAgent:
  ToolResult { call_id, output, success, duration }
  （从 tool.execute() 返回，添加到 conversation）
```

### 9.3 OrchestratorAgent ↔ ResourceManager

```
OrchestratorAgent → ResourceManager:
  resources.spawn(AgentKind::Execution, spec) → AgentId
  resources.send_command(child_id, AgentCommand::Kill { ... })
  resources.snapshot(child_id) → Option<AgentSnapshot>
  resources.snapshots() → Vec<AgentSnapshot>

ResourceManager → OrchestratorAgent:
  AgentEvent（所有 Agent 的所有事件，通过 event_rx）
```

### 9.4 StateServer ↔ 客户端

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
  Server → Client: AgentEvent（实时）
  Client → Server: { action: "kill", agent_id: N }
                  { action: "open_vscode", agent_id: N }
                  { action: "subscribe" }
```

### 9.5 HydraWorkspace ↔ ISO-Framework

```
Hydra 类型:
  AgentId, AgentKind, AgentState
  AgentSpec（branch, worktree, base_branch）

ISO-Framework 类型（封装，不泄露到 core）:
  iso_code::Manager
  iso_code::Handle
  iso_code::CreateOptions
  iso_code::DeleteOptions
  iso_code::GcOptions

适配层（hydra-workspace/src/adapter.rs）:
  AgentId → 分支名:  format!("agent/{}", id.0)
  AgentSpec → CreateOptions
  AgentOutcome → 决定是否删除或保留 worktree
```

---

## 10. 失败模式与保证

| 失败场景 | 保证 | 机制 |
|---------|------|------|
| Agent panic | 发出事件，worktree 保留供检查 | tokio::spawn + JoinHandle::await |
| Agent 无进展挂起 | 超过可配置超时后被 kill | Orchestrator 的空闲检测（规则下限） |
| Worktree 损坏 | main 分支无数据丢失 | ISO-Framework：绝不删除未合并的分支 |
| 孤立 worktree | 清理时 GC | ISO-Framework::gc() |
| 网络失败（LLM） | 带退避重试，然后 kill | TurnRunner 重试逻辑（来自 hydra） |
| Git merge conflict | Agent 被 kill，冲突保留 | ISO-Framework 冲突检测 |
| ResourceManager panic | Agent 继续运行（它们持有自己的 event_rx） | Agent 在 spawn 后不依赖 ResourceManager |

---

## 11. 扩展点

| 扩展 | 方式 |
|------|------|
| 新 AgentKind | 实现 `Agent` trait，在 `ResourceManager::spawn()` 中注册 |
| 新 Provider | 实现 `LlmProvider`，注册到 `ProviderRegistry` |
| 新 Tool | 实现 `Tool`，注册到 `ToolRegistry` |
| 新事件订阅者 | 调用 `resource_manager.subscribe()`，处理 `AgentEvent` 流 |
| 新前端 | 连接 StateServer 的 REST + WS —— 无需修改 core |
| 新质量策略 | 实现 `Policy` trait，通过 `AgentCommand::SetPolicy` 传入 |

---

## 12. 实施顺序

```
Phase 0（1 周）：基础
  - Fork ISO-Framework → hydra-workspace
  - 定义 Agent trait + AgentId + AgentState + AgentEvent
  - 定义 ResourceManager（最小可用：spawn + send_command + subscribe）
  - 定义 AgentCommand + AgentOutcome

Phase 1（1 周）：ExecutionAgent
  - 从零实现 ExecutionAgent（基于 §3.1 Agent trait）
  - 工具范围限制（ScopedReadFile 模式）
  - ToolContext 使用 worktree 作为 cwd
  - TurnEvent → AgentEvent 映射

Phase 2（1 周）：OrchestratorAgent
  - 带元工具的 Orchestrator（inspect/spawn/kill/promote）
  - System prompt + 工具定义
  - 基础策略（规则版质量门作为 fallback）
  - Subtask 分解（SubtaskDriver 集成）

Phase 3（1 周）：可视化
  - StateServer（REST + WS）
  - hydra-cli：serve、run、workers、open
  - AgentSnapshot 序列化
  - VSCode 扩展：树形视图 + 打开 worktree

Phase 4（1 周）：打磨
  - QualityGate trait（用 LLM 版替换规则版）
  - 多模型 fallback
  - 错误处理 + 遥测
  - 文档
```

---

## 13. 风险与开放问题

| # | 风险 | 缓解 | 状态 |
|---|------|------|------|
| R1 | ISO-Framework API 在我们 fork 前发生变化 | 立即 fork 到稳定 commit | 待处理 |
| R2 | Orchestrator LLM 做出错误的 kill 决策 | 规则安全下限（min_turns_before_kill） | 设计阶段 |
| R3 | Orchestrator 上下文 token 成本增长 | 上下文预算 + 事件摘要 | 设计阶段 |
| R4 | Windows git worktree 支持 | ISO-Framework 声称跨平台；在 WSL 上验证 | 待处理 |
| R5 | 并发 git 操作（两个 Agent 同时 commit） | 通过 ResourceManager 锁顺序执行 | 设计阶段 |
