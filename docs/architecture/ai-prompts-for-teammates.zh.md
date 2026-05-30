# 伙伴 AI 协作提示词

## 使用方式
每位同学开始开发时，把对应的提示词发给 AI（如 Trae/Kiro/Cursor），AI 就能直接上手帮你干活。

前提：先从 `develop` 分支拉出自己的功能分支。

```bash
git checkout develop
git checkout -b feat/agent-runtime       # 同学 A
git checkout -b feat/worktree-branch-api # 同学 B
git checkout -b feat/tui-agent-panel     # 同学 C
```

---

## 同学 A：Agent Runtime 真实化

### 发给 AI 的提示词

```
我在 Hydra 项目中负责把 Agent Runtime 从 mock 升级为真实执行。

当前状态：
- `crates/hydra-daemon/src/api_agent.rs` 中有一个 `AgentRegistry`，它的 `spawn_mock_progression` 方法目前只是 sleep 后自动推进状态（queued → running → completed）
- 我需要把它改成真正调用 `hydra-core` 的 `TurnRunner` 来执行 LLM turn

具体要求：
1. 在 `api_agent.rs` 中，把 `spawn_mock_progression` 改名为 `spawn_agent_execution`
2. start 命令被接受后，spawn 一个后台 tokio task：
   - 创建一个 session（复用 hydra-core 的 SessionManager）
   - 构建 ToolRegistry（参考 `crates/hydra-daemon/src/main.rs` 中 `process_chat_request` 的做法）
   - 用 TurnRunner 执行 turn loop
   - 把 turn 过程中的事件（assistant message、tool call、tool result）转换为 AgentEvent 写入 AgentEventStore
   - turn 正常结束时把状态改为 completed，异常时改为 failed
3. 如果 agent 收到 cancel 命令，需要能中断正在执行的 turn（用 CancellationToken 或类似机制）
4. 如果 agent 进入 waiting_input 状态（比如需要审批），需要等 append_input 命令后继续

参考文件：
- `crates/hydra-daemon/src/api_agent.rs`（当前 mock 实现）
- `crates/hydra-daemon/src/main.rs`（process_chat_request 函数，看它怎么构建 tool registry 和调 turn runner）
- `crates/hydra-core/src/turn/runner.rs`（TurnRunner 的接口）
- `crates/hydra-core/src/turn/permission.rs`（审批机制）

验收标准：
- `POST /api/v1/agents/:id/commands` 发 start 后，agent 真正执行 LLM turn
- 事件流中能看到真实的 agent_message 事件
- cancel 能中断执行
- 现有 10 条 `cargo test -p hydra-daemon` 测试继续通过
- 不要改动 API 接口面（请求/响应 DTO 保持不变）

完成后跑：
cargo test -p hydra-daemon
cargo test -p hydra-core --test contract_connectivity
```

---

## 同学 B：Worktree / Branch API 补全

### 发给 AI 的提示词

```
我在 Hydra 项目中负责补全 Worktree 和 Branch 管理的 REST API。

当前状态：
- `hydra-core` 已经有 `WorktreeManager`（在 `crates/hydra-core/src/git/worktree.rs`），支持 create/list/remove
- `hydra-daemon` 已经有 Agent API 的模块组织方式可以参考（`crates/hydra-daemon/src/api_agent.rs`）
- 但目前没有 Worktree 和 Branch 的 HTTP API

具体要求：

1. 新建 `crates/hydra-daemon/src/api_worktree.rs`，实现：
   - `GET /api/v1/worktrees` — 列出当前项目的所有 worktree
   - `POST /api/v1/worktrees` — 创建新 worktree（请求体：`{"name": "feature-x", "base_ref": "HEAD"}`）
   - `DELETE /api/v1/worktrees/:id` — 删除 worktree
   - 内部复用 `hydra-core::git::worktree::WorktreeManager`

2. 新建 `crates/hydra-daemon/src/api_branch.rs`，实现：
   - `GET /api/v1/branches` — 列出本地分支
   - `POST /api/v1/branches` — 创建新分支（请求体：`{"name": "feat/xxx", "base_ref": "HEAD"}`）
   - `DELETE /api/v1/branches/:name` — 删除分支
   - 内部用 git 命令实现（参考 WorktreeManager 的做法）

3. 在 `crates/hydra-daemon/src/main.rs` 中：
   - 添加 `mod api_worktree;` 和 `mod api_branch;`
   - 注册路由（用 `:param` 语法，不是 `{param}`，因为 axum 版本是 0.7.9）

4. 补契约测试，至少覆盖：
   - 创建 worktree 后能在列表中看到
   - 删除 worktree 后列表中消失
   - 创建分支后能在列表中看到

参考文件：
- `crates/hydra-core/src/git/worktree.rs`（WorktreeManager 实现）
- `crates/hydra-daemon/src/api_agent.rs`（模块组织方式、handler 写法参考）
- `crates/hydra-daemon/src/main.rs`（路由注册位置，搜索 `/api/v1/agents`）

注意事项：
- axum 版本是 0.7.9，路由参数用 `:id` 不是 `{id}`
- 不要加代码注释
- 匹配现有代码风格

验收标准：
- 能通过 API 创建/列出/删除 worktree
- 能通过 API 列出/创建/删除 branch
- 契约测试通过
- 不破坏现有测试

完成后跑：
cargo test -p hydra-daemon
cargo test -p hydra-core --test contract_connectivity
cargo check -p hydra-tuix
```

---

## 同学 C：TUI Agent 面板 + 事件流展示

### 发给 AI 的提示词

```
我在 Hydra 项目中负责升级 TUI 的 Agent 交互体验。

当前状态：
- TUI 已经有一个基础的 `/agents` slash command（在 `crates/hydra-tuix/src/event_loop/commands.rs` 的 `handle_agents` 函数）
- 它能列表、创建、查看详情、发送 start/cancel 命令
- 但目前是一次性输出，没有实时刷新、没有事件流展示、没有交互式输入

具体要求：

1. 增加定时轮询 agent 状态变化：
   - 当用户执行 `/agents <id> start` 后，自动每 500ms 轮询一次 `GET /api/v1/agents/:id`
   - 状态变化时在 scrollback 中显示通知（比如 "agent-xxx: running → completed"）
   - 到达终态（completed/failed/cancelled）后停止轮询

2. 增加事件流展示：
   - `/agents <id> events` 子命令，调用 `GET /api/v1/agents/:id/events`
   - 显示最近的事件列表

3. 增加 waiting_input 交互：
   - 当 agent 进入 waiting_input 状态时，提示用户输入
   - 用户输入后自动发送 `POST /api/v1/agents/:id/commands` with `{"type":"append_input","payload":{"text":"用户输入的内容"}}`

4. 可选：做成 modal 形式（类似 `/resume` 的 session picker）
   - 参考 `crates/hydra-tuix/src/modals/session_picker.rs`

参考文件：
- `crates/hydra-tuix/src/event_loop/commands.rs`（现有 handle_agents 实现，约第 2014 行）
- `crates/hydra-tuix/src/modals/session_picker.rs`（modal 模式参考）
- `crates/hydra-tuix/src/event_loop/mod.rs`（事件循环主逻辑）
- `crates/hydra-tuix/src/render/mod.rs`（渲染接口）

注意事项：
- HTTP 调用必须在独立线程中执行（用 `std::thread::spawn` + `std::sync::mpsc::channel`），不能直接在 tokio runtime 中调 `reqwest::blocking`
- daemon API 地址是 `http://127.0.0.1:13456`（可通过 `HYDRA_DAEMON_PORT` 环境变量覆盖）
- axum 路由参数是 `:id` 格式
- 不要加代码注释
- 匹配现有代码风格

验收标准：
- TUI 能实时看到 agent 状态变化
- 能查看事件流
- waiting_input 时能交互式补输入
- 不破坏现有 TUI 功能

完成后跑：
cargo check -p hydra-tuix
cargo test -p hydra-daemon
```

---

## 合并顺序

1. 同学 B 先合（改 main.rs 路由最多）
2. 同学 A 再合（只改 api_agent.rs 内部）
3. 同学 C 最后合（只改 TUI 侧）

合并前每人都要确保：
```bash
cargo test -p hydra-daemon
cargo test -p hydra-core --test contract_connectivity
cargo check -p hydra-tuix
```
全部通过。
