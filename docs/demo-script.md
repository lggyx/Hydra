# Hydra 项目介绍视频 — 口播稿

**时长**：约 4 分 15 秒
**风格**：技术演示 + 数据对比 + 旁白
**基于**：README v4.23.3（含对比数据、架构图、Mermaid 图）

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
> 这些代码结构高度模式化——op_api、op_host、op_kernel 三层骨架是固定的，真正变化的只是算子本身的数学逻辑。这种重复性工作，恰恰是 AI 最擅长的。
>
> 但问题是——单靠一个 AI 聊天窗口，写二十个算子还是会累。上下文疲劳、遗漏边界情况、性能参差不齐。
>
> 所以我们在想：能不能不只是一个 AI 帮你写，而是一整支 AI 团队帮你干？

**画面**：ops-math 目录结构 → 快速滚动 op_api/op_host/op_kernel 代码 → 三层模式对比示意图。

---

## Hydra 是什么（0:40 — 1:25）

> Hydra 是一个面向昇腾 CANN 算子开发测试的 AI 原生多智能体系统。
>
> 记住这个词——**多智能体**。它不是一个聊天窗口里的一问一答。它是一支 AI 团队。
>
> 这是我们的系统拓扑图——

**画面**：展示 README 中的 System Topology Mermaid 图。

> 从下往上看：
> - 最底层是 **CANN 审查层**，接入 cannbot-skills，所有算子代码都要通过四道检查。
> - 往上是 **Agent 层**——OrchestratorAgent 当项目经理，多个 ExecutionAgent 当开发工程师，并行工作。
> - 再往上是 **核心引擎**——ResourceManager 管理 Agent 生命周期，EventBus 做事件扇出。
> - 最上面是 **用户界面**——TUI 终端和 CLI。
>
> 团队里三种角色：
> - **OrchestratorAgent**，项目经理。接任务、拆分子任务、派发给开发工程师、跟踪进度。
> - **ExecutionAgent**，开发工程师。每个独立负责一个算子，从读规格到写三层代码到编译测试。
> - **ReviewerAgent**，审查员（规划中），接入 cannbot-skills，自动 lint、正确性、性能、精度四道检查。
>
> 最关键的是——这些 Agent 共享一个统一的 Agent trait 接口。新增任何类型，不改一行编排代码。
>
> 而且它们是真正**并行**的。三个算子？三个 ExecutionAgent 同时开工。十个算子？十个同时开工。

**画面**：架构图 → Agent Lifecycle 状态图 → 代码片段。

---

## 数据说话：多智能体 vs 单智能体（1:25 — 2:00）

> 我们不是空口说白话。我们做了严格的对比测试。
>
> 相同任务：给 ops-math 实现 Mul、Add、Pow 三个算子。相同 LLM。唯一变量是架构。

**画面**：左右分屏对比卡片——左侧 Hydra（紫色边框），右侧 OpenCode（灰色边框）。

> Hydra 多智能体：30 个测试用例，**96.7%** 通过率。行覆盖率 **87.1%**，分支覆盖率 **77.1%**。质量评分 **4.7 分**。P0 问题 **零个**。
>
> OpenCode 单智能体：同样的 30 个测试用例，通过率只有 **73.3%**。行覆盖率 58.4%，分支覆盖率 42.6%。质量评分 **2.3 分**。P0 问题 **5 个**——内存泄漏、边界越界、类型转换错误。
>
> 性能方面：Hydra 产出算子的平均执行时间 **48.3 微秒**，OpenCode 是 112.7 微秒——慢了 **2.3 倍**。
>
> 开发速度：Hydra 并行执行，三个算子 **3 分钟**搞定。单智能体串行，要 **8 分钟**——慢了 **2.7 倍**。
>
> 为什么差距这么大？因为单智能体在做长任务时会上下文疲劳，越往后越容易出错。而多智能体并行、专注、互相审查。

**画面**：对比表格逐行高亮，最后定格在提升列。

---

## 一行安装（2:00 — 2:15）

> 安装只需要一行命令。

```bash
curl -fsSL https://raw.githubusercontent.com/lggyx/Hydra/main/install.sh | bash
```

> Linux、macOS、Windows 都支持。没有 Rust？自动装。没有 git？自动装。clone 仓库、编译、部署到 `~/.hydra/bin`。全程不用你操心。

**画面**：终端运行安装命令，完整安装过程，最后 `hydra --version` 验证。

---

## 实战演示：三个算子并行开发（2:15 — 3:15）

> 来看真实操作。我们要给 ops-math 实现 Mul、Add、Pow 三个算子。

**画面**：终端1 启动 `hydra-daemon`，终端2 启动 `hydra` TUI。

> 先登录，领取免费 API 配额。然后创建一个 orchestrator 类型的 Agent。

```
/login
/agents create --kind orchestrator
```

> 一句话描述你要什么。

```
/agents <id> start "实现 ops-math 中 Mul、Add、Pow 算子的
  op_api/op_host/op_kernel 三层代码，并编写端到端测试"
```

> 现在注意看 TUI 的实时事件流。这个是 SSE 实时推送，不是轮询。
>
> Orchestrator 收到任务，开始分析。它自动识别出三个独立算子，连续调用了三次 `spawn_execution`——不是人在操作，是 **AI 在调度 AI**。
>
> 三个 ExecutionAgent 马上并行启动。你看这些事件——`calling read_file`、`calling write_file`、`calling bash`——每一步都在你眼前发生，不是黑盒。

**画面**：TUI 全屏录制，实时滚动事件流，清晰可见 spawn_execution / read_file / write_file / bash。

> 切到子 agent 的事件历史——

```
/agents <execution_id> events
```

> 完整的执行记录：读了什么文件、写了什么代码、编译结果、测试通过率。**全部可追溯。**

**画面**：`/agents` 列出多个并行 agent，`/agents <id> events` 展示事件历史。

---

## 审查门禁（3:15 — 3:35）

> 算子写完，代码不会直接合入。自动进入 cannbot-skills 审查层。
>
> 这就是刚才系统拓扑图里最下面的那一层——四道检查：
> 1. **静态分析**——lint、类型安全、内存边界
> 2. **正确性检查**——跟参考实现逐行对比
> 3. **性能基准**——跟手写基线跑分，偏差超过 5% 自动标记
> 4. **精度验证**——跟 golden output 做容差感知对比
>
> 如果有算子没通过，审查层把失败原因和修复建议送回给对应的 ExecutionAgent。Agent 拿着反馈自己改、重新编译、重新测试、重新提审。全自动循环，直到全部通过。

**画面**：CANN Operator Workflow Mermaid 图——算子代码 → cannbot-skills 四道检查 → 通过/打回 → 自动迭代。

---

## 多轮交互：人在回路（3:35 — 3:55）

> 如果 Orchestrator 需要你做决策——比如选什么精度、用哪个 CANN 版本——它会自动进入 `waiting_input` 状态。

```
agent-orch: → waiting_input
agent-orch: waiting for input. Use /agents <id> input <text> to respond.
```

> 而且它会展示所有子 agent 的**实时进度**——谁在跑、谁跑完了、谁卡住了——一目了然。
>
> 你可以直接输入指令继续对话。这是一个真正的**人 + AI 团队协作**流程。

```
/agents <id> input "所有算子使用 float32 精度，目标平台 Ascend 910B"
```

**画面**：TUI 展示 waiting_input → 子 agent 进度 → 输入指令 → Agent 恢复执行。

---

## 结尾（3:55 — 4:15）

> 回顾一下：
>
> Hydra 的核心思想——让一支 AI 团队替你写算子、跑测试、做审查。
>
> 数据证明——多智能体比单智能体：
> - 通过率高出 **23 个百分点**
> - 代码覆盖率高出 **29 个百分点**
> - 执行速度快 **2.3 倍**
> - 开发速度快 **2.7 倍**
>
> 开源，MIT 协议。支持 Anthropic、OpenAI、DeepSeek、GLM、Qwen、Ollama 等任何 LLM。
>
> 下一步规划：ReviewerAgent 正式接入 cannbot-skills、支持更多 CANN 算子类型、性能自动调优。
>
> GitHub 链接在下方。一行命令，立刻开始。
>
> **让 AI 团队去干活，你去做更重要的事。**

**画面**：GitHub 仓库地址 + 安装命令 + cannbot-skills 链接 + 数据对比定格。

---

## 录制要点

| 环节 | 画面 | 时长 |
|------|------|------|
| 问题引入 | ops-math 代码 + 三层模式图 | 40s |
| Hydra 介绍 | 系统拓扑图 + 生命周期图 + Agent trait 代码 | 45s |
| 数据对比 | 左右分屏对比卡片，逐行高亮 | 35s |
| 一行安装 | 终端 curl → 完整安装过程 | 15s |
| 实战演示 | TUI 全屏（create → start → spawn → events） | 60s |
| 审查门禁 | CANN 工作流 Mermaid 图 | 20s |
| 多轮交互 | waiting_input → 进度展示 → input | 20s |
| 结尾 | 数据定格 + GitHub + CTA | 20s |

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
/agents                          # 展示所有 agent 及状态
/agents abc123                   # orchestrator 详情
/agents <exec_id> events         # 子 agent 事件历史
# 如果进入 waiting_input：
/agents abc123 input "使用 float32 精度，目标平台 Ascend 910B"
```

## 关键画面 Checklist

- [ ] ops-math 项目目录结构（op_api/op_host/op_kernel 路径可见）
- [ ] System Topology Mermaid 图（UI → API → Core → Agent → CANN Review）
- [ ] Agent Lifecycle 状态图（Created → Running → WaitingInput → Completed）
- [ ] 左右分屏对比卡片（Hydra 4.7 紫色 vs OpenCode 2.3 灰色）
- [ ] 对比数据表格逐行高亮（2.3x faster / 2.7x faster dev）
- [ ] 终端安装过程（curl | bash 完整输出）
- [ ] TUI：`/agents create --kind orchestrator`
- [ ] TUI：`/agents start` 后实时事件流（spawn_execution / read_file / write_file）
- [ ] TUI：`/agents` 列表显示多个 agent 并行
- [ ] TUI：`/agents <id> events` 事件历史
- [ ] CANN 工作流 Mermaid 图（审查层四道检查 + 打回循环）
- [ ] TUI：waiting_input → input 多轮交互
- [ ] GitHub 链接 + 安装命令 + 数据定格截图

## 旁白语气指引

- 开场：痛点共情，平实叙述
- 架构：自信、清晰，像在讲解白板
- 数据对比：客观有力，"我们做了对比测试"，让数字说话
- 安装：轻快，"一行命令"
- 演示：兴奋但不夸张，实时事件流本身就很有冲击力
- 审查：逻辑清晰，四步逐一说明
- 结尾：总结 + 数据回顾 + CTA，给观众留下数字印象
