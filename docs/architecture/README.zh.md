# Hydra 架构文档

本文档包含 Hydra 的完整架构设计 —— 一个面向昇腾 CANN 算子开发测试的 Rust 多智能体系统，采用 LLM 驱动编排 + 集成审查门禁。

## 文档列表

| 文件 | 内容 |
|------|------|
| [overview.zh.md](./overview.zh.md) | 完整架构设计：设计原则（含 CANN 领域）、核心抽象、事件流、crate 结构 |
| [current-implementation-contract.md](./current-implementation-contract.md) | 当前已实现架构 vs 目标架构对比 |
| [diagrams.zh.md](./diagrams.zh.md) | Mermaid 架构图：系统拓扑、Agent 生命周期、CANN 算子开发工作流 |
| [fork-analysis.zh.md](./fork-analysis.zh.md) | 开源项目分析及 fork 集成方案 |
| [../cann-operator-workflow.zh.md](../cann-operator-workflow.zh.md) | CANN 算子开发测试端到端工作流 + cannbot-skills 审查层 |
| [README.zh.md](./README.zh.md) | 本文件 —— 架构文档入口 |

## 快速导航

### 面向架构师
1. 从 [overview.zh.md § 设计原则](./overview.zh.md#1-设计原则) 开始
2. 阅读 [overview.zh.md § 核心抽象](./overview.zh.md#3-核心抽象) 了解 Agent trait 和类型系统
3. 查看 [diagrams.zh.md § 系统拓扑](./diagrams.zh.md#1-系统拓扑) 理解整体架构

### 面向开发者
1. 阅读 [fork-analysis.zh.md § 推荐结论](./fork-analysis.zh.md#2-推荐总结) —— 需要 fork ISO-Framework
2. 按照 [overview.zh.md § 实施顺序](./overview.zh.md#12-实施顺序) 分阶段推进
3. 连接模块时参考 [overview.zh.md § 接口契约](./overview.zh.md#9-接口契约详解)

### 面向集成者（VSCode、Dashboard、CLI）
1. 阅读 [overview.zh.md § 接口契约 § StateServer ↔ 客户端](./overview.zh.md#94-stateserver--客户端) 了解 REST/WS API
2. 查看 [diagrams.zh.md § 分层通信模型](./diagrams.zh.md#8-分层通信模型) 理解 5 层架构

## 关键决策

| 决策 | 理由 | 文档 |
|------|------|------|
| 所有 Agent 共享同一个 `Agent` trait | 消除"Orchestrator 是特殊的"反模式；支持递归多级编排 | [overview.zh.md § 3.1](./overview.zh.md#31-agent-trait-统一接口) |
| Fork ISO-Framework 做 worktree 管理 | 经过实战检验的安全保证（5 步 unmerged 检查、GC、crash-safe 状态）；解决最困难的问题 | [fork-analysis.zh.md § 1.1](./fork-analysis.zh.md#11-iso-framework-snehith01001110iso-framework-13) |
| 单向事件流（仅 broadcast） | 没有 Agent 持有另一个 Agent 的 receiver；ResourceManager 是唯一路由器 | [overview.zh.md § 6](./overview.zh.md#6-事件流) |
| Orchestrator 用 LLM 做决策（带规则兜底） | 自适应策略优于硬编码规则；规则仅作为安全下限 | [overview.zh.md § 5.2](./overview.zh.md#52-orchestratoragent-调度者) |
| 默认工作区隔离 | 每个 ExecutionAgent 独占 worktree；Agent 之间零共享可变状态 | [overview.zh.md § 7](./overview.zh.md#7-git-worktree-隔离) |

## Crate 结构

```
hydra/
├── crates/
│   ├── hydra-core/          # Agent trait、ResourceManager、所有 Agent 类型
│   ├── hydra-daemon/        # REST + WebSocket 服务
│   ├── hydra-cli/           # CLI 入口
│   ├── hydra-telemetry/     # 日志、追踪、datalog
│   └── hydra-workspace/     # ISO-Framework fork（worktree 安全）
├── extensions/vscode/       # VSCode 扩展
└── dashboard/               # Web 看板
```

完整 crate 布局和依赖图见 [overview.zh.md § 8](./overview.zh.md#8-crate-结构)。

## 实施阶段

| 阶段 | 周期 | 交付物 |
|------|------|--------|
| Phase 0 | 1 周 | Fork ISO-Framework + Agent trait + ResourceManager 骨架 |
| Phase 1 | 1 周 | ExecutionAgent（实现 Agent trait + tool scoping） |
| Phase 2 | 1 周 | OrchestratorAgent（LLM 驱动调度器） |
| Phase 3 | 1 周 | 可视化：StateServer + CLI + VSCode 扩展 |
| Phase 4 | 1 周 | 优化：QualityGate trait、多模型 fallback、遥测 |

## 外部参考

| 项目 | 地址 | 关联 |
|------|------|------|
| ISO-Framework | https://github.com/snehith01001110/ISO-Framework | **Fork 目标** —— worktree 安全 |
| hydra | /mnt/c/Users/15853/Workspace/Hydra/hydra | **上游依赖** —— Provider、Tool、TurnRunner |
| AutoAgents | https://github.com/liquidos-ai/AutoAgents | 参考 —— typed pub/sub、agent trait |
| forge | https://github.com/automagik-dev/forge | 参考 —— UX 模式（不 fork） |
