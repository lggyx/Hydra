# Hydra Development Workflow

> **Purpose**: Defines Git branching rules, merge policies, module development division, and the current plan for communication verification between layers using mock data.  
> **Status**: Draft  
> **Date**: 2026-05-30

---

## Table of Contents

1. [Git Branch Creation Rules](#1-git-branch-creation-rules)
2. [Repository Merge Rules](#2-repository-merge-rules)
3. [Current Work: Communication Verification Plan](#3-current-work-communication-verification-plan)
4. [Module Development Division of Labor](#4-module-development-division-of-labor)
5. [Mock Strategy for Layer Communication](#5-mock-strategy-for-layer-communication)
6. [Verification Scope & Exit Criteria](#6-verification-scope--exit-criteria)

---

## 1. Git Branch Creation Rules

### 1.1 Branch Naming Convention

All feature branches must follow the pattern:

```
<type>/<module>-<short-description>
```

| Type | Purpose | Example |
|------|---------|---------|
| `feat` | New feature or module | `feat/resource-manager-skeleton` |
| `fix` | Bug fix | `fix/agent-event-serialization` |
| `refactor` | Code restructuring without behavior change | `refactor/event-bus-api` |
| `test` | Test-only changes | `test/resource-manager-unit-tests` |
| `docs` | Documentation only | `docs/architecture-overview` |
| `chore` | Tooling, CI, build scripts | `chore/update-gitignore` |

### 1.2 Module Prefix Reference

| Module | Prefix |
|--------|--------|
| `hydra-core` (Agent trait, ResourceManager, agent types) | `core` |
| `hydra-daemon` (REST/WS server) | `daemon` |
| `hydra-cli` (CLI entry) | `cli` |
| `hydra-telemetry` (logging, tracing, datalog) | `telemetry` |
| `hydra-workspace` (ISO-Framework fork, worktree safety) | `workspace` |
| `extensions/vscode` | `vscode` |
| `dashboard` | `dashboard` |
| Cross-cutting / architecture | `arch` |

### 1.3 Branch Lifecycle

```
main (protected)
  ├── feat/core-resource-manager
  │     └── PR → merge → delete branch
  ├── feat/daemon-state-server
  │     └── PR → merge → delete branch
  └── feat/workspace-iso-framework-fork
        └── PR → merge → delete branch
```

**Rules**:
1. `main` is the single integration branch. Direct pushes are prohibited; all changes must go through PRs.
2. Branch off `main` for every new piece of work.
3. Keep branches short-lived: target ≤ 1 week per branch.
4. Delete the branch after merge.
5. If a branch is abandoned, convert it to a draft PR for reference, then delete.

### 1.4 Branch Permissions

| Branch | Who can push | Who can merge |
|--------|-------------|---------------|
| `main` | No one (protected) | Core maintainers only |
| `feat/*`, `fix/*`, `refactor/*`, `test/*`, `docs/*`, `chore/*` | Author + reviewers | Author + reviewers |

---

## 2. Repository Merge Rules

### 2.1 Pull Request Requirements

Before a PR can be merged, ALL of the following must be satisfied:

| Requirement | Enforcement | Notes |
|-------------|-------------|-------|
| **CI passes** | Automated | `cargo check`, `cargo test`, `cargo clippy` |
| **At least 1 approval** | Human review | From a team member with write access |
| **No conflicts** | Automated + Human | Must rebase onto latest `main` |
| **Draft PR → Ready** | Human | Draft PRs must be explicitly marked ready |

### 2.2 Merge Strategy

We use **Squash and Merge** for all feature branches.

**Rationale**:  
Hydra is a multi-crate Rust workspace. Squashing keeps `main` history linear and makes reverts atomic (one commit per feature). Detailed commit history is preserved in the feature branch for pre-merge review.

```
feat/core-resource-manager (3 commits)
  commit A: "feat(core): add ResourceManager spawn logic"
  commit B: "fix(core): handle spawn race condition"
  commit C: "test(core): add ResourceManager integration test"
        │
        │  [Squash + Merge]
        ▼
main
  commit D: "feat(core): ResourceManager spawn logic + fix + tests"
```

### 2.3 Merge Order Dependencies

Some modules depend on others. The merge order must respect the dependency graph:

```
Phase 0 (no internal deps, can merge in parallel):
  feat/workspace-iso-framework-fork
  feat/core-agent-trait

Phase 1 (depends on Phase 0):
  feat/core-resource-manager     ← depends on agent-trait
  feat/workspace-worktree-manager ← depends on iso-framework-fork

Phase 2 (depends on Phase 1):
  feat/core-execution-agent      ← depends on RM + agent-trait
  feat/core-orchestrator-agent   ← depends on RM + agent-trait

Phase 3 (depends on Phase 2):
  feat/daemon-state-server       ← depends on core agents
  feat/cli-interface             ← depends on RM
  feat/vscode-extension          ← depends on StateServer

Phase 4 (polish, depends on all):
  feat/core-quality-gate
  feat/telemetry-datalog
  feat/multi-model-fallback
```

**Rules**:
1. Do not merge Phase N branches until all Phase N-1 branches are merged.
2. Within the same phase, branches can merge in any order (they should be independent).
3. If two branches in the same phase have merge conflicts, coordinate with the authors to resolve.

### 2.4 Revert Policy

- **Hotfix**: Create `fix/<issue>` off `main`, fix, merge immediately. If the fix itself causes issues, revert the merge commit directly on `main`.
- **Feature revert**: If a merged feature must be reverted, use `git revert` on `main` to create a new commit that undoes the squash merge. Do NOT rewrite `main` history.
- **Revert scope**: Reverts should be complete (undo the entire feature). Partial reverts should be handled by the original author in a new `fix/*` branch.

---

## 3. Current Work: Communication Verification Plan

### 3.1 Objective

**Do NOT implement actual business logic.**  
The goal is to verify that every layer in the system can communicate correctly using mock implementations. This is a wiring/integration test at the architectural level.

### 3.2 Sub-Branches to Create

| Branch | Owner | Scope | Merge Target |
|--------|-------|-------|-------------|
| `feat/arch-mock-communication` | — | Top-level branch; aggregates all verification code | `main` |
| `feat/core-mock-agent-impl` | — | Mock Agent implementations for each AgentKind | `feat/arch-mock-communication` |
| `feat/core-mock-resource-manager` | — | Mock ResourceManager with in-memory event bus | `feat/arch-mock-communication` |
| `feat/core-mock-workspace` | — | Mock GitWorktreeManager (no real git operations) | `feat/arch-mock-communication` |
| `feat/core-mock-llm-provider` | — | Mock LlmProvider returning canned responses | `feat/arch-mock-communication` |
| `feat/test-mock-communication` | — | Integration test wiring all mocks together | `feat/arch-mock-communication` |

### 3.3 Branch Creation Commands

```bash
# From main
git checkout main
git pull origin main

# Create all sub-branches
git checkout -b feat/arch-mock-communication
git checkout -b feat/core-mock-agent-impl
git checkout -b feat/core-mock-resource-manager
git checkout -b feat/core-mock-workspace
git checkout -b feat/core-mock-llm-provider
git checkout -b feat/test-mock-communication

# Push to remote
git push origin feat/arch-mock-communication feat/core-mock-agent-impl \
  feat/core-mock-resource-manager feat/core-mock-workspace \
  feat/core-mock-llm-provider feat/test-mock-communication
```

### 3.4 Branch Merge Order

All sub-branches merge into `feat/arch-mock-communication` first, then that branch is merged into `main`:

```
feat/core-mock-agent-impl       ─┐
feat/core-mock-resource-manager ─┤
feat/core-mock-workspace        ─┤
feat/core-mock-llm-provider     ─┤→ feat/arch-mock-communication → main
feat/test-mock-communication    ─┘
```

---

## 4. Module Development Division of Labor

### 4.1 Module Ownership Matrix

| Module | Primary Owner | Reviewers | Dependencies |
|--------|--------------|-----------|--------------|
| `hydra-core` (Agent trait, Agent types, events) | TBD | TBD | — |
| `hydra-core` (ResourceManager) | TBD | TBD | Agent trait |
| `hydra-workspace` (ISO-Framework fork) | TBD | TBD | — |
| `hydra-daemon` (StateServer, REST/WS) | TBD | TBD | hydra-core |
| `hydra-cli` | TBD | TBD | hydra-core |
| `hydra-telemetry` | TBD | TBD | hydra-core |
| `extensions/vscode` | TBD | TBD | hydra-daemon |
| `dashboard` | TBD | TBD | hydra-daemon |

### 4.2 Crate Dependency Map

```
hydra-workspace (standalone, no internal deps)
    │
hydra-core
    ├── depends on: hydra-workspace
    │
    ├── hydra-daemon
    │     └── depends on: hydra-core
    │
    ├── hydra-cli
    │     └── depends on: hydra-core
    │
    ├── hydra-telemetry
    │     └── depends on: hydra-core
    │
    └── (extensions/vscode, dashboard depend on hydra-daemon)
```

### 4.3 Interface Ownership

Each crate owns its public API. Changes to public interfaces require:
1. Discussion in an architecture review issue
2. Approval from at least 2 team members
3. Update to `docs/architecture/overview.md` §9 (Interface Contracts)

| Interface | Owning Crate | Versioning |
|-----------|-------------|------------|
| `Agent` trait | `hydra-core` | Semver major for breaking changes |
| `AgentEvent`, `AgentCommand`, `AgentResponse` | `hydra-core` | Semver minor for additive changes |
| `ResourceManager` (public API) | `hydra-core` | Semver major for breaking changes |
| `LlmProvider` trait | `hydra-core` | Semver minor for additive changes |
| `GitWorktreeManager` trait | `hydra-workspace` | Semver major for breaking changes |
| `StateServer` REST/WS API | `hydra-daemon` | Semver minor for additive, major for breaking |
| CLI argument schema | `hydra-cli` | Semver minor for additive changes |

### 4.4 Development Sequence

| Week | Phase | Deliverable | Branch(es) |
|------|-------|-------------|------------|
| 1 | Phase 0 | Fork ISO-Framework + Agent trait skeleton | `feat/workspace-iso-framework-fork`, `feat/core-agent-trait` |
| 2 | Phase 1 | ResourceManager + ExecutionAgent | `feat/core-resource-manager`, `feat/core-execution-agent` |
| 3 | Phase 2 | OrchestratorAgent | `feat/core-orchestrator-agent` |
| 4 | Phase 3 | StateServer + CLI + VSCode extension | `feat/daemon-state-server`, `feat/cli-interface`, `feat/vscode-extension` |
| 5 | Phase 4 | QualityGate + telemetry + polish | `feat/core-quality-gate`, `feat/telemetry-datalog` |

---

## 5. Mock Strategy for Layer Communication

### 5.1 Why Mock?

During the communication verification phase, we are NOT implementing:
- Real LLM calls
- Real git worktree operations
- Real tool executions
- Real file system operations

We ARE verifying:
- `Agent` trait can be implemented by all agent kinds (Worker, Orchestrator, Reviewer, Custom)
- `ResourceManager` can spawn agents, send commands, and fan out events
- Agents can emit `AgentEvent` and receive it via the event bus
- `ResourceHandle` can cross `tokio::spawn` boundaries
- The full lifecycle: spawn → run → event emission → command → shutdown

### 5.2 Mock Implementations Needed

#### 5.2.1 Mock Agent Implementations

Located in: `hydra-core/src/mock/` (temporary, removed before Phase 1)

```rust
// Pseudo-signatures (actual code to be written in sub-branches)

pub struct MockWorkerAgent { ... }     // Implements Agent, emits work events
pub struct MockOrchestratorAgent { ... } // Implements Agent, emits scheduling events
pub struct MockReviewerAgent { ... }    // Implements Agent, emits review events
pub struct MockCustomAgent { ... }      // Implements Agent, configurable behavior
```

Each mock agent:
- Has a fixed `AgentId` and `AgentKind`
- Holds a mock `ResourceHandle` (in-memory, no real channels to external systems)
- On `run()`, emits a sequence of canned `AgentEvent`s (Progress, Log, etc.)
- On `on_command()`, acknowledges and transitions state
- Uses a mock `AgentState` that transitions deterministically

#### 5.2.2 Mock ResourceManager

Located in: `hydra-core/src/mock/`

```rust
pub struct MockResourceManager {
    agents: HashMap<AgentId, Box<dyn Agent>>,
    event_bus: broadcast::Sender<AgentEvent>,
    // No real GitWorktreeManager
    // No real LlmProvider
}
```

Behavior:
- `spawn_agent()` instantiates a mock agent, stores it, returns a handle
- `send_command()` delivers the command directly to the mock agent's `on_command()`
- `subscribe()` returns a `broadcast::Receiver<AgentEvent>` connected to the in-memory bus
- `cleanup()` removes the agent from the HashMap

#### 5.2.3 Mock GitWorktreeManager

Located in: `hydra-workspace/src/mock/`

```rust
pub struct MockGitWorktreeManager {
    worktrees: HashMap<String, PathBuf>,
}

impl GitWorktreeManager for MockGitWorktreeManager {
    fn create_worktree(&mut self, ...) -> Result<PathBuf> {
        // Insert into HashMap, return a temp dir
    }
    fn remove_worktree(&mut self, ...) -> Result<()> {
        // Remove from HashMap
    }
    // ... all methods return Ok(()) or canned results
}
```

#### 5.2.4 Mock LlmProvider

Located in: `hydra-core/src/mock/`

```rust
pub struct MockLlmProvider {
    responses: Vec<LlmResponse>,
    call_count: AtomicUsize,
}

impl LlmProvider for MockLlmProvider {
    async fn complete(&self, ...) -> Result<LlmResponse> {
        // Return the next canned response in sequence
    }
}
```

#### 5.2.5 Mock Tools

Located in: `hydra-core/src/mock/`

```rust
pub struct MockTool { name: &'static str }
impl Tool for MockTool {
    fn execute(&self, ...) -> Result<ToolOutput> {
        Ok(ToolOutput::success("mocked"))
    }
}
```

### 5.3 Verification Test Scenarios

Located in: `hydra-core/tests/mock_communication.rs`

| Test | What it verifies |
|------|-----------------|
| `test_spawn_and_run_worker` | MockWorkerAgent can be spawned, run, and emits events |
| `test_spawn_and_run_orchestrator` | MockOrchestratorAgent can be spawned, run, and emits events |
| `test_spawn_and_run_reviewer` | MockReviewerAgent can be spawned, run, and emits events |
| `test_spawn_and_run_custom` | MockCustomAgent can be spawned with configurable behavior |
| `test_send_command_kill` | ResourceManager can send Kill command to a running agent |
| `test_event_bus_fan_out` | Multiple subscribers receive the same events |
| `test_resource_handle_clone` | ResourceHandle can be cloned and passed into tokio::spawn |
| `test_full_lifecycle` | Spawn → run → command → shutdown completes without panic |
| `test_multiple_agents_concurrent` | 5+ mock agents run concurrently, each emits distinct events |
| `test_agent_state_transitions` | AgentState transitions: Idle → Running → Completed |

---

## 6. Verification Scope & Exit Criteria

### 6.1 In Scope

- All `AgentKind` variants (Worker, Orchestrator, Reviewer, Custom) implement `Agent` trait
- `ResourceManager` can spawn any agent kind
- Event flow is unidirectional: Agent → event_tx → EventBus → Subscribers
- `ResourceHandle` crosses `tokio::spawn` boundaries via `Clone`
- `AgentState` transitions work correctly under mock conditions
- No panics in any mock scenario (use `#[should_panic]` to document expected panics)
- Compile with `--all-features` and `--no-default-features` both succeed

### 6.2 Out of Scope (for this phase)

- Real LLM API calls
- Real git operations (no actual worktrees created)
- Real tool execution (no shell commands run)
- Performance benchmarks
- Network communication between processes
- Persistence (all state is in-memory)
- Error recovery from real failures (only mock error paths tested)

### 6.3 Exit Criteria

This phase is complete when:

1. **All tests pass**: `cargo test --workspace` passes with 0 failures
2. **No code changes to production paths**: All mock code is in `src/mock/` and test modules; `src/lib.rs` production code is unchanged
3. **Review approved**: At least 1 team member reviews and approves the mock communication tests
4. **Documentation updated**: `docs/architecture/overview.md` §12 (Implementation Sequence) is updated to reflect actual progress
5. **Branch merged**: `feat/arch-mock-communication` is merged into `main`

### 6.4 Rollout After Exit

Once mock communication is verified:

| Next Step | Branch | Description |
|-----------|--------|-------------|
| Replace mocks one-by-one | `feat/workspace-real-git` | Replace MockGitWorktreeManager with real ISO-Framework implementation |
| Replace mocks one-by-one | `feat/core-real-llm` | Replace MockLlmProvider with real provider |
| Replace mocks one-by-one | `feat/core-real-tools` | Replace MockTool with real tool implementations |
| Each replacement | Same pattern | Write tests alongside implementation, verify communication still works |

---

## Appendix A: Quick Reference Commands

```bash
# Create all sub-branches
git checkout main && git pull
git checkout -b feat/arch-mock-communication
git checkout -b feat/core-mock-agent-impl
git checkout -b feat/core-mock-resource-manager
git checkout -b feat/core-mock-workspace
git checkout -b feat/core-mock-llm-provider
git checkout -b feat/test-mock-communication
git push origin --all

# Merge sub-branches into arch-mock-communication
git checkout feat/arch-mock-communication
git merge feat/core-mock-agent-impl --squash
git commit -m "feat(arch): add mock agent implementations"
git merge feat/core-mock-resource-manager --squash
git commit -m "feat(arch): add mock ResourceManager"
git merge feat/core-mock-workspace --squash
git commit -m "feat(arch): add mock GitWorktreeManager"
git merge feat/core-mock-llm-provider --squash
git commit -m "feat(arch): add mock LlmProvider"
git merge feat/test-mock-communication --squash
git commit -m "feat(arch): add mock communication integration tests"
git push origin feat/arch-mock-communication

# Open PR from feat/arch-mock-communication → main
# After approval:
git checkout main && git merge feat/arch-mock-communication --squash
git push origin main
```

## Appendix B: Mock Code Cleanup Promise

Before Phase 1 implementation begins, all mock code in `src/mock/` and test helpers will be removed. No mock scaffolding remains in production code paths. This is enforced by:

1. `cargo deny` or a custom lint that forbids `mock` module in `--release` builds
2. Code review checklist item: "No mock code in production paths"
