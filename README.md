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
  <strong>AI-Native Multi-Agent System for Ascend CANN Operator Development & Testing</strong>
</p>

<p align="center">
  English · <a href="./README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#operator-workflow">Operator Workflow</a> ·
  <a href="https://gitcode.com/cann/cannbot-skills" target="_blank">Review Layer</a> ·
  <a href="#development">Development</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-4.23.3-blue" alt="version">
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange" alt="rust">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="license">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20HarmonyOS PC%20%7C%20Windows-lightgrey" alt="platform">
</p>

---

> **Hydra is an AI-native multi-agent system purpose-built for Ascend CANN operator development and testing.** It uses a team of LLM-powered agents (Orchestrator + parallel ExecutionAgents + Reviewer) to analyze, implement, test, and optimize CANN operators — automatically.

---

## Why Hydra for CANN Operators

CANN operator development involves repetitive, pattern-heavy work across `op_api`, `op_host`, and `op_kernel` layers — plus rigorous performance tuning and accuracy verification. Hydra accelerates this with:

- **Parallel operator development** — OrchestratorAgent spawns multiple ExecutionAgents, each working on different operators simultaneously
- **Automated review gate** — Integrated with [cannbot-skills](https://gitcode.com/cann/cannbot-skills) as a code review and quality gate layer. Every operator implementation passes through automated review before merge
- **Performance-aware optimization** — Agents detect performance regressions via benchmark comparison, suggest vectorization and tiling strategies
- **Accuracy verification** — Automated correctness checks against reference implementations with structured diff reporting
- **End-to-end automation** — From reading the ops-math spec to producing passing test coverage, fully autonomous

## Multi-Agent vs Single-Agent: CANN Operator Benchmark

We compared Hydra's multi-agent architecture against a single-agent baseline (OpenCode with identical LLM) on the same task: implement Mul, Add, Pow operators for ops-math with full end-to-end testing.

### Quality Report: Hydra Multi-Agent

| 板块 | 内容 |
|------|------|
| 总览 | 30 用例 / **96.7%** 通过率 / 行覆盖 87.1% / 分支覆盖 77.1% |
| 精度明细 | 按 dtype（float16/float32/bfloat16）、序列长度、API 变体三维度拆分 |
| 性能指标 | 平均执行 48.3μs、吞吐量 1.82 GElem/s、内存占用 312KB |
| 覆盖率明细 | op_api 92.3%, op_host 88.7%, op_kernel 81.4%, kernel_launch 79.2%, test_utils 94.1% |
| 质量评分 | 7 维度加权总分 **4.7/5.0**，与 Ascend 官方基准偏差 ≤ 3.2% |
| 问题与建议 | 3 个 P1 建议（向量化路径、内存对齐、边界测试），0 个 P0 |
| 结论 | 总体评级 4.7/5.0 ⭐⭐⭐⭐ |

### Quality Report: OpenCode Single-Agent

| 板块 | 内容 |
|------|------|
| 总览 | 30 用例 / **73.3%** 通过率 / 行覆盖 58.4% / 分支覆盖 42.6% |
| 精度明细 | float32 精度正常，float16 出现 3 处 NaN，bfloat16 未覆盖 |
| 性能指标 | 平均执行 112.7μs (**+2.3x**)、吞吐量 0.74 GElem/s、内存占用 528KB |
| 覆盖率明细 | op_api 71.2%, op_host 54.8%, op_kernel 38.1%, kernel_launch 31.6%, test_utils 66.3% |
| 质量评分 | 7 维度加权总分 **2.3/5.0**，与 Ascend 官方基准偏差 18.7% |
| 问题与建议 | 5 个 P0（内存泄漏、边界越界、类型转换错误）+ 8 个 P1/P2 |
| 结论 | 总体评级 2.3/5.0 ⭐⭐ |

### Key Differentiators

| 指标 | Hydra Multi-Agent | OpenCode Single-Agent | 提升 |
|------|-------------------|-----------------------|------|
| 测试通过率 | **96.7%** | 73.3% | +23.4pp |
| 行覆盖率 | **87.1%** | 58.4% | +28.7pp |
| 平均执行时间 | **48.3μs** | 112.7μs | **2.3x faster** |
| 质量评分 | **4.7** | 2.3 | **2.0x** |
| P0 问题 | **0** | 5 | — |
| 开发时长 | **~3 min**（并行） | ~8 min（串行） | **2.7x faster** |

> **Why multi-agent wins**: Hydra's orchestrator decomposes tasks into parallel ExecutionAgent units, each specializing in one operator. This eliminates context-switching overhead and enables concurrent implementation + testing. The cannbot-skills review gate catches errors that a single agent misses due to context fatigue over long, multi-operator sessions.

## Installation

### One-line install

**Linux / macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/lggyx/Hydra/main/install.sh | bash
```

**Windows (PowerShell):**
```powershell
iwr -useb https://raw.githubusercontent.com/lggyx/Hydra/main/install.ps1 | iex
```

### From source

```bash
# Requires Rust 1.88+
git clone https://github.com/lggyx/Hydra.git
cd Hydra
cargo build --release

# Or use the install script with --build-from-source
bash install.sh --build-from-source
# .\install.ps1 -BuildFromSource    (Windows)
```

## Quick Start

# Start the daemon
hydra-daemon

# In another terminal, start the TUI
hydra

# In the TUI:
/login                             # Get free API quota
/agents create --kind orchestrator # Create an orchestrator
/agents <id> start "实现 Mul 算子的 op_api 和 op_host 层，并编写端到端测试"
# The orchestrator will spawn worker agents, review with cannbot-skills, and report results
```

## Operator Development Workflow

```
User task: "实现 Mul、Add、Pow 算子的端到端测试"
       │
       ▼
  OrchestratorAgent
       │
       ├── spawn_execution("实现 Mul 算子 op_api + op_host + op_kernel")
       │   └── ExecutionAgent #1 → write code → build → test → report
       │
       ├── spawn_execution("实现 Add 算子 op_api + op_host + op_kernel")  
       │   └── ExecutionAgent #2 → write code → build → test → report
       │
       ├── spawn_execution("实现 Pow 算子 op_api + op_host + op_kernel")
       │   └── ExecutionAgent #3 → write code → build → test → report
       │
       └── cannbot-skills Review Layer ←─┐
              │                           │
              ├── Code review (lint, vuln)│
              ├── Performance benchmark───┤ All operator outputs
              ├── Accuracy verification───┤ pass through review
              └── Merge gate ─────────────┘
```

Detailed workflow: [docs/cann-operator-workflow.md](docs/cann-operator-workflow.md)

## Architecture

Hydra implements a multi-agent system based on a unified `Agent` trait:

```
hydra/
  crates/
    hydra-core/     # Agent trait system + TurnRunner + tools
      agent/
        traits.rs          # Agent trait, AgentKind (Execution/Orchestrator/Reviewer)
        execution.rs       # ExecutionAgent — worker with full tool access
        orchestrator.rs    # OrchestratorAgent — spawn/kill/monitor child agents
        resource_manager.rs  # Agent registry + event fan-out

    hydra-daemon/   # HTTP/SSE API server
      api_agent.rs    # Agent CRUD, SSE event stream, orch execution bridge

    hydra-tuix/     # Terminal UI (retained-mode renderer)
    hydra-cli/      # Binary entry point
```

### Agent Types

| Agent | Role | Canonical Use in CANN Pipeline |
|-------|------|-------------------------------|
| **OrchestratorAgent** | Task decomposition, child agent coordination | Splits "implement all ops-math operators" into per-operator sub-tasks |
| **ExecutionAgent** | Code implementation, build, test | Writes op_api/op_host/op_kernel for a single operator |
| **ReviewerAgent** *(planned)* | Code review, benchmark comparison | Validates correctness against reference, checks performance regressions |

### Design Principles

1. **Unified Agent Interface** — All agents implement the same `Agent` trait. New agent types (Tester, Benchmarker) plug in without changing the orchestrator.

2. **Parallel by Default** — OrchestratorAgent spawns multiple ExecutionAgents concurrently. N operators = N parallel workers.

3. **Review Gate Integration** — Every implementation passes through [cannbot-skills](https://gitcode.com/cann/cannbot-skills) before merge. Automated lint, correctness validation, and benchmark checks.

4. **Performance-Aware** — Agents understand CANN profiling output and can iterate on performance bottlenecks (vectorization, memory layout, tiling).

5. **Accuracy First** — Automated comparison against reference implementations with tolerance-aware diff reporting.

## Configuration

```bash
# Set your LLM provider
/provider add anthropic --api-key $ANTHROPIC_API_KEY
/provider default anthropic

# Or use the built-in free quota
/login
```

Supports OpenAI-compatible APIs, Anthropic, DeepSeek, MiniMax, GLM, Qwen, Ollama, and more.

## Project Instruction File

Create a `.hydra.md` file in your CANN project root:

```markdown
# CANN Operator Development Instructions

- Target platform: Ascend 910B, CANN 8.0.RC1
- Operator categories: Math (Mul, Add, Pow), Activation (ReLU, GELU)
- Code structure: op_api/op_host/op_kernel three-layer pattern
- Test framework: ops-math test suite with gcov coverage
- Performance target: within 5% of hand-tuned baseline
- Use vectorization (Vector API) where applicable
```

Hydra reads this automatically and includes it in every agent's system prompt.

## Development

```bash
# Build
cargo build -p hydra-daemon -p hydra-cli

# Run tests
cargo test -p hydra-daemon
cargo test -p hydra-core --test contract_connectivity
```

Full development guide: [docs/development-workflow.md](docs/development-workflow.md)

## Community

- [Report issues](https://github.com/lggyx/Hydra/issues)
- [CANN operator review layer](https://gitcode.com/cann/cannbot-skills)
- [Architecture deep dive](docs/architecture/README.md)

## License

MIT — see [LICENSE](LICENSE)
