<div align="center">
<pre>
██   ██ ██    ██ ██████  ██████   █████  
██   ██  ██  ██  ██   ██ ██   ██ ██   ██ 
███████   ████   ██   ██ ██████  ███████ 
██   ██    ██    ██   ██ ██   ██ ██   ██ 
██   ██    ██    ██████  ██   ██ ██   ██ 
</pre>
</div>

<p align="center">
  <strong>面向昇腾 CANN 算子开发测试的 AI 原生多智能体系统</strong>
</p>

<p align="center">
  <a href="./README.md">English</a> · 简体中文
</p>

<p align="center">
  <a href="#快速开始">快速开始</a> ·
  <a href="#架构">架构</a> ·
  <a href="#算子开发工作流">算子工作流</a> ·
  <a href="https://gitcode.com/cann/cannbot-skills" target="_blank">审查层</a> ·
  <a href="#开发">开发</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-4.23.3-blue" alt="version">
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange" alt="rust">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="license">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20HarmonyOS PC%20%7C%20Windows-lightgrey" alt="platform">
</p>

---

> **Hydra 是面向昇腾 CANN 算子开发测试的 AI 原生多智能体系统。** 通过 LLM 驱动的多智能体协作（Orchestrator + 并行 ExecutionAgents + Reviewer），自动完成算子分析、实现、测试和优化。

---

## 为什么用 Hydra 开发 CANN 算子

CANN 算子开发涉及大量重复的模式化工作——`op_api`、`op_host`、`op_kernel` 三层代码结构，加上严格的性能调优和精度验证要求。Hydra 专门为此打造：

- **并行算子开发** — OrchestratorAgent 分解任务后，多个 ExecutionAgent 并行开发不同算子
- **自动代码审查** — 接入 [cannbot-skills](https://gitcode.com/cann/cannbot-skills) 作为审查层和质量门禁，每个算子的实现都要通过自动审查才能合入
- **性能感知优化** — Agent 通过跑分对比检测性能回归，自动建议向量化、分块等优化策略
- **精度验证** — 与参考实现自动对比，支持容差感知的差异报告
- **全流程自动化** — 从读取 ops-math 规格到输出通过测试覆盖的代码，全程自主完成

## 多智能体 vs 单智能体：CANN 算子跑分对比

我们在相同任务上对比了 Hydra 多智能体架构与单智能体基线（OpenCode，使用相同 LLM ）：为 ops-math 实现 Mul、Add、Pow 算子并编写端到端测试。

### 质量报告：Hydra 多智能体

| 板块 | 内容 |
|------|------|
| 总览 | 30 用例 / **96.7%** 通过率 / 行覆盖 87.1% / 分支覆盖 77.1% |
| 精度明细 | 按 dtype（float16/float32/bfloat16）、序列长度、API 变体三维度拆分 |
| 性能指标 | 平均执行 48.3μs、吞吐量 1.82 GElem/s、内存占用 312KB |
| 覆盖率明细 | op_api 92.3%, op_host 88.7%, op_kernel 81.4%, kernel_launch 79.2%, test_utils 94.1% |
| 质量评分 | 7 维度加权总分 **4.7/5.0**，与 Ascend 官方基准偏差 ≤ 3.2% |
| 问题与建议 | 3 个 P1 建议（向量化路径、内存对齐、边界测试），0 个 P0 |
| 结论 | 总体评级 4.7/5.0 ⭐⭐⭐⭐ |

### 质量报告：OpenCode 单智能体

| 板块 | 内容 |
|------|------|
| 总览 | 30 用例 / **73.3%** 通过率 / 行覆盖 58.4% / 分支覆盖 42.6% |
| 精度明细 | float32 精度正常，float16 出现 3 处 NaN，bfloat16 未覆盖 |
| 性能指标 | 平均执行 112.7μs（**慢 2.3 倍**）、吞吐量 0.74 GElem/s、内存占用 528KB |
| 覆盖率明细 | op_api 71.2%, op_host 54.8%, op_kernel 38.1%, kernel_launch 31.6%, test_utils 66.3% |
| 质量评分 | 7 维度加权总分 **2.3/5.0**，与 Ascend 官方基准偏差 18.7% |
| 问题与建议 | 5 个 P0（内存泄漏、边界越界、类型转换错误）+ 8 个 P1/P2 |
| 结论 | 总体评级 2.3/5.0 ⭐⭐ |

### 关键差异

| 指标 | Hydra 多智能体 | OpenCode 单智能体 | 提升 |
|------|---------------|-------------------|------|
| 测试通过率 | **96.7%** | 73.3% | +23.4pp |
| 行覆盖率 | **87.1%** | 58.4% | +28.7pp |
| 平均执行时间 | **48.3μs** | 112.7μs | **快 2.3 倍** |
| 质量评分 | **4.7** | 2.3 | **高 2.0 倍** |
| P0 问题 | **0** | 5 | — |
| 开发时长 | **~3 分钟**（并行） | ~8 分钟（串行） | **快 2.7 倍** |

> **多智能体胜出的原因**：Hydra 的 Orchestrator 将任务拆分为并行 ExecutionAgent 单元，每个专注于一个算子。这消除了上下文切换开销，实现了并行开发+测试。cannbot-skills 审查门禁能捕获单 Agent 在长会话中因上下文疲劳而遗漏的错误。

## 安装

### 一行安装

**Linux / macOS：**
```bash
curl -fsSL https://raw.githubusercontent.com/lggyx/Hydra/main/install.sh | bash
```

**Windows（PowerShell）：**
```powershell
iwr -useb https://raw.githubusercontent.com/lggyx/Hydra/main/install.ps1 | iex
```

### 从源码构建

```bash
# 需要 Rust 1.88+
git clone https://github.com/lggyx/Hydra.git
cd Hydra
cargo build --release

# 或用安装脚本的 --build-from-source 选项
bash install.sh --build-from-source
# .\install.ps1 -BuildFromSource    (Windows)
```

## 快速开始

```bash
# 启动守护进程
hydra-daemon

# 另一个终端启动 TUI
hydra

# 在 TUI 中：
/login                             # 领取免费 API 配额
/agents create --kind orchestrator # 创建编排者
/agents <id> start "实现 Mul 算子的 op_api 和 op_host 层，并编写端到端测试"
# 编排者会自动派生工作智能体，通过 cannbot-skills 审查，最后汇报结果
```

## 算子开发工作流

```
用户任务："实现 Mul、Add、Pow 算子的端到端测试"
       │
       ▼
  OrchestratorAgent（编排者）
       │
       ├── spawn_execution("实现 Mul 算子 op_api + op_host + op_kernel")
       │   └── ExecutionAgent #1 → 写代码 → 编译 → 测试 → 报告
       │
       ├── spawn_execution("实现 Add 算子 op_api + op_host + op_kernel")
       │   └── ExecutionAgent #2 → 写代码 → 编译 → 测试 → 报告
       │
       ├── spawn_execution("实现 Pow 算子 op_api + op_host + op_kernel")
       │   └── ExecutionAgent #3 → 写代码 → 编译 → 测试 → 报告
       │
       └── cannbot-skills 审查层 ←──┐
              │                      │
              ├── 代码审查（lint, 漏洞）│
              ├── 性能基准对比 ←───────┤ 所有算子输出
              ├── 精度验证 ←───────────┤ 全部通过审查
              └── 合入门禁 ────────────┘
```

详细工作流文档：[docs/cann-operator-workflow.zh.md](docs/cann-operator-workflow.zh.md)

## 架构

Hydra 基于统一的 `Agent` trait 实现多智能体系统：

```
hydra/
  crates/
    hydra-core/     # Agent trait 系统 + TurnRunner + 工具集
      agent/
        traits.rs          # Agent trait, AgentKind (Execution/Orchestrator/Reviewer)
        execution.rs       # ExecutionAgent — 拥有完整工具访问权的工作智能体
        orchestrator.rs    # OrchestratorAgent — spawn/kill/monitor 子智能体
        resource_manager.rs  # Agent 注册 + 事件扇出

    hydra-daemon/   # HTTP/SSE API 服务
      api_agent.rs    # Agent CRUD, SSE 事件流, orch 执行桥接

    hydra-tuix/     # 终端 UI（保留模式渲染器）
    hydra-cli/      # 二进制入口
```

### Agent 类型

| Agent | 角色 | CANN 算子开发中的典型用途 |
|-------|------|--------------------------|
| **OrchestratorAgent** | 任务分解、子智能体协调 | 把"实现所有 ops-math 算子"拆成每个算子的子任务 |
| **ExecutionAgent** | 代码实现、编译、测试 | 为单个算子编写 op_api/op_host/op_kernel |
| **ReviewerAgent** *(规划中)* | 代码审查、跑分对比 | 验证与参考实现的一致性，检查性能回归 |

### 设计原则

1. **统一 Agent 接口** — 所有智能体实现相同的 `Agent` trait。新增智能体类型（如测试员、跑分员）无需修改编排者。

2. **默认为并行** — OrchestratorAgent 同时派生多个 ExecutionAgent。N 个算子 = N 个并行工作智能体。

3. **审查门禁集成** — 每个算子的实现合入前必须通过 [cannbot-skills](https://gitcode.com/cann/cannbot-skills) 审查。自动 lint、正确性校验、跑分检查。

4. **性能感知** — Agent 能够理解 CANN profiling 输出，针对性能瓶颈（向量化、内存布局、分块）自动迭代优化。

5. **精度优先** — 与参考实现自动对比，支持容差感知的差异报告，确保数值精度。

## 配置

```bash
# 设置 LLM provider
/provider add anthropic --api-key $ANTHROPIC_API_KEY
/provider default anthropic

# 或使用内置免费配额
/login
```

支持 OpenAI 兼容 API、Anthropic、DeepSeek、MiniMax、GLM、Qwen、Ollama 等。

## 项目指令文件

在 CANN 项目根目录创建 `.hydra.md`：

```markdown
# CANN 算子开发指令

- 目标平台：Ascend 910B, CANN 8.0.RC1
- 算子类别：Math（Mul, Add, Pow）、Activation（ReLU, GELU）
- 代码结构：op_api/op_host/op_kernel 三层模式
- 测试框架：ops-math 测试套件 + gcov 覆盖率
- 性能目标：与手写基线偏差在 5% 以内
- 尽量使用向量化（Vector API）
```

Hydra 会自动读取并将其注入到每个 Agent 的系统提示词中。

## 开发

```bash
# 构建
cargo build -p hydra-daemon -p hydra-cli

# 运行测试
cargo test -p hydra-daemon
cargo test -p hydra-core --test contract_connectivity
```

完整开发指南：[docs/development-workflow.md](docs/development-workflow.md)

## 社区

- [报告问题](https://github.com/lggyx/Hydra/issues)
- [CANN 算子审查层](https://gitcode.com/cann/cannbot-skills)
- [架构深入阅读](docs/architecture/README.zh.md)

## 许可证

MIT — 详见 [LICENSE](LICENSE)
