# Hydra Fork Analysis and Recommendations

**Date**: 2026-05-30  
**Purpose**: Identify GitHub projects to fork, analyze fit, and define integration plan.

---

## 1. Candidate Projects

### 1.1 ISO-Framework (snehith01001110/ISO-Framework) ⭐13

**Repository**: https://github.com/snehith01001110/ISO-Framework  
**Language**: Rust  
**License**: Apache-2.0 / MIT  

**What it does**: Safe git worktree lifecycle management for AI coding agents.

**Why it's relevant**: Solves the exact problem Hydra has at its foundation — managing isolated git worktrees for multiple concurrent AI agents. The README explicitly names Claude Code, Cursor, and OpenCode as tools with documented worktree bugs.

**Key features**:
- `Manager::create(branch, path, options)` — create worktree with safety checks
- `Manager::delete(handle, options)` — 5-step unmerged commit check before deletion
- `Manager::gc(options)` — garbage collect orphaned worktrees
- `Manager::list()` — enumerate active worktrees
- Crash-safe state persistence (atomic write via tmp + fsync + rename)
- MCP server integration (works with Claude Code, Cursor, Copilot, OpenCode)
- Hook integration for Claude Code

**Fit assessment**: 9/10

| Hydra need | ISO-Framework provides | Gap |
|-----------|----------------------|-----|
| Create worktree per agent | ✅ Manager::create | None — direct mapping |
| Delete worktree on agent cleanup | ✅ Manager::delete (with 5-step safety) | None |
| GC orphaned worktrees | ✅ Manager::gc | None |
| List active worktrees | ✅ Manager::list | None |
| Crash-safe state | ✅ atomic write | None |
| Cross-platform (Windows/WSL) | Claimed but unverified | Need to test on WSL |
| Hook into Claude Code | ✅ built-in | Not needed for Hydra core |

**Integration plan**:
```
1. Fork ISO-Framework into hydra/hydra-workspace
2. Thin adapter layer: Hydra types → ISO-Framework types
3. Replace internal WorkspaceManager calls with ISO-Framework API
4. Keep ISO-Framework's safety guarantees intact (no custom worktree logic)
```

**Fork commit**: Pin to a specific SHA once ISO-Framework's API stabilizes.  
**Maintenance**: Track upstream for bug fixes; merge periodically.

---

### 1.2 forge (automagik-dev/forge)

**Repository**: https://github.com/automagik-dev/forge  
**Language**: TypeScript (Tauri) + Python  
**License**: Unknown (check repo)  

**What it does**: Multi-agent kanban platform with MCP integration. Orchestrates AI coding agents across machines via SSH/Tailscale.

**Why it's relevant**: Has the same user-facing workflow (isolated attempts in worktrees, kanban tracking, real-time monitoring). Good reference for UX patterns.

**Fit assessment**: 4/10

| Hydra need | forge provides | Gap |
|-----------|---------------|-----|
| Git worktree isolation | ✅ | Already covered by ISO-Framework |
| Multi-agent parallel execution | ✅ | Different architecture (manual kanban vs LLM orchestration) |
| Real-time monitoring | ✅ Web UI | Hydra has TUI + REST |
| Cross-machine orchestration | ✅ SSH/Tailscale | Not a Hydra priority (local-first) |
| LLM-driven scheduling | ❌ Human-driven kanban | Core Hydra feature |
| Pure Rust backend | ❌ Node.js + Python | No fit |

**Verdict**: Do not fork. Reference only for UX inspiration. forge's kanban model is "human plans, AI executes" — Hydra's model is "AI plans, human approves."

---

### 1.3 AutoAgents (liquidos-ai/AutoAgents) ⭐658

**Repository**: https://github.com/liquidos-ai/AutoAgents  
**Language**: Rust  
**License**: MIT / Apache-2.0  

**What it does**: General-purpose multi-agent framework with typed pub/sub, environment management, and 10+ LLM providers.

**Why it's relevant**: Has a mature Agent trait, typed pub/sub communication, and environment management. Largest Rust multi-agent project by stars.

**Fit assessment**: 5/10

| Hydra need | AutoAgents provides | Gap |
|-----------|--------------------|-----|
| Agent trait | ✅ Agent trait exists | Different design goals — AutoAgents is generic, Hydra is code-specific |
| Typed pub/sub | ✅ | Hydra uses event bus + channels (simpler for single-process) |
| 10+ LLM providers | ✅ OpenAI, Anthropic, DeepSeek, etc. | Hydra needs 3-4 providers initially |
| WASM sandboxed tools | ✅ | Hydra uses native Rust tools (more powerful for code editing) |
| Memory backends | ✅ | Not a Hydra priority (Hydra uses git for memory) |
| Sliding window context | ✅ | Hydra uses atomcode's hot/cold context strategy |

**Verdict**: Do not fork. AutoAgents solves a different problem (generic agent framework). Hydra's value is in the code-generation-specific layers (git worktree isolation + LLM-driven scheduling + tool scoping). AutoAgents' Agent trait could inform Hydra's design, but direct integration would add complexity without proportional value.

---

### 1.4 swarms-rs (The-Swarm-Corporation/swarms-rs) ⭐163

**Repository**: https://github.com/The-Swarm-Corporation/swarms-rs  
**Language**: Rust  

**Assessment**: README is minimal. Cannot evaluate architecture without reading source code. Low priority until more information is available.

---

## 2. Recommendation Summary

| Project | Action | Reason |
|---------|--------|--------|
| **ISO-Framework** | **FORK** | Solves Hydra's hardest unsolved problem (worktree safety) with battle-tested code |
| forge | Reference only | Different architecture; Node.js-based |
| AutoAgents | Reference only | Generic framework; Hydra's agent model is domain-specific |
| swarms-rs | Monitor | Insufficient information to evaluate |

---

## 3. Fork Integration Plan

### 3.1 Phase 0: Fork and Adapter (Week 1)

**Step 1: Fork**

```bash
# Fork snehith01001110/ISO-Framework into hydra-ai/ISO-Framework
# Then add as submodule or direct dependency
cd /path/to/hydra
git submodule add https://github.com/hydra-ai/ISO-Framework.git crates/hydra-workspace/upstream
```

Or use a Cargo patch directive:

```toml
# hydra/Cargo.toml (workspace)
[patch."https://github.com/hydra-ai/ISO-Framework"]
iso-code = { path = "crates/hydra-workspace/upstream" }
```

**Step 2: Create adapter layer**

```
crates/hydra-workspace/
├── Cargo.toml          # depends on iso-code + hydra-core
├── src/
│   ├── lib.rs
│   ├── manager.rs      # HydraWorkspaceManager wraps iso_code::Manager
│   ├── adapter.rs      # Hydra AgentId/branch ↔ ISO-Framework Handle
│   └── safety.rs       # Hydra-specific safety policies
│   └── types.rs        # Hydra worktree types (mirrors ISO-Framework types)
└── tests/
```

**Step 3: Adapter implementation sketch**

```rust
// crates/hydra-workspace/src/manager.rs

use iso_code::{Manager as IsoManager, Config as IsoConfig, CreateOptions, DeleteOptions};
use hydra_core::{AgentId, AgentState, AgentOutcome, AgentMetrics};

pub struct HydraWorkspaceManager {
    inner: IsoManager,
    base_path: PathBuf,
}

impl HydraWorkspaceManager {
    pub fn new(repo_path: PathBuf) -> anyhow::Result<Self> {
        let iso_config = IsoConfig::default();
        let inner = IsoManager::new(&repo_path, iso_config)?;
        Ok(Self { inner, base_path: repo_path })
    }

    /// Create a worktree for an agent. Returns the absolute path.
    pub fn create_for_agent(
        &self,
        agent_id: AgentId,
        base_branch: &str,
    ) -> anyhow::Result<PathBuf> {
        let branch_name = format!("hydra/agent/{}", agent_id.0);
        let wt_name = format!("wt-{}", agent_id.0);
        let wt_path = self.base_path.join(".worktrees").join(&wt_name);

        let (handle, info) = self.inner.create(
            &branch_name,
            &wt_path,
            CreateOptions::default(),
        )?;

        // Store handle mapping for later cleanup
        // info.path == wt_path, info.branch == branch_name

        Ok(wt_path)
    }

    /// Delete worktree after agent completes. Respects ISO-Framework safety.
    pub fn cleanup_agent(&self, agent_id: AgentId, outcome: &AgentOutcome) -> anyhow::Result<()> {
        // If agent succeeded and user wants to keep the branch, skip deletion
        // If agent failed, retain worktree for debugging
        // If user explicitly deletes, run full cleanup

        let wt_path = self.base_path.join(".worktrees").join(format!("wt-{}", agent_id.0));
        let branch_name = format!("hydra/agent/{}", agent_id.0);

        // Check if branch was merged before deleting
        let handle = self.inner.find_handle(&wt_path)?;
        self.inner.delete(&handle, DeleteOptions::default())?;

        Ok(())
    }

    /// GC orphaned worktrees (run periodically or on startup)
    pub fn gc(&self) -> anyhow::Result<iso_code::GcReport> {
        self.inner.gc(Default::default())
    }
}
```

### 3.2 Phase 1: Core Integration (Weeks 2-3)

Replace the placeholder `WorkspaceManager` in ResourceManager with `HydraWorkspaceManager`.

```rust
// crates/hydra-core/src/resource.rs

use hydra_workspace::HydraWorkspaceManager;

pub struct ResourceManager {
    agents:          Arc<RwLock<AgentRegistry>>,
    event_rx:        mpsc::UnboundedReceiver<AgentEvent>,
    event_bus:       mpsc::UnboundedSender<AgentEvent>,   // cloned from event_rx half
    subscribers:     Arc<RwLock<Vec<mpsc::UnboundedSender<AgentEvent>>>>,
    control_senders: Arc<RwLock<HashMap<AgentId, mpsc::UnboundedSender<AgentCommand>>>>,
    next_id:         AtomicU64,
    git:             Arc<dyn GitWorktreeManager>,          // trait object, mockable
    providers:       Arc<RwLock<ProviderRegistry>>,
    tool_registry:   Arc<RwLock<ToolRegistry>>,
}

/// Internal struct assembled by build_handle() — not exposed publicly.
struct AgentBuildResult {
    agent:    Box<dyn Agent>,        // concrete agent, boxed for uniform storage
    state:    Arc<RwLock<AgentState>>,  // agent owns the strong Arc; Registry gets Weak
    resources: ResourceHandle,
}

impl ResourceManager {
    /// Build an agent + its ResourceHandle.  Implementation detail of spawn().
    fn build_handle(&self, id: AgentId, kind: AgentKind, spec: AgentSpec)
        -> Result<AgentBuildResult>
    {
        // ... concrete agent construction based on kind ...
        todo!()
    }

    pub fn spawn(&self, kind: AgentKind, spec: AgentSpec) -> Result<AgentHandle> {
        let id = AgentId(self.next_id.fetch_add(1, Ordering::SeqCst));

        // Build ResourceHandle (agent owns its channels + shared infra)
        let handle = self.build_handle(id, kind, spec)?;

        // Register the agent's control sender so send_command can route to it
        self.control_senders.write().unwrap().insert(id, handle.resources.control_tx.clone());

        // Register the agent's state Arc in the AgentRegistry for observation
        self.agents.write().unwrap().insert(id, handle.state.clone());

        // Spawn agent's run loop
        let event_tx = self.event_bus.clone();
        tokio::spawn(async move {
            let mut agent = handle.agent;
            let outcome = agent.run(handle.resources).await;
            event_tx.send(AgentEvent::Completed { agent_id: id, outcome }).ok();
        });

        Ok(AgentHandle { id })
    }

    pub fn send_command(&self, id: AgentId, cmd: AgentCommand) -> Result<()> {
        let senders = self.control_senders.read().unwrap();
        senders.get(&id)
            .ok_or_anyhow!("agent {} not found or terminated", id)?
            .send(cmd)
            .map_err(|_| anyhow!("agent {} control channel closed", id))
    }
}
```

### 3.3 Phase 2+: Upstream Tracking

```bash
# In crates/hydra-workspace/
git remote add upstream https://github.com/snehith01001110/ISO-Framework.git
git fetch upstream

# Merge upstream fixes monthly
git merge upstream/main
# Resolve any conflicts in the adapter layer
```

---

## 4. ISO-Framework Code Entry Points

Key files to understand before forking:

| File | Purpose | Hydra relevance |
|------|---------|----------------|
| `src/manager.rs` | Core Manager (create/delete/gc/list) | Primary API surface |
| `src/state.rs` | Crash-safe state persistence | Understand atomic write pattern |
| `src/safety.rs` | Safety checks (unmerged, nested, locked) | Trust these guarantees |
| `src/hooks.rs` | Claude Code / Cursor hook integration | Reference for CLI integration |
| `src/mcp.rs` | MCP server implementation | Reference for hydra-daemon |
| `tests/integration_test.rs` | Integration tests | Test cases to preserve |

---

## 5. Risks Specific to Forking ISO-Framework

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| ISO-Framework is abandoned | Medium | Medium | Fork is permanent; we own the code |
| API breaks between Rust versions | Low | Low | Pin Rust version in Cargo.toml |
| ISO-Framework has hidden bugs | Low | High | Run their test suite + Hydra's own tests |
| License incompatibility | Very Low | High | Both Apache-2.0 and MIT — compatible with Hydra |

---

## 6. Maintenance Model for the Fork

Since Hydra forks ISO-Framework directly into `crates/hydra-workspace/`, ongoing maintenance follows this pattern:

| Activity | Cadence | Owner |
|----------|---------|-------|
| Sync upstream bug fixes | Monthly | Core maintainer |
| Review upstream PRs | Weekly | Core maintainer |
| Update adapter layer | As needed (when ISO-Framework API changes) | Core maintainer |
| Hydra-specific safety policies | Ongoing | Hydra team |

```bash
# Monthly sync procedure
cd crates/hydra-workspace/
git fetch upstream
git log upstream/main..HEAD --oneline  # review our local patches
git merge upstream/main                 # merge upstream fixes
cargo test                              # verify nothing broke
```

**Rationale**: ISO-Framework is small (~2000 LOC estimated), purpose-built for AI agent worktree safety, and actively used. Forking gives full control over safety policies while preserving the ability to pull upstream bug fixes. Vendoring would forfeit upstream improvements and add merge friction later.

**License note**: ISO-Framework is MIT/Apache-2.0 dual-licensed — compatible with Hydra's licensing. No legal barrier to forking.
