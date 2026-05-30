# Hydra Architecture Documentation

This directory contains the complete architecture design for Hydra — a Rust-based multi-agent code generation system with LLM-driven orchestration and git worktree isolation.

## Documents

| File | Purpose |
|------|---------|
| [overview.md](./overview.md) | Complete architecture design: principles, core abstractions, event flow, crate structure, interface contracts, failure modes |
| [fork-analysis.md](./fork-analysis.md) | Analysis of GitHub projects (ISO-Framework, forge, AutoAgents) and fork integration plan |
| [diagrams.md](./diagrams.md) | Mermaid diagrams: system topology, agent lifecycle, event flow, deployment, dependencies, workspace layout |
| [README.md](./README.md) | This file — entry point to architecture docs |

## Quick Navigation

### For architects
1. Start with [overview.md § Design Principles](./overview.md#1-design-principles)
2. Read [overview.md § Core Abstractions](./overview.md#3-core-abstractions) for the Agent trait and type system
3. Study [diagrams.md § System Topology](./diagrams.md#1-system-topology-mermaid) for the big picture

### For implementers
1. Read [fork-analysis.md § Recommendation Summary](./fork-analysis.md#2-recommendation-summary) — fork ISO-Framework
2. Follow [overview.md § Implementation Sequence](./overview.md#12-implementation-sequence) for phase-by-phase guidance
3. Reference [overview.md § Interface Contracts](./overview.md#9-interface-contracts-detailed) when connecting modules

### For integrators (VSCode, Dashboard, CLI)
1. Read [overview.md § Interface Contracts § StateServer ↔ Clients](./overview.md#94-stateserver--clients) for the REST/WS API
2. Read [diagrams.md § Layered Communication Model](./diagrams.md#8-layered-communication-model) for the 5-layer architecture

## Key Decisions

| Decision | Rationale | Document |
|----------|-----------|----------|
| All agents share the same `Agent` trait | Eliminates "orchestrator is special" anti-pattern; enables recursive multi-level orchestration | [overview.md § 3.1](./overview.md#31-agent-trait-universal-interface) |
| Fork ISO-Framework for worktree management | Battle-tested safety guarantees (5-step unmerged check, GC, crash-safe state); solves the hardest problem | [fork-analysis.md § 1.1](./fork-analysis.md#11-iso-framework-snehith01001110iso-framework-13) |
| Single-direction event flow (broadcast only) | No agent holds another agent's receiver; ResourceManager is the sole router | [overview.md § 6](./overview.md#6-event-flow) |
| Orchestrator uses LLM for decisions (with rule floor) | Adaptive strategy vs hard-coded rules; rules only as safety minimums | [overview.md § 5.2](./overview.md#52-orchestratoragent-the-scheduler) |
| Workspace isolation by default | Each ExecutionAgent gets its own worktree; zero shared mutable state between agents | [overview.md § 7](./overview.md#7-git-worktree-isolation) |

## Crate Structure

```
hydra/
├── crates/
│   ├── hydra-core/          # Agent trait, ResourceManager, all agent types
│   ├── hydra-daemon/        # REST + WebSocket server
│   ├── hydra-cli/           # CLI entry point
│   ├── hydra-telemetry/     # Logging, tracing, datalog
│   └── hydra-workspace/     # ISO-Framework fork (worktree safety)
├── extensions/vscode/       # VSCode extension
└── dashboard/               # Web dashboard
```

See [overview.md § 8](./overview.md#8-crate-structure) for full crate layout and dependency graph.

## Implementation Phases

| Phase | Duration | Deliverable |
|-------|----------|-------------|
| Phase 0 | 1 week | Fork ISO-Framework + Agent trait + ResourceManager skeleton |
| Phase 1 | 1 week | ExecutionAgent (wraps atomcode's SubAgentTask) |
| Phase 2 | 1 week | OrchestratorAgent (LLM-driven scheduler) |
| Phase 3 | 1 week | Visibility: StateServer + CLI + VSCode extension |
| Phase 4 | 1 week | Polish: QualityGate trait, multi-model fallback, telemetry |

## External References

| Project | URL | Relevance |
|---------|-----|-----------|
| ISO-Framework | https://github.com/snehith01001110/ISO-Framework | **Fork target** — worktree safety |
| atomcode | /mnt/c/Users/15853/Workspace/Hydra/atomcode | **Upstream dependency** — Provider, Tool, TurnRunner |
| AutoAgents | https://github.com/liquidos-ai/AutoAgents | Reference — typed pub/sub, agent trait |
| forge | https://github.com/automagik-dev/forge | Reference — UX patterns (not forked) |
