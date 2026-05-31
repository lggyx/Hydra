# 当前实现契约

**日期**: 2026-05-31
**状态**: 现行 — 与代码实际一致

本文档描述 Hydra 当前**真实已实现**的架构，与 `overview.md` 中的目标架构互补。

---

## 1. 核心架构：AgentLoop 通道驱动

当前 agent 系统的核心是 `hydra-core` 中的 **AgentLoop**，而非 `overview.md` 描述的 Agent trait / ResourceManager 模型。

```
TUI (hydra-cli)                     Daemon (hydra-daemon)
     │                                     │
     │  /agents create (HTTP)              │
     ├────────────────────────────────────►│ AgentRegistry.create()
     │                                     │    → AgentSnapshot { status: Created }
     │                                     │
     │  /agents <id> start (HTTP)          │
     ├────────────────────────────────────►│ spawn_agent_execution()
     │                                     │    → run_real_execution()
     │                                     │      ├─ Config + Provider
     │                                     │      ├─ ToolRegistry (gated + MCP + LSP)
     │                                     │      ├─ TurnRunner → LLM turn loop
     │                                     │      └─ map_turn_event → AgentEvent
     │                                     │
     │  SSE /events/stream                 │
     │◄════════════════════════════════════│ broadcast::channel per agent
     │                                     │
     ▼                                     ▼
  AgentPollEvent                      AgentEvent → AgentEventStore
  → scrollback 渲染                   → broadcast → SSE subscribers
```

### 关键差异

| 概念 | 目标架构 (overview.md) | 当前实现 |
|------|------------------------|----------|
| Agent 接口 | `Agent` trait (run / on_command) | `AgentSnapshot` + `AgentRegistry` (状态机驱动)，Agent trait 已实现 |
| Agent 调度 | `ResourceManager` (spawn / send_command / subscribe) | `spawn_agent_execution` → `run_real_execution` / `run_orchestrator_execution` |
| Agent 类型 | ExecutionAgent / OrchestratorAgent / ReviewerAgent | ExecutionAgent + OrchestratorAgent 已实现，ReviewerAgent 规划中（接入 cannbot-skills） |
| 事件分发 | `ResourceManager` 单点 fan-out | `AgentEventStore` + `broadcast::Sender` per agent |
| Worktree 隔离 | 每个 Agent 运行在独立 git worktree | `AgentSnapshot.worktree_id` 可选关联，执行时解析路径 |

---

## 2. 已实现的模块

### hydra-core

| 模块 | 路径 | 说明 |
|------|------|------|
| Turn Runner | `turn/runner.rs` | LLM stream → tool call → tool result 主循环 |
| Permission | `turn/permission.rs` | CLI 交互审批 / API BypassAll |
| Session | `session/mod.rs` | `SessionManager` 持久化到 `$HYDRA_HOME/sessions/` |
| Worktree | `git/worktree.rs` | `WorktreeManager` 创建/列出/删除 worktree |
| Agent Loop | `agent/mod.rs` | `AgentLoop` + `AgentCommand`/`AgentEvent` 通道，TUI 侧入口 |
| Sub Agent | `agent/sub_agent.rs` | 并行编辑 / 后台任务子 agent |
| Tools | `tool/*.rs` | 完整工具集：文件、bash、搜索、web 等 |
| MCP | `mcp/mod.rs` | MCP 工具注册和缓存 |
| LSP | `lsp/manager.rs` | 诊断和代码分析 |

### hydra-daemon

| 模块 | 路径 | 端点 |
|------|------|------|
| Agent API | `api_agent.rs` | `GET/POST /api/v1/agents`, `GET /:id`, `POST /:id/commands`, `GET /:id/events`, `GET /:id/events/stream` |
| Worktree API | `api_worktree.rs` | `GET/POST /api/v1/worktrees`, `DELETE /:id` |
| Branch API | `api_branch.rs` | `GET/POST /api/v1/branches`, `DELETE /:name` |
| Sessions | `main.rs` | `POST /sessions`, `GET /projects/:hash/sessions/:id` |
| Chat (SSE) | `main.rs` | `POST /chat` — SSE 流式响应 |
| Auth | `api_auth.rs` | OAuth 登录流程 |
| Config | `api_config.rs` | Provider 配置管理 |
| CodingPlan | `api_codingplan.rs` | CodingPlan setup |

### hydra-tuix (TUI 库)

| 模块 | 路径 | 说明 |
|------|------|------|
| Agent 命令 | `event_loop/commands.rs` | `/agents` slash command，create/start/cancel/input/events |
| Agent 轮询 | `event_loop/mod.rs` | `AgentPollEvent` 枚举，`handle_agent_poll_event` 渲染 |
| SSE 客户端 | `event_loop/commands.rs` | `spawn_agent_sse` 阻塞线程读 SSE 流 |
| Modals | `modals/` | Session picker、provider wizard 等 |

---

## 3. 已验证的契约

### 自动化测试覆盖

| 层级 | 测试数 | 说明 |
|------|--------|------|
| daemon 单元测试 | 19 | Agent/worktree/branch API + Router smoke |
| contract_connectivity | 5 | Session 持久化、worktree 回滚、CLI 交互审批 |

### 端到端验证链路

```
TUI /agents create → POST /api/v1/agents → AgentRegistry.create()
TUI /agents start  → POST /:id/commands   → spawn_agent_execution()
                   → run_real_execution()  → TurnRunner + LLM
                   → map_turn_event()      → AgentEventStore + broadcast
TUI SSE client     → GET /:id/events/stream → scrollback 实时渲染
```

---

## 4. 与目标架构的关系

`overview.md` 描述的目标架构（Agent trait、ResourceManager、多 Agent 类型）是长期演进方向。当前 AgentLoop 架构可视为 Phase 1 的最小可行实现：

- `AgentRegistry` 承担了 `ResourceManager` 的 agent 管理职责
- `broadcast::channel` 承担了 fan-out 事件分发
- `AgentPollEvent` 承担了 subscriber 侧的渲染逻辑

未来按 Agent trait 架构重构时（方案 B），当前实现的 turn runner、tool registry、session、worktree 等模块可复用。
