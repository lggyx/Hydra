# Hydra 项目介绍视频 — 口播稿

**时长**：约 4 分钟
**风格**：技术演示 + 旁白

---

## 开场：为什么需要 Hydra（0:00 — 0:40）

> 如果你做过昇腾 CANN 算子开发，你一定经历过这个：
>
> 每个算子要写三层代码——op_api 定义接口，op_host 处理主机侧逻辑，op_kernel 写设备端实现。
>
> 写完要编译。要用 ops-math 测试框架跑端到端测试。要看 gcov 覆盖率。要调性能，跟手写基线对比不能超过 5%。要对精度，误差不能超过 1e-6。
>
> 一个算子一套流程。十个算子就是十套。二十个算子呢？
>
> 而且这些代码结构高度模式化——op_api、op_host、op_kernel 三层骨架是固定的，真正变化的只是算子本身的数学逻辑。这种重复性工作，恰恰是 AI 最擅长的。
>
> 所以我们在想：能不能不只是一个 AI 帮你写，而是一整支 AI 团队帮你干？

**画面**：ops-math 目录结构 → 快速滚动 op_api/op_host/op_kernel 代码 → 展示三层模式示意图。

---

## Hydra 是什么（0:40 — 1:20）

> Hydra 是一个面向昇腾 CANN 算子开发测试的 AI 原生多智能体系统。
>
> 记住这个词——多智能体。它不是一个聊天窗口里的一问一答。它是一支 AI 团队。
>
> 团队里有三种角色：
> - **OrchestratorAgent**，项目经理。它负责接任务、拆分子任务、派发给合适的开发工程师、跟踪进度、最后汇总。
> - **ExecutionAgent**，开发工程师。每个 ExecutionAgent 独立负责一个算子——从读规格到写三层代码到编译测试，全流程自己搞定。
> - 还有一个规划中的 **ReviewerAgent**，审查员。它会接入 cannbot-skills 审查层，自动做 lint、正确性检查、性能对比、精度验证，不通过的自动打回返工。
>
> 最关键的是——这些 Agent 共享一个统一的 Agent trait 接口。新增任何类型的 Agent，不需要改一行编排代码。
>
> 而且它们是真正并行执行的。三个算子？三个 ExecutionAgent 同时开工。

**画面**：架构动画——从 Orchestrator 扇出到多个 ExecutionAgent，再汇聚到 cannbot-skills 审查层。展示 Agent trait 代码片段（5 秒）。

---

## 一行安装（1:20 — 1:35）

> 安装只需要一行命令。

```bash
curl -fsSL https://raw.githubusercontent.com/lggyx/Hydra/main/install.sh | bash
```

> 脚本会自动检测你的环境——Linux、macOS、Windows 都支持。没有 Rust？自动装。没有 git？自动装。然后 clone 仓库、编译、部署到 ~/.hydra/bin。
>
> 装完就有两个命令：`hydra-daemon` 启动 API 服务，`hydra` 启动终端界面。

**画面**：终端运行安装命令，显示完整安装过程，最后 `hydra --version`。

---

## 实战演示：三个算子并行开发（1:35 — 2:50）

> 现在来看真实操作。我们要给 ops-math 项目实现 Mul、Add、Pow 三个算子的端到端测试。

**画面**：终端1 启动 `hydra-daemon`，终端2 启动 `hydra`。

> 先登录，领取免费 API 配额。

```
/login
```

> 然后创建一个 orchestrator 类型的 Agent。

```
/agents create --kind orchestrator
```

> 把任务交给它——一句话描述你要什么。

```
/agents <id> start "实现 ops-math 中 Mul、Add、Pow 算子的
  op_api/op_host/op_kernel 三层代码，并编写端到端测试"
```

> 现在看 TUI 的实时事件流。
>
> Orchestrator 收到任务，开始分析。它识别出三个独立算子，然后连续调用了三次 spawn_execution 工具——不是人在操作，是 AI 自己在调度。
>
> 三个 ExecutionAgent 马上并行启动。一个在写 Mul，一个在写 Add，一个在写 Pow。它们各自去读 ops-math 规格、编写三层代码、用 CANN 工具链编译、跑 ops-math 测试。
>
> 你看这些事件——`[agent-id] calling read_file`、`[agent-id] calling write_file`、`[agent-id] calling bash`——每一步都在你眼前发生，不是黑盒。

**画面**：TUI 全屏录制，实时滚动事件流，清晰可见 spawn_execution / read_file / write_file / bash 调用。

> 我们切到另一个 agent 看看。

```
/agents <execution_id> events
```

> 可以看到这个 agent 的完整事件历史——读了什么文件、写了什么代码、编译结果是什么、测试通过没有。全部可追溯。

---

## 审查与迭代（2:50 — 3:15）

> 所有算子写完之后，代码不会直接合入。它会进入 cannbot-skills 审查层。
>
> 审查层自动做四件事：
> - **静态分析**——lint、类型安全、内存边界检查
> - **正确性检查**——跟参考实现对比
> - **性能基准对比**——跟手写基线跑分，标记偏差超过 5% 的回归
> - **精度验证**——跟 golden output 做容差感知的对比
>
> 如果有算子没通过，审查层会把详细的失败原因和修复建议送回给对应的 ExecutionAgent。Agent 拿着反馈自己改、重新编译、重新测试、重新提审。
>
> 这个循环是全自动的，直到全部通过。

**画面**：审查流程图——算子代码 → cannbot-skills 四道检查 → 通过/打回 → 自动迭代。

---

## 多轮交互：人在回路中（3:15 — 3:35）

> 如果 Orchestrator 在执行过程中需要你做决策——比如选什么精度、用哪个版本的 CANN 工具链——它会自动进入 waiting_input 状态。

```
agent-orch: → waiting_input
agent-orch: waiting for input. Use /agents <id> input <text> to respond.
```

> 而且它会给你展示当前所有子 agent 的进度状态。你可以直接输入指令，继续对话。

```
/agents <id> input "所有算子使用 float32，目标平台 Ascend 910B"
```

> Orchestrator 收到指令，唤醒继续执行。这是一个真正的多轮协作流程——人和 AI 团队一起工作。

**画面**：TUI 展示 waiting_input 状态 → 输入指令 → Agent 恢复执行。

---

## 结尾（3:35 — 4:00）

> Hydra 的核心思想很简单：
>
> 让一支 AI 团队替你写算子、跑测试、做审查。你只需要告诉它做什么，它自己决定怎么做、谁来做。
>
> 开源，MIT 协议。支持 Anthropic、OpenAI、DeepSeek、MiniMax、GLM、Qwen、Ollama——任何 OpenAI 兼容的 LLM 都能接入。
>
> 我们在持续迭代：接下来会实现 ReviewerAgent、完善 cannbot-skills 集成、支持更多 CANN 算子类型。
>
> GitHub 链接在下方。一行命令，立刻开始。让 AI 去干活，你去做更重要的事。

**画面**：GitHub 仓库二维码 + 安装命令 + cannbot-skills 链接。

---

## 录制要点

| 环节 | 画面 | 时长 |
|------|------|------|
| 问题引入 | ops-math 代码 + 三层模式图 | 40s |
| Hydra 介绍 | 架构动画 + Agent trait 代码 | 40s |
| 一行安装 | 终端 curl → 安装过程 | 15s |
| 实战演示 | TUI 全屏录制（create → start → spawn → events） | 75s |
| 审查与迭代 | 审查流程图 | 25s |
| 多轮交互 | waiting_input → input → 恢复 | 20s |
| 结尾 | GitHub 链接 + CTA | 25s |

## 终端录制命令参考

```bash
# 窗口1：daemon 启动
hydra-daemon

# 窗口2：TUI 操作
hydra

# TUI 内依次输入：
/login
/agents create --kind orchestrator
# 假设返回 id: abc123
/agents abc123 start "实现 ops-math 中 Mul、Add、Pow 算子的 op_api/op_host/op_kernel 三层代码，并编写端到端测试"
# 等待 spawn_execution 事件出现
/agents                          # 展示所有 agent
/agents abc123                   # orchestrator 详情
/agents <exec_id> events         # 子 agent 事件流
# 如果进入 waiting_input：
/agents abc123 input "使用 float32 精度，目标平台 Ascend 910B"
```

## 关键画面 Checklist

- [ ] ops-math 项目目录结构（op_api/op_host/op_kernel 路径可见）
- [ ] 架构图（Orchestrator → ExecutionAgent ×3 → cannbot-skills）
- [ ] 终端安装过程（curl | bash 完整输出）
- [ ] TUI：`/agents create --kind orchestrator`
- [ ] TUI：`/agents start` 后实时事件流（spawn_execution / read_file / write_file）
- [ ] TUI：`/agents` 列表显示多个 agent 并行运行
- [ ] TUI：`/agents <id> events` 事件历史
- [ ] TUI：waiting_input → input 多轮交互
- [ ] 审查流程图
- [ ] GitHub 链接 + 安装命令截图
