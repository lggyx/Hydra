# Hydra 架构图

所有图表使用 Mermaid 语法。在 https://mermaid.live 或 VS Code Mermaid 插件中渲染。

---

## 1. 系统拓扑

```mermaid
flowchart TB
    subgraph "用户界面层"
        CLI["hydra CLI"]
        TUI["TUI 监控"]
        VSC["VSCode 扩展"]
        WEB["Web 看板"]
    end

    subgraph "API 层"
        REST["REST 服务<br/>axum :7890"]
        WS["WebSocket<br/>事件流"]
    end

    subgraph "核心引擎"
        RM["ResourceManager"]
        REG["AgentRegistry"]
        BUS["EventBus"]
        PROV["ProviderRegistry"]
        TOOLS["ToolRegistry"]
    end

    subgraph "Agent 层"
        E1["ExecutionAgent #1<br/>(Service.java)"]
        E2["ExecutionAgent #2<br/>(Controller.java)"]
        O1["OrchestratorAgent"]
        R1["ReviewerAgent"]
    end

    subgraph "工作区层"
        ISO["HydraWorkspaceManager<br/>(ISO-Framework)"]
        WT1[".worktrees/wt-1/"]
        WT2[".worktrees/wt-2/"]
        WTR[".worktrees/wt-r/"]
    end

    subgraph "Git 层"
        REPO["Repository<br/>(main)"]
        B1["agent/1"]
        B2["agent/2"]
        BR["agent/r"]
    end

    CLI --> REST
    TUI --> REST
    VSC --> REST
    WEB --> WS

    REST --> RM
    WS --> BUS

    RM --> REG
    RM --> BUS
    RM --> PROV
    RM --> TOOLS
    RM --> ISO

    BUS --> E1
    BUS --> E2
    BUS --> O1
    BUS --> R1
    BUS --> CLI
    BUS --> TUI
    BUS --> VSC
    BUS --> WEB

    REG --> E1
    REG --> E2
    REG --> O1
    REG --> R1

    E1 --> PROV
    E2 --> PROV
    O1 --> PROV
    R1 --> PROV

    E1 --> TOOLS
    E2 --> TOOLS
    R1 --> TOOLS

    E1 --> ISO
    E2 --> ISO
    R1 --> ISO
    O1 -.-> ISO

    ISO --> WT1
    ISO --> WT2
    ISO --> WTR

    WT1 --> B1
    WT2 --> B2
    WTR --> BR

    B1 --> REPO
    B2 --> REPO
    BR --> REPO

    classDef core fill:#2563eb,stroke:#1d4ed8,color:#fff
    classDef agent fill:#059669,stroke:#047857,color:#fff
    classDef workspace fill:#d97706,stroke:#b45309,color:#fff
    classDef git fill:#7c3aed,stroke:#6d28d9,color:#fff
    classDef api fill:#dc2626,stroke:#b91c1c,color:#fff
    classDef ui fill:#4b5563,stroke:#374151,color:#fff

    class RM,REG,BUS,PROV,TOOLS core
    class E1,E2,O1,R1 agent
    class ISO,WT1,WT2,WTR workspace
    class REPO,B1,B2,BR git
    class REST,WS api
    class CLI,TUI,VSC,WEB ui
```

---

## 2. Agent 生命周期

```mermaid
stateDiagram-v2
    [*] --> Created
    Created --> Running

    Running --> Running
    Running --> Paused
    Paused --> Running

    Running --> Completed
    Running --> Killed
    Running --> Failed

    Completed --> [*]
    Killed --> [*]
    Failed --> [*]
```

**状态转换说明：**

| 从 | 到 | 触发条件 |
|----|----|---------|
| 开始 | Created | `ResourceManager::spawn()` |
| Created | Running | `agent.run()` 启动 |
| Running | Running | LLM 轮次 / 工具执行 |
| Running | Paused | `AgentCommand::Pause` |
| Paused | Running | `AgentCommand::Resume` |
| Running | Completed | 自然完成 |
| Running | Killed | `AgentCommand::Kill` |
| Running | Failed | 不可恢复错误 |
| Completed | 结束 | cleanup（GC worktree） |
| Killed | 结束 | cleanup（根据需要保留 worktree） |
| Failed | 结束 | cleanup（保留 worktree 供调试） |

---

## 3. 事件流

```mermaid
sequenceDiagram
    participant E as ExecutionAgent
    participant BUS as EventBus
    participant O as OrchestratorAgent
    participant RM as ResourceManager
    participant UI as UI 订阅者

    Note over E: edit_file Service.java
    E->>BUS: AgentEvent::ToolCall
    BUS->>O: 扇出
    BUS->>UI: 扇出

    Note over O: 评估质量
    O->>RM: send_command(Kill agent #2)
    RM->>E: AgentCommand::Kill
    E->>BUS: AgentEvent::Killed

    Note over E: 完成工作
    E->>BUS: AgentEvent::Completed
    BUS->>O: 扇出
    BUS->>UI: 扇出
```

---

## 4. 部署拓扑

```mermaid
flowchart LR
    subgraph "进程: hydra serve"
        RM["ResourceManager"]
        O1["OrchestratorAgent"]
        DAEMON["StateServer<br/>(REST + WS)"]
    end

    subgraph "进程: Agent #1"
        E1["ExecutionAgent #1"]
        T1["ToolContext<br/>(cwd: .worktrees/wt-1)"]
    end

    subgraph "进程: Agent #2"
        E2["ExecutionAgent #2"]
        T2["ToolContext<br/>(cwd: .worktrees/wt-2)"]
    end

    subgraph "外部"
        VSC1["VSCode #1"]
        VSC2["VSCode #2"]
        LLM1["Claude API"]
        LLM2["GPT-4o API"]
    end

    RM -->|spawn| E1
    RM -->|spawn| E2
    O1 -->|inspect/kill/spawn| RM

    E1 -->|LLM| LLM1
    E2 -->|LLM| LLM2

    E1 -->|file ops| T1
    E2 -->|file ops| T2

    T1 -->|git| ISO1[".worktrees/wt-1/"]
    T2 -->|git| ISO2[".worktrees/wt-2/"]

    DAEMON -->|read| RM

    VSC1 -->|open| ISO1
    VSC2 -->|open| ISO2

    DAEMON -->|REST/WS| EXT["VSCode 扩展<br/>看板<br/>CLI"]
```

---

## 5. Crate 依赖图

```mermaid
flowchart TB
    subgraph "入口"
        CLI["hydra-cli"]
        DAEMON["hydra-daemon"]
        EXT["extensions/vscode"]
        DASH["dashboard"]
    end

    subgraph "核心"
        CORE["hydra-core"]
    end

    subgraph "基础设施"
        WS["hydra-workspace"]
        TEL["hydra-telemetry"]
    end

    CLI --> CORE
    CLI --> DAEMON
    DAEMON --> CORE
    EXT --> DAEMON
    DASH --> DAEMON

    CORE --> WS
    CORE --> TEL

    subgraph "外部依赖"
        ISO["iso-code crate"]
        ATO["atomcode-core"]
    end

    WS --> ISO
    CORE --> ATO
```

---

## 6. Agent 通信协议

```mermaid
flowchart LR
    subgraph "Agent A（发送方）"
        A_TX["event_tx"]
    end

    subgraph "ResourceManager（路由器）"
        BUS["EventBus"]
        REG["AgentRegistry"]
    end

    subgraph "Agent B"
        B_RX["event_rx"]
        B_CTRL["control_rx"]
    end

    subgraph "Agent C"
        C_RX["event_rx"]
        C_CTRL["control_rx"]
    end

    subgraph "UI 订阅者"
        UI_RX["event_rx"]
    end

    A_TX -->|AgentEvent| BUS
    BUS --> B_RX
    BUS --> C_RX
    BUS --> UI_RX

    REG -.->|AgentCommand| B_CTRL
    REG -.->|AgentCommand| C_CTRL

    style BUS fill:#2563eb,stroke:#1d4ed8,color:#fff
    style REG fill:#7c3aed,stroke:#6d28d9,color:#fff
```

---

## 7. 工作区隔离模型

### 目录布局

```
repo-root/
├── .git/
├── src/
├── Cargo.toml
└── .worktrees/
    ├── wt-1/
    │   ├── .git                    (链接到主 .git)
    │   ├── src/
    │   ├── Cargo.toml
    │   └── .hydra-agent            (元数据: id, branch)
    ├── wt-2/
    └── wt-reviewer-1/
```

### Git 分支结构

```
main ──────────────────────────────────────── (受保护)
  │
  ├── agent/1 → .worktrees/wt-1/   ExecutionAgent #1
  │     └── "feat: 添加分页"
  │
  ├── agent/2 → .worktrees/wt-2/   ExecutionAgent #2
  │     └── "feat: 添加 Service 调用"
  │
  └── agent/r → .worktrees/wt-r/   ReviewerAgent #1
        └── (只读，无提交)
```

### Merge 流程

```
agent/1 ──┐
           ├──→ staging/service-paginated ──→ main（已合并）
agent/2 ──┘

冲突情况:
agent/1 ──→ staging/a ──┐
                        ├──→ main（人工解决）
agent/2 ──→ staging/b ──┘
```

---

## 8. 分层通信模型

```mermaid
flowchart TB
    subgraph "L4: 用户交互"
        L4["CLI / TUI / VSCode / Web"]
    end

    subgraph "L3: API 接口"
        L3["REST API + WebSocket"]
    end

    subgraph "L2: 编排层"
        L2["OrchestratorAgent"]
    end

    subgraph "L1: Agent 运行时"
        L1["ResourceManager"]
    end

    subgraph "L0: 基础设施"
        L0["ISO-Framework<br/>+ LlmProvider"]
    end

    L4 <-->|HTTP/WS| L3
    L3 <-->|AgentSnapshot<br/>AgentEvent| L2
    L2 <-->|AgentCommand<br/>AgentEvent| L1
    L1 <-->|worktree 操作<br/>LLM 调用| L0
```

---

## 图表约定

| 颜色 | 含义 |
|-------|---------|
| 蓝色 | 核心引擎（ResourceManager、AgentRegistry、EventBus） |
| 绿色 | Agent（Execution、Orchestrator、Reviewer） |
| 橙色 | 工作区（ISO-Framework、worktree） |
| 紫色 | Git 基础设施（分支、仓库） |
| 红色 | API 层（REST、WebSocket） |
| 灰色 | 用户界面（CLI、TUI、VSCode） |

## 渲染方式

- **Mermaid Live Editor**: https://mermaid.live —— 粘贴任意代码块
- **VS Code**: 安装 "Mermaid Preview" 扩展，打开本文件
- **CLI**: `npm install -g @mermaid-js/mermaid-cli` 然后 `mmdc -i diagrams.zh.md -o architecture.svg`
- **GitHub**: markdown 文件原生支持
