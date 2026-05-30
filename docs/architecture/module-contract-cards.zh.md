# 模块分工契约卡

## 使用方式
每张卡包含：
- 模块职责
- 允许依赖
- 对外契约
- 当前已有测试验收点
- 推荐负责人关注范围

适合直接拿来拆分给不同伙伴并行开发。

---

## 卡 1：Turn / Tool Orchestration（`hydra-core`）

### 职责
- 驱动单次 turn 的 LLM 输出解析
- 执行 tool calls 并回灌结果
- 产出 turn 级事件与结果

### 允许依赖
- `provider::*`
- `tool::*`
- `turn::permission::*`
- `conversation::*`
- `hook::*`

### 对外契约
- 输入：conversation、system prompt、tool registry、permission decider
- 输出：`TurnResult` + `TurnEvent`
- 不变量：
  - 审批拒绝时不得继续执行危险工具
  - 审批允许时必须继续执行并产出 tool result
  - 会话授权后相同工具不再重复弹审批

### 已有验收
- `crates/hydra-core/tests/contract_connectivity.rs`
  - `cli_interactive_approval_emits_prompt_and_denies_tool_execution`
  - `cli_interactive_approval_allows_tool_execution_after_confirmation`
  - `cli_session_grant_skips_second_prompt_and_executes_directly`

### 适合负责人
- 负责 agent loop、tool execution、permission flow 的同学

---

## 卡 2：Worktree Isolation（`hydra-core/git/worktree`）

### 职责
- 创建隔离 worktree
- 为执行过程提供独立文件系统工作区
- 失败后清理 worktree，不污染主仓库

### 允许依赖
- git 命令调用
- 临时目录与路径工具

### 对外契约
- 输入：branch name、base ref
- 输出：worktree path、list/remove 能力
- 不变量：
  - worktree 删除后路径不存在
  - `git worktree list` 中不再出现该 worktree
  - 主仓库基线文件内容不变

### 已有验收
- `crates/hydra-core/tests/contract_connectivity.rs`
  - `worktree_rollback_cleanup_discards_isolated_changes_without_touching_main_repo`

### 适合负责人
- 负责隔离执行、回滚与清理语义的同学

---

## 卡 3：Session Persistence（`hydra-core/session`）

### 职责
- 管理会话文件的持久化与回读
- 维护 project hash → session bucket 的映射

### 允许依赖
- `serde_json`
- 文件系统
- 时间戳/会话命名工具

### 对外契约
- 输入：working directory、session
- 输出：session json 文件、session meta 列表、session detail
- 不变量：
  - 会话必须落在 `$HYDRA_HOME/sessions/<project_hash>/<session_id>.json`
  - `save/load/list` 三者口径一致
  - project hash 算法必须在所有调用方保持一致

### 已有验收
- `crates/hydra-core/tests/contract_connectivity.rs`
  - `session_manager_persists_session_under_hydra_home_contract`
- `crates/hydra-daemon/src/main.rs` 测试模块
  - `sessions_endpoint_creates_and_persists_session_under_hydra_home`

### 适合负责人
- 负责会话持久化、恢复、项目历史浏览的同学

---

## 卡 4：Daemon API / Session Facade（`hydra-daemon`）

### 职责
- 对外暴露 REST/SSE 接口
- 桥接 session、provider、chat、project state
- 提供 API 入口层的工具门禁

### 允许依赖
- `hydra-core::session`
- `hydra-core::provider`
- `hydra-core::turn`
- `axum`

### 对外契约
- 输入：HTTP 请求
- 输出：JSON / SSE 响应
- 不变量：
  - `POST /sessions` 创建的会话必须立即落盘
  - 返回的 `project_hash` 必须能用于后续读取
  - 危险工具默认关闭，显式开启后才注册
  - `HYDRA_DISABLE_TOOLS` 优先级高于危险工具总开关

### 已有验收
- `crates/hydra-daemon/src/main.rs` 测试模块
  - `dangerous_tools_require_opt_in_even_when_not_disabled`
  - `dangerous_tools_can_be_enabled_but_disable_list_still_wins`
  - `sessions_endpoint_creates_and_persists_session_under_hydra_home`

### 适合负责人
- 负责 daemon API、服务端状态管理、权限门禁的同学

---

## 卡 5：架构对齐 / 文档维护

### 职责
- 保证文档中的模块关系、接口面、权限语义与实现一致
- 标注“当前实现态”与“目标规划态”

### 允许依赖
- `docs/architecture/*`
- 上述各模块的契约测试结果

### 对外契约
- 输入：代码真实实现、测试结果
- 输出：清晰的架构说明与差异清单
- 不变量：
  - 文档不得把未实现接口写成已实现事实
  - 文档中的契约描述应能映射到至少一条测试

### 已有文档产物
- `docs/architecture/implementation-gap-checklist.zh.md`
- `docs/superpowers/specs/2026-05-30-contract-connectivity-tests-design.md`

### 适合负责人
- 负责整体架构收敛、跨模块对齐与任务拆分的同学
