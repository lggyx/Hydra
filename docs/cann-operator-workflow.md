# CANN Operator Development & Testing Workflow

This document describes the end-to-end AI-agent-driven workflow for Ascend CANN operator development and testing using Hydra.

## Overview

CANN operator development follows a three-layer code structure: `op_api` (interface layer), `op_host` (host-side logic), and `op_kernel` (device-side implementation). Hydra automates this pattern using a multi-agent architecture with integrated review gates from [cannbot-skills](https://gitcode.com/cann/cannbot-skills).

## Workflow Stages

### Stage 1: Task Decomposition (Orchestrator)

```
User: "Implement Mul, Add, Pow operators with end-to-end tests"

OrchestratorAgent:
  1. Analyzes ops-math specification
  2. Identifies independent operator units
  3. For each operator, creates a spawn_execution task with detailed spec:
     - Operator name and type signature
     - op_api / op_host / op_kernel requirements
     - Test coverage targets (gcov)
     - Performance baseline reference
```

### Stage 2: Parallel Implementation (ExecutionAgents)

Multiple ExecutionAgents run concurrently, each handling one operator:

```
ExecutionAgent #1: Mul operator
  ├── Read ops-math spec
  ├── Implement op_api (type definitions, memory management)
  ├── Implement op_host (kernel launch, error handling)
  ├── Implement op_kernel (device-side computation, vectorization)
  ├── Build with CANN toolchain
  ├── Run test suite
  └── Report: pass/fail, coverage %, performance metrics

ExecutionAgent #2: Add operator  (runs in parallel with #1)
ExecutionAgent #3: Pow operator  (runs in parallel with #1, #2)
```

### Stage 3: Review Gate (cannbot-skills)

All operator implementations pass through the cannbot-skills review layer:

```
cannbot-skills Review Layer
  ├── Static Analysis (lint, type safety, memory bounds)
  ├── Correctness Check (compare against reference implementation)
  ├── Performance Benchmark (compare against baseline)
  │     └── Flag regressions > 5% deviation from baseline
  ├── Accuracy Verification
  │     └── Tolerance-aware diff against golden outputs
  └── Merge Gate
        ├── Pass → approve for merge
        └── Fail → return to ExecutionAgent with detailed feedback
```

### Stage 4: Iteration & Completion

```
OrchestratorAgent:
  1. Collects review results from cannbot-skills
  2. For failed operators:
     ├── Returns failures to ExecutionAgents with review feedback
     └── ExecutionAgents iterate: fix → rebuild → retest → re-review
  3. For passed operators: marks as complete
  4. When all pass → declare_complete with summary report
```

## Usage

### Starting an orchestrator

```bash
# In Hydra TUI
/agents create --kind orchestrator
/agents <id> start "根据 ops-math 规格实现 Mul、Add、Pow 算子的 op_api/op_host/op_kernel 三层代码并编写端到端测试"
```

### Monitoring progress

```bash
/agents                                    # List all agents and status
/agents <orchestrator_id>                  # Check orchestrator status
/agents <execution_id> events              # Check specific operator's events
```

### Interacting during development

```bash
# If orchestrator enters waiting_input (asking for clarification):
/agents <orchestrator_id> input "Mul 算子使用 float32 精度，性能目标在 910B 上小于 100us"

# Cancel a stuck operator
/agents <execution_id> cancel
```

## Performance & Accuracy Targets

| Metric | Target | Verification |
|--------|--------|-------------|
| Correctness | 100% test pass | ops-math test suite |
| Code coverage | ≥ 85% per operator | gcov |
| Performance | ≤ 5% deviation from hand-tuned baseline | Profiling comparison |
| Accuracy | ≤ 1e-6 tolerance vs reference | Golden output comparison |
| Review gate | All checks green | cannbot-skills pipeline |

## Integration with CANN Toolchain

Hydra agents work with the standard CANN development environment:

- **Build system**: CMake with CANN toolchain
- **Compiler**: Ascend clang (based on LLVM)
- **Profiling**: msprof for kernel-level profiling
- **Testing**: ops-math test framework with gcov coverage
- **Target**: Ascend 910B, CANN 8.0.RC1+

## Review Layer

The [cannbot-skills](https://gitcode.com/cann/cannbot-skills) project provides the review layer that validates all agent-produced operator code. It is integrated as an external quality gate:

- Operators are submitted to cannbot-skills for automated review
- Review results (pass/fail/detailed feedback) are returned to the orchestrator
- Failed reviews trigger automatic rework cycles
- The orchestrator tracks review status for each operator

## Related Documents

- [CANN operator workflow (中文)](cann-operator-workflow.zh.md)
- [Architecture overview](architecture/overview.md)
- [Current implementation contract](architecture/current-implementation-contract.md)
- [cannbot-skills repository](https://gitcode.com/cann/cannbot-skills)
- [ops-math test documentation](test.md)
