# 伙伴分工方案

## 当前已交付的稳定基线

| 层 | 产物 | 验证状态 |
|---|---|---|
| Agent 后端 API | `crates/hydra-daemon/src/api_agent.rs` | 10/10 测试通过 |
| Agent TUI 接入 | `crates/hydra-tuix/src/event_loop/commands.rs` `/agents` | 编译通过 |
| 连通性契约测试 | `crates/hydra-core/tests/contract_connectivity.rs` | 5/5 通过 |
| 设计稿 | `docs/superpowers/specs/2026-05-30-agent-control-plane-design.md` | 已确认 |
| 差异清单 | `docs/architecture/implementation-gap-checklist.zh.md` | 已产出 |
| 模块契约卡 | `docs/architecture/module-contract-cards.zh.md` | 已产出 |

## 推荐分工（3 人并行）

### 同学 A：Agent Runtime 真实化

**职责**
- 把 `AgentRuntimeBridge` 从 mock 推进升级为真实执行
- 绑定 `hydra-core` 的 `TurnRunner` / session / tool
- `start` 后真正创建后台 tokio task 执行 turn loop
- 把 turn event 转换为 agent event 写入 `AgentEventStore`

**依赖**
- 当前 `api_agent.rs` 中的 `AgentRegistry` 接口不变
- 只改 `mock_start_progression` / `mock_append_input_progression` 内部实现

**验收标准**
- `POST /api/v1/agents/{id}/commands` 发 `start` 后，agent 真正执行 LLM turn
- 事件流中能看到真实 `agent_message` 事件
- 现有 10 条测试继续通过

**参考入口**
- `crates/hydra-daemon/src/api_agent.rs`（mock_start_progression）
- `crates/hydra-core/src/turn/runner.rs`

---

### 同学 B：Worktree / Branch API 补全

**职责**
- 新增 `crates/hydra-daemon/src/api_worktree.rs`
- 实现：
  - `GET /api/v1/worktrees`
  - `POST /api/v1/worktrees`
  - `DELETE /api/v1/worktrees/{id}`
- 新增 `crates/hydra-daemon/src/api_branch.rs`
- 实现：
  - `GET /api/v1/branches`
  - `POST /api/v1/branches`
  - `DELETE /api/v1/branches/{name}`
- 复用 `hydra-core::git::worktree::WorktreeManager`

**依赖**
- 不依赖 Agent 层
- 只依赖 `hydra-core` 现有 worktree 能力

**验收标准**
- 能通过 API 创建/列出/删除 worktree
- 能通过 API 列出/创建/删除 branch
- 补契约测试
- 不破坏现有测试

**参考入口**
- `crates/hydra-core/src/git/worktree.rs`
- `crates/hydra-daemon/src/api_agent.rs`（参考模块组织方式）

---

### 同学 C：TUI Agent 面板 + 事件流展示

**职责**
- 把当前 `/agents` 命令升级为更丰富的 TUI 交互
- 增加：
  - 定时轮询 agent 状态变化
  - 详情页展示最近事件流
  - `waiting_input` 时自动提示用户输入
  - 状态变化时在 scrollback 中显示通知
- 可选：做成 modal（类似 `/resume` 的 session picker）

**依赖**
- 只依赖 daemon 的 `/api/v1/agents` HTTP 接口
- 不直接依赖 `hydra-core`

**验收标准**
- TUI 能实时看到 agent 状态变化
- `waiting_input` 时能交互式补输入
- 不破坏现有 TUI 功能

**参考入口**
- `crates/hydra-tuix/src/event_loop/commands.rs`（handle_agents）
- `crates/hydra-tuix/src/modals/session_picker.rs`（modal 模式参考）

---

## 分工依赖关系

```
同学 A（Agent Runtime 真实化）
    ↓ 不阻塞 B/C
同学 B（Worktree/Branch API）
    ↓ 完全独立
同学 C（TUI Agent 面板）
    ↓ 只依赖 HTTP 接口（已稳定）
```

三人可以完全并行，互不阻塞。

## 验证命令

每位同学完成后都应跑：
```bash
cargo test -p hydra-daemon
cargo test -p hydra-core --test contract_connectivity
cargo check -p hydra-tuix
```

## 后续汇合点

三人完成后，下一轮迭代目标：
- 把 Agent 和 Worktree/Branch 关联起来（`worktree_id` / `branch_name` 字段生效）
- 把 TUI 面板和真实 runtime 联调
- 补 SSE 事件流（替代轮询）

## 相关文档索引

- 设计稿：`docs/superpowers/specs/2026-05-30-agent-control-plane-design.md`
- 差异清单：`docs/architecture/implementation-gap-checklist.zh.md`
- 模块契约卡：`docs/architecture/module-contract-cards.zh.md`
- 本文档：`docs/architecture/team-assignment.zh.md`
