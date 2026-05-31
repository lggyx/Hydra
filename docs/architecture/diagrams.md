# Hydra Architecture Diagrams

All diagrams use Mermaid syntax. Render at https://mermaid.live or with VS Code Mermaid extension.

---

## 1. System Topology

```mermaid
flowchart TB
    subgraph "User Interface Layer"
        CLI["hydra CLI"]
        TUI["TUI Monitor"]
        VSC["VSCode Extension"]
        WEB["Web Dashboard"]
    end

    subgraph "API Layer"
        REST["REST Server<br/>axum :7890"]
        WS["WebSocket<br/>Event Stream"]
    end

    subgraph "Core Engine"
        RM["ResourceManager"]
        REG["AgentRegistry"]
        BUS["EventBus"]
        PROV["ProviderRegistry"]
        TOOLS["ToolRegistry"]
    end

    subgraph "Agent Layer"
        E1["ExecutionAgent #1<br/>(Service.java)"]
        E2["ExecutionAgent #2<br/>(Controller.java)"]
        O1["OrchestratorAgent"]
        R1["ReviewerAgent"]
    end

    subgraph "Workspace Layer"
        ISO["HydraWorkspaceManager<br/>(ISO-Framework)"]
        WT1[".worktrees/wt-1/"]
        WT2[".worktrees/wt-2/"]
        WTR[".worktrees/wt-r/"]
    end

    subgraph "Git Layer"
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

## 2. Agent Lifecycle

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

**State transitions:**

| From | To | Trigger |
|------|----|---------|
| start | Created | `ResourceManager::spawn()` |
| Created | Running | `agent.run()` starts |
| Running | Running | LLM turn / tool execution |
| Running | Paused | `AgentCommand::Pause` |
| Paused | Running | `AgentCommand::Resume` |
| Running | Completed | natural finish |
| Running | Killed | `AgentCommand::Kill` |
| Running | Failed | unrecoverable error |
| Completed | end | cleanup (GC worktree) |
| Killed | end | cleanup (retain worktree if needed) |
| Failed | end | cleanup (retain worktree for debug) |

---

## 3. Event Flow

```mermaid
sequenceDiagram
    participant E as ExecutionAgent
    participant RM as ResourceManager
    participant BUS as EventBus
    participant O as OrchestratorAgent
    participant UI as UI Subscribers

    Note over E: edit_file Service.java
    E->>RM: event_tx.send(ToolCall)
    RM->>BUS: event_rx → fan-out to subscribers
    BUS->>O: AgentEvent::ToolCall
    BUS->>UI: AgentEvent::ToolCall

    Note over O: evaluates quality
    O->>RM: control_tx.send(Kill agent #2)
    RM->>RM: lookup control_senders[agent_2]
    RM->>E: AgentCommand::Kill (via control sender)

    E->>RM: event_tx.send(Killed)
    RM->>BUS: fan-out
    BUS->>O: AgentEvent::Killed
    BUS->>UI: AgentEvent::Killed

    Note over E: completes work
    E->>RM: event_tx.send(Completed)
    RM->>BUS: fan-out
    BUS->>O: AgentEvent::Completed
    BUS->>UI: AgentEvent::Completed
```

---

## 4. Deployment Topology

```mermaid
flowchart LR
    subgraph "Process: hydra serve"
        RM["ResourceManager"]
        O1["OrchestratorAgent"]
        DAEMON["StateServer<br/>(REST + WS)"]
    end

    subgraph "Process: Agent #1"
        E1["ExecutionAgent #1"]
        T1["ToolContext<br/>(cwd: .worktrees/wt-1)"]
    end

    subgraph "Process: Agent #2"
        E2["ExecutionAgent #2"]
        T2["ToolContext<br/>(cwd: .worktrees/wt-2)"]
    end

    subgraph "External"
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

    DAEMON -->|REST/WS| EXT["VSCode Ext<br/>Dashboard<br/>CLI"]
```

---

## 5. Crate Dependency Graph

```mermaid
flowchart TB
    subgraph "Entry Points"
        CLI["hydra-cli"]
        DAEMON["hydra-daemon"]
        EXT["extensions/vscode"]
        DASH["dashboard"]
    end

    subgraph "Core"
        CORE["hydra-core"]
    end

    subgraph "Infrastructure"
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

    subgraph "External"
        ISO["iso-code crate"]
        ATO["hydra-core"]
    end

    WS --> ISO
    CORE --> ATO
```

---

## 6. Agent Communication Protocol

```mermaid
flowchart LR
    subgraph "Agent A"
        A_TX["event_tx"]
    end

    subgraph "ResourceManager"
        RM_RX["event_rx"]
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

    subgraph "UI Subscriber"
        UI_RX["event_rx"]
    end

    A_TX -->|AgentEvent| RM_RX
    RM_RX --> BUS
    BUS --> B_RX
    BUS --> C_RX
    BUS --> UI_RX

    REG -.->|AgentCommand| B_CTRL
    REG -.->|AgentCommand| C_CTRL

    style BUS fill:#2563eb,stroke:#1d4ed8,color:#fff
    style REG fill:#7c3aed,stroke:#6d28d9,color:#fff
```

---

## 7. Workspace Isolation Model

### Directory Layout

```
repo-root/
├── .git/
├── src/
├── Cargo.toml
└── .worktrees/
    ├── wt-1/
    │   ├── .git                    (link to main .git)
    │   ├── src/
    │   ├── Cargo.toml
    │   └── .hydra-agent            (metadata: id, branch)
    ├── wt-2/
    └── wt-reviewer-1/
```

### Git Branch Structure

```
main ──────────────────────────────────────── (protected)
  │
  ├── agent/1 → .worktrees/wt-1/   ExecutionAgent #1
  │     └── "feat: add pagination"
  │
  ├── agent/2 → .worktrees/wt-2/   ExecutionAgent #2
  │     └── "feat: add Service call"
  │
  └── agent/r → .worktrees/wt-r/   ReviewerAgent #1
        └── (read-only, no commits)
```

### Merge Flow

```
agent/1 ──┐
           ├──→ staging/service-paginated ──→ main (merged)
agent/2 ──┘

Conflicts:
agent/1 ──→ staging/a ──┐
                        ├──→ main (manual resolution)
agent/2 ──→ staging/b ──┘
```

---

## 8. Layered Communication Model

```mermaid
flowchart TB
    subgraph "L4: User Interaction"
        L4["CLI / TUI / VSCode / Web"]
    end

    subgraph "L3: API Surface"
        L3["REST API + WebSocket"]
    end

    subgraph "L2: Orchestration"
        L2["OrchestratorAgent"]
    end

    subgraph "L1: Agent Runtime"
        L1["ResourceManager"]
    end

    subgraph "L0: Infrastructure"
        L0["ISO-Framework<br/>+ LlmProvider"]
    end

    L4 <-->|HTTP/WS| L3
    L3 <-->|AgentSnapshot<br/>AgentEvent| L2
    L2 <-->|AgentCommand<br/>AgentEvent| L1
    L1 <-->|worktree ops<br/>LLM calls| L0
```

---

## Diagram Conventions

| Color | Meaning |
|-------|---------|
| Blue | Core engine (ResourceManager, AgentRegistry, EventBus) |
| Green | Agents (Execution, Orchestrator, Reviewer) |
| Orange | Workspace (ISO-Framework, worktrees) |
| Purple | Git infrastructure (branches, repo) |
| Red | API layer (REST, WebSocket) |
| Gray | User-facing interfaces (CLI, TUI, VSCode) |

## How to Render

- **Mermaid Live Editor**: https://mermaid.live — paste any code block
- **VS Code**: Install "Mermaid Preview" extension, open this file
- **CLI**: `npm install -g @mermaid-js/mermaid-cli` then `mmdc -i diagrams.md -o architecture.svg`
- **GitHub**: Native support in markdown files
