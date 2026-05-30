# Hydra Fork 分析与推荐

**日期**: 2026-05-30  
**目的**: 识别需要 fork 的 GitHub 项目、分析适配度、定义集成方案。

---

## 1. 候选项目

### 1.1 ISO-Framework（snehith01001110/ISO-Framework）⭐13

**仓库**: https://github.com/snehith01001110/ISO-Framework  
**语言**: Rust  
**协议**: Apache-2.0 / MIT  

**做什么**: 为 AI 编码 Agent 提供安全的 git worktree 生命周期管理。

**为什么相关**: 精确解决了 Hydra 最底层的问题 —— 管理多个并发 AI Agent 的隔离 git worktree。README 明确点名 Claude Code、Cursor 和 OpenCode 存在已记录的 worktree bug。

**核心功能**:
- `Manager::create(branch, path, options)` — 带安全检查创建 worktree
- `Manager::delete(handle, options)` — 删除前执行 5 步 unmerged commit 检查
- `Manager::gc(options)` — 垃圾回收孤立 worktree
- `Manager::list()` — 枚举活跃 worktree
- Crash-safe 状态持久化（tmp + fsync + rename 的 atomic write）
- MCP 服务器集成（兼容 Claude Code、Cursor、Copilot、OpenCode）
- Claude Code hook 集成

**适配度评分**: 9/10

| Hydra 需求 | ISO-Framework 提供 | 差距 |
|-----------|-------------------|------|
| 为每个 Agent 创建 worktree | ✅ Manager::create | 无 — 直接映射 |
| Agent 清理时删除 worktree | ✅ Manager::delete（含 5 步安全检查） | 无 |
| GC 孤立 worktree | ✅ Manager::gc | 无 |
| 列出活跃 worktree | ✅ Manager::list | 无 |
| Crash-safe 状态 | ✅ atomic write | 无 |
| 跨平台（Windows/WSL） | 声称支持但未验证 | 需要在 WSL 上测试 |
| 接入 Claude Code | ✅ 内置 | Hydra core 不需要 |

**集成方案**:
```
1. Fork ISO-Framework 到 hydra/hydra-workspace
2. 薄适配层：Hydra 类型 → ISO-Framework 类型
3. 用 ISO-Framework API 替换内部 WorkspaceManager 调用
4. 保留 ISO-Framework 的安全保证（不写自定义 worktree 逻辑）
```

**Fork commit**: ISO-Framework API 稳定后锁定到特定 SHA。  
**维护**: 追踪上游 bug 修复；定期合并。

---

### 1.2 forge（automagik-dev/forge）

**仓库**: https://github.com/automagik-dev/forge  
**语言**: TypeScript（Tauri）+ Python  
**协议**: 未知（需检查仓库）  

**做什么**: 带 MCP 集成的多 Agent kanban 平台。通过 SSH/Tailscale 跨机器编排 AI 编码 Agent。

**为什么相关**: 有相同的用户工作流（在 worktree 中隔离执行、kanban 跟踪、实时监控）。UX 模式的好参考。

**适配度评分**: 4/10

| Hydra 需求 | forge 提供 | 差距 |
|-----------|-----------|------|
| Git worktree 隔离 | ✅ | ISO-Framework 已覆盖 |
| 多 Agent 并行执行 | ✅ | 架构不同（人工 kanban vs LLM 编排） |
| 实时监控 | ✅ Web UI | Hydra 有 TUI + REST |
| 跨机器编排 | ✅ SSH/Tailscale | 非 Hydra 优先级（local-first） |
| LLM 驱动调度 | ❌ 人工驱动 kanban | Hydra 核心功能 |
| 纯 Rust 后端 | ❌ Node.js + Python | 不匹配 |

**结论**: 不 fork。仅作为 UX 参考。forge 的 kanban 模型是"人工规划、AI 执行"——Hydra 的模型是"AI 规划、人工批准"。

---

### 1.3 AutoAgents（liquidos-ai/AutoAgents）⭐658

**仓库**: https://github.com/liquidos-ai/AutoAgents  
**语言**: Rust  
**协议**: MIT / Apache-2.0  

**做什么**: 通用多 Agent 框架，具备 typed pub/sub、环境管理、10+ LLM 提供方。

**为什么相关**: 有成熟的 Agent trait、typed pub/sub 通信和环境管理。Rust 多 Agent 项目中 star 数最高。

**适配度评分**: 5/10

| Hydra 需求 | AutoAgents 提供 | 差距 |
|-----------|----------------|------|
| Agent trait | ✅ 有 Agent trait | 设计目标不同 —— AutoAgents 是通用框架，Hydra 是代码专用 |
| Typed pub/sub | ✅ | Hydra 使用 event bus + channels（单进程场景更简单） |
| 10+ LLM 提供方 | ✅ OpenAI、Anthropic、DeepSeek 等 | Hydra 初期只需要 3-4 个提供方 |
| WASM sandboxed tools | ✅ | Hydra 使用原生 Rust tools（代码编辑更强大） |
| Memory backends | ✅ | 非 Hydra 优先级（Hydra 用 git 做 memory） |
| Sliding window context | ✅ | Hydra 用 atomcode 的 hot/cold 上下文策略 |

**结论**: 不 fork。AutoAgents 解决的是不同问题（通用 Agent 框架）。Hydra 的价值在代码生成专用层（git worktree 隔离 + LLM 驱动调度 + tool scoping）。AutoAgents 的 Agent trait 可以影响 Hydra 的设计，但直接集成会增加复杂度而没有对等的价值。

---

### 1.4 swarms-rs（The-Swarm-Corporation/swarms-rs）⭐163

**仓库**: https://github.com/The-Swarm-Corporation/swarms-rs  
**语言**: Rust  

**评估**: README 几乎是空的。不读源码无法评估架构。在更多信息可用之前优先级低。

---

## 2. 推荐总结

| 项目 | 操作 | 理由 |
|------|------|------|
| **ISO-Framework** | **FORK** | 用久经考验的代码解决 Hydra 最困难的问题（worktree 安全） |
| forge | 仅参考 | 架构不同；Node.js 技术栈 |
| AutoAgents | 仅参考 | 通用框架；Hydra 的 Agent 模型是领域专用的 |
| swarms-rs | 监控 | 信息不足，无法评估 |

---

## 3. Fork 集成方案

### 3.1 Phase 0：Fork 与适配层（第 1 周）

**步骤 1：Fork**

```bash
# 将 snehith01001110/ISO-Framework fork 到 hydra-ai/ISO-Framework
# 然后作为 submodule 或直接依赖加入
cd /path/to/hydra
git submodule add https://github.com/hydra-ai/ISO-Framework.git crates/hydra-workspace/upstream
```

或用 Cargo patch 指令：

```toml
# hydra/Cargo.toml (workspace)
[patch."https://github.com/hydra-ai/ISO-Framework"]
iso-code = { path = "crates/hydra-workspace/upstream" }
```

**步骤 2：创建适配层**

```
crates/hydra-workspace/
├── Cargo.toml          # 依赖 iso-code + hydra-core
├── src/
│   ├── lib.rs
│   ├── manager.rs      # HydraWorkspaceManager 封装 iso_code::Manager
│   ├── adapter.rs      # Hydra AgentId/branch ↔ ISO-Framework Handle
│   └── safety.rs       # Hydra 专属安全策略
│   └── types.rs        # Hydra worktree 类型（镜像 ISO-Framework 类型）
└── tests/
```

**步骤 3：适配层代码骨架**

```rust
// crates/hydra-workspace/src/manager.rs

use iso_code::{Manager as IsoManager, Config as IsoConfig, CreateOptions, DeleteOptions};
use hydra_core::{AgentId, AgentState, AgentOutcome, AgentMetrics};

pub struct HydraWorkspaceManager {
    inner: IsoManager,
    base_path: PathBuf,
}

impl HydraWorkspaceManager {
    pub fn new(repo_path: PathBuf) -> anyhow::Result<Self> {
        let iso_config = IsoConfig::default();
        let inner = IsoManager::new(&repo_path, iso_config)?;
        Ok(Self { inner, base_path: repo_path })
    }

    /// 为 Agent 创建 worktree。返回绝对路径。
    pub fn create_for_agent(
        &self,
        agent_id: AgentId,
        base_branch: &str,
    ) -> anyhow::Result<PathBuf> {
        let branch_name = format!("hydra/agent/{}", agent_id.0);
        let wt_name = format!("wt-{}", agent_id.0);
        let wt_path = self.base_path.join(".worktrees").join(&wt_name);

        let (handle, info) = self.inner.create(
            &branch_name,
            &wt_path,
            CreateOptions::default(),
        )?;

        // 存储 handle 映射供后续清理
        // info.path == wt_path, info.branch == branch_name

        Ok(wt_path)
    }

    /// Agent 完成后删除 worktree。遵循 ISO-Framework 安全保证。
    pub fn cleanup_agent(&self, agent_id: AgentId, outcome: &AgentOutcome) -> anyhow::Result<()> {
        // 如果 Agent 成功且用户想保留分支，跳过删除
        // 如果 Agent 失败，保留 worktree 供调试
        // 如果用户显式删除，执行完整清理

        let wt_path = self.base_path.join(".worktrees").join(format!("wt-{}", agent_id.0));
        let branch_name = format!("hydra/agent/{}", agent_id.0);

        // 删除前检查分支是否已合并
        let handle = self.inner.find_handle(&wt_path)?;
        self.inner.delete(&handle, DeleteOptions::default())?;

        Ok(())
    }

    /// GC 孤立 worktree（定期或启动时运行）
    pub fn gc(&self) -> anyhow::Result<iso_code::GcReport> {
        self.inner.gc(Default::default())
    }
}
```

### 3.2 Phase 1：Core 集成（第 2-3 周）

将 ResourceManager 中的占位 `WorkspaceManager` 替换为 `HydraWorkspaceManager`。

```rust
// crates/hydra-core/src/resource.rs

use hydra_workspace::HydraWorkspaceManager;

pub struct ResourceManager {
    agents:          Arc<RwLock<AgentRegistry>>,
    event_rx:        mpsc::UnboundedReceiver<AgentEvent>,
    event_bus:       mpsc::UnboundedSender<AgentEvent>,   // cloned from event_rx half
    subscribers:     Arc<RwLock<Vec<mpsc::UnboundedSender<AgentEvent>>>>,
    control_senders: Arc<RwLock<HashMap<AgentId, mpsc::UnboundedSender<AgentCommand>>>>,
    next_id:         AtomicU64,
    git:             Arc<dyn GitWorktreeManager>,          // trait object, mockable
    providers:       Arc<RwLock<ProviderRegistry>>,
    tool_registry:   Arc<RwLock<ToolRegistry>>,
}

impl ResourceManager {
    pub fn spawn(&self, kind: AgentKind, spec: AgentSpec) -> Result<AgentHandle> {
        let id = AgentId(self.next_id.fetch_add(1, Ordering::SeqCst));

        match kind {
            AgentKind::Execution => {
                let branch = format!("hydra/agent/{}", id.0);
                let worktree = self.git.create_for_agent(id, "main")?;

                let agent = ExecutionAgent::new(id, branch, worktree, spec, self.clone())?;
                self.agents.write().unwrap().register(id, agent);
            }
            AgentKind::Orchestrator => {
                let agent = OrchestratorAgent::new(id, self.clone())?;
                self.agents.write().unwrap().register(id, agent);
            }
            AgentKind::Reviewer => {
                let branch = spec.branch.clone().unwrap();
                let worktree = self.git.create_worktree_for_branch(&branch)?;
                let agent = ReviewerAgent::new(id, branch, worktree, spec, self.clone())?;
                self.agents.write().unwrap().register(id, agent);
            }
        }

        // 启动 Agent 的 run() 循环
        let handle = self.agents.read().unwrap().get(id).unwrap().clone_box();
        let resources = self.clone();
        let event_tx = self.event_bus.clone();
        tokio::spawn(async move {
            let mut agent = handle;
            let outcome = agent.run(&resources).await;
            event_tx.send(AgentEvent::Completed { agent_id: id, outcome }).ok();
        });

        Ok(AgentHandle { id, state: /* ... */ })
    }
}
```

### 3.3 Phase 2+：上游追踪

```bash
# 在 crates/hydra-workspace/ 中
git remote add upstream https://github.com/snehith01001110/ISO-Framework.git
git fetch upstream

# 每月合并上游修复
git merge upstream/main
# 在适配层解决冲突
```

---

## 4. ISO-Framework 代码入口点

Fork 前需要理解的关键文件：

| 文件 | 用途 | Hydra 关联 |
|------|------|-----------|
| `src/manager.rs` | 核心 Manager（create/delete/gc/list） | 主要 API 接口 |
| `src/state.rs` | Crash-safe 状态持久化 | 理解 atomic write 模式 |
| `src/safety.rs` | 安全检查（unmerged、nested、locked） | 信任这些保证 |
| `src/hooks.rs` | Claude Code / Cursor hook 集成 | CLI 集成参考 |
| `src/mcp.rs` | MCP 服务器实现 | hydra-daemon 参考 |
| `tests/integration_test.rs` | 集成测试 | 需要保留的测试用例 |

---

## 5. Fork ISO-Framework 的特有风险

| 风险 | 可能性 | 影响 | 缓解 |
|------|--------|------|------|
| ISO-Framework 停止维护 | 中 | 中 | Fork 是永久的；我们拥有代码 |
| Rust 版本间 API 破坏 | 低 | 低 | 在 Cargo.toml 中锁定 Rust 版本 |
| ISO-Framework 有隐藏 bug | 低 | 高 | 运行它们的测试套件 + Hydra 自己的测试 |
| 协议不兼容 | 极低 | 高 | 两者都是 Apache-2.0 和 MIT —— 与 Hydra 兼容 |

---

## 6. Fork 的维护模型

由于 Hydra 直接将 ISO-Framework fork 到 `crates/hydra-workspace/`，后续维护遵循以下模式：

| 活动 | 频率 | 负责人 |
|----------|---------|-------|
| 同步上游 bug 修复 | 每月 | 核心维护者 |
| 审阅上游 PR | 每周 | 核心维护者 |
| 更新适配层 | 按需（当 ISO-Framework API 变更时） | 核心维护者 |
| Hydra 专属安全策略 | 持续 | Hydra 团队 |

```bash
# 每月同步流程
cd crates/hydra-workspace/
git fetch upstream
git log upstream/main..HEAD --oneline  # 审阅我们的本地补丁
git merge upstream/main                 # 合并上游修复
cargo test                              # 验证没有破坏
```

**理由**: ISO-Framework 规模小（估计约 2000 行），专为 AI Agent worktree 安全设计，且积极维护。Fork 既给予了对安全策略的完全控制权，又保留了拉取上游 bug 修复的能力。Vendoring 会 forfeit upstream improvements 并在后期增加合并摩擦。

**许可证说明**: ISO-Framework 采用 MIT/Apache-2.0 双许可证 —— 与 Hydra 的许可证兼容。Fork 没有法律障碍。
