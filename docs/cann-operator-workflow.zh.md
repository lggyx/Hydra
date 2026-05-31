# CANN 算子开发测试工作流

本文档描述使用 Hydra 多智能体系统实现昇腾 CANN 算子开发测试的端到端 AI 驱动工作流。

## 概述

CANN 算子开发遵循三层代码结构：`op_api`（接口层）、`op_host`（主机侧逻辑）、`op_kernel`（设备侧实现）。Hydra 通过多智能体架构自动化这一模式，并集成 [cannbot-skills](https://gitcode.com/cann/cannbot-skills) 作为审查门禁。

## 工作流阶段

### 阶段一：任务分解（Orchestrator）

```
用户："实现 Mul、Add、Pow 算子并编写端到端测试"

OrchestratorAgent:
  1. 分析 ops-math 规格文档
  2. 识别独立的算子单元
  3. 为每个算子创建 spawn_execution 任务，附带详细规格：
     - 算子名称和类型签名
     - op_api / op_host / op_kernel 需求
     - 测试覆盖目标（gcov）
     - 性能基线参考
```

### 阶段二：并行实现（ExecutionAgents）

多个 ExecutionAgent 并行运行，每个处理一个算子：

```
ExecutionAgent #1: Mul 算子
  ├── 读取 ops-math 规格
  ├── 实现 op_api（类型定义、内存管理）
  ├── 实现 op_host（kernel 启动、错误处理）
  ├── 实现 op_kernel（设备侧计算、向量化）
  ├── 用 CANN 工具链编译
  ├── 运行测试套件
  └── 报告：通过/失败、覆盖率%、性能指标

ExecutionAgent #2: Add 算子（与 #1 并行）
ExecutionAgent #3: Pow 算子（与 #1、#2 并行）
```

### 阶段三：审查门禁（cannbot-skills）

所有算子实现都经过 cannbot-skills 审查层：

```
cannbot-skills 审查层
  ├── 静态分析（lint、类型安全、内存边界）
  ├── 正确性检查（与参考实现对比）
  ├── 性能基准对比（与基线对比）
  │     └── 标记偏差 > 5% 的回归
  ├── 精度验证
  │     └── 与 golden output 的容差感知对比
  └── 合入门禁
        ├── 通过 → 批准合并
        └── 失败 → 带详细反馈返回给 ExecutionAgent
```

### 阶段四：迭代与完成

```
OrchestratorAgent:
  1. 收集 cannbot-skills 的审查结果
  2. 对于失败的算子：
     ├── 带审查反馈返回给 ExecutionAgent
     └── ExecutionAgent 迭代：修复 → 重新编译 → 重新测试 → 重新审查
  3. 对于通过的算子：标记为完成
  4. 全部通过后 → declare_complete 输出汇总报告
```

## 使用方式

### 启动编排者

```bash
# 在 Hydra TUI 中
/agents create --kind orchestrator
/agents <id> start "根据 ops-math 规格实现 Mul、Add、Pow 算子的 op_api/op_host/op_kernel 三层代码并编写端到端测试"
```

### 监控进度

```bash
/agents                                    # 列出所有 agent 及状态
/agents <orchestrator_id>                  # 查看编排者状态
/agents <execution_id> events              # 查看特定算子的执行事件
```

### 开发过程中交互

```bash
# 当编排者进入 waiting_input（请求澄清时）：
/agents <orchestrator_id> input "Mul 算子使用 float32 精度，性能目标在 910B 上小于 100us"

# 取消卡住的算子
/agents <execution_id> cancel
```

## 性能与精度目标

| 指标 | 目标 | 验证方式 |
|------|------|---------|
| 正确性 | 100% 测试通过 | ops-math 测试套件 |
| 代码覆盖率 | 每个算子 ≥ 85% | gcov |
| 性能 | 与手写基线偏差 ≤ 5% | Profiling 对比 |
| 精度 | 与参考实现误差 ≤ 1e-6 | Golden output 对比 |
| 审查门禁 | 全部检查绿色 | cannbot-skills 流水线 |

## CANN 工具链集成

Hydra Agent 与标准 CANN 开发环境协作：

- **构建系统**：CMake + CANN toolchain
- **编译器**：Ascend clang（基于 LLVM）
- **性能分析**：msprof 内核级 profiling
- **测试**：ops-math 测试框架 + gcov 覆盖率
- **目标平台**：Ascend 910B, CANN 8.0.RC1+

## 审查层

[cannbot-skills](https://gitcode.com/cann/cannbot-skills) 项目提供审查层，验证所有 Agent 产出的算子代码：

- 算子代码提交到 cannbot-skills 进行自动审查
- 审查结果（通过/失败/详细反馈）返回给编排者
- 失败的审查触发自动返工循环
- 编排者跟踪每个算子的审查状态

## 相关文档

- [CANN operator workflow (英文)](cann-operator-workflow.md)
- [架构概览](architecture/overview.zh.md)
- [当前实现契约](architecture/current-implementation-contract.md)
- [cannbot-skills 仓库](https://gitcode.com/cann/cannbot-skills)
- [ops-math 测试文档](test.md)
