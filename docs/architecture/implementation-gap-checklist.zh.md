# 架构文档与实现差异清单

## 目的

这份清单用于把 `docs/architecture/` 中的目标架构，与当前仓库里的实际实现对齐。
使用方式：
- 把“已确认一致”的部分当作当前分工的稳定边界
- 把“存在差异”的部分当作后续补实现或修文档的 backlog
- 连通性契约测试优先覆盖“当前真实实现的稳定不变量”

## 已确认一致

### 1. `hydra-core` turn / tool / permission 主链路
- `TurnRunner` 负责 LLM stream → tool call → tool result → turn result 的主循环
- CLI 交互审批语义由 `InteractivePermissionDecider` 承担
- 会话级授权由 `PermissionStore` 维护
- 这部分已经由 `crates/hydra-core/tests/contract_connectivity.rs` 覆盖

### 2. worktree 隔离执行
- `WorktreeManager` 负责创建、列出、删除 worktree
- worktree 内变更不会污染主仓库工作区
- 当前测试已覆盖“失败后删除 worktree，主仓库内容保持不变”的不变量

### 3. session 持久化
- `SessionManager` 统一负责 `$HYDRA_HOME/sessions/<project_hash>/<session_id>.json` 的落盘与回读
- `hydra-core` 与 `hydra-daemon` 现在已对齐同一套 `project_hash` 口径

### 4. daemon 最小会话链路
- `POST /sessions` 可创建会话并立即落盘
- handler 级详情读取可以通过 `project_hash + session_id` 成功回读
- 当前测试已覆盖该最小链路

## 已发现差异

### A. ❌ 已解决：`/api/v1/agents|worktrees|branches` 端点已实现
- `crates/hydra-daemon/src/api_agent.rs` — GET/POST `/api/v1/agents`, GET `/:id`, POST `/:id/commands`, GET `/:id/events`, GET `/:id/events/stream`（SSE）
- `crates/hydra-daemon/src/api_worktree.rs` — GET/POST `/api/v1/worktrees`, DELETE `/:id`
- `crates/hydra-daemon/src/api_branch.rs` — GET/POST `/api/v1/branches`, DELETE `/:name`
- 已补 Router 级 smoke 测试（`main.rs` — `agent_smoke_create_list_start_events`）

### B. daemon 与 CLI 的权限语义不同
- CLI：危险操作走交互审批
- daemon：默认无交互审批，采用自动批准模式
- 新增修正：`HYDRA_DAEMON_ENABLE_DANGEROUS_TOOLS=1` 现在真正控制 `bash` / `write_file` / `edit_file` 是否注册
- `HYDRA_DISABLE_TOOLS` 仍然具有更高优先级
- 处理建议：在架构文档中把这两套语义明确区分为“交互入口权限模型”和“API 入口权限模型”

### C. daemon 的会话详情链路曾存在 project hash 口径漂移
- 现象：`create_session` 返回的 `project_hash` 与 `SessionManager` 内部实际使用的目录 hash 算法不一致
- 影响：按返回值访问详情可能 404
- 当前已修复：`hydra-daemon` 统一改为使用与 `hydra-core::SessionManager` 对齐的 hash 逻辑
- 处理建议：后续所有涉及 project hash 的新接口，都应复用同一套 helper/契约测试

### D. ❌ 已解决：路由巡检完成，已补 Router 级 smoke 测试
- 路由参数风格已统一为 axum `:param` 语法
- 已补 `agent_smoke_create_list_start_events` 黑盒测试：完整走 Router 验证 agent 创建→列表→详情→start→事件查询的端到端链路

### E. Agent trait 架构已部分实现（2026-05-31 更新）
- Section 3-5（Agent trait / ResourceManager / ExecutionAgent / OrchestratorAgent）已实现，标记移除
- Section 2、8、12（System Topology / Crate Structure / Implementation Sequence）仍为「规划态」
- 已合入 develop，详见 `crates/hydra-core/src/agent/` 目录下 `traits.rs`、`execution.rs`、`orchestrator.rs`、`resource_manager.rs`

### F. 设计规范文档 `docs/superpowers/specs/` 缺失
- `team-assignment.zh.md` 和 `module-contract-cards.zh.md` 引用的设计规范不存在
- 处理建议：可后续补写，或删除引用

## 当前建议的执行策略

### P0：按当前真实实现分工开发
以已经被契约测试覆盖的真实边界为准：
- `hydra-core`：turn / permission / tool orchestration
- `worktree`：隔离执行与清理
- `session`：持久化与项目 hash 口径
- `hydra-daemon`：会话 API、工具门禁、后续 chat/stream API

### P1：补文档或补实现
对于当前不一致的部分，分两类处理：
- 文档超前：标注为“规划态”
- 实现偏离文档但应保留目标态：开独立任务补实现

## 推荐后续动作
1. 在 `docs/architecture/` 增补一页“当前实现契约”
2. 为 `hydra-daemon` 再补 1 条真正通过 Router 的详情 blackbox smoke
3. 若准备做多 Agent / worktree 编排，再落 `/api/v1/agents|worktrees|branches` 的最小接口骨架
