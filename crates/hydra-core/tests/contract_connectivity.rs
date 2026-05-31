use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use futures::stream;
use futures::Stream;
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use hydra_core::config::provider::ProviderConfig;
use hydra_core::config::Config;
use hydra_core::conversation::message::{Message, MessageContent, Role};
use hydra_core::conversation::Conversation;
use hydra_core::ctx::{CtxBuilder, DefaultCtx};
use hydra_core::git::worktree::WorktreeManager;
use hydra_core::provider::LlmProvider;
use hydra_core::session::{Session, SessionManager};
use hydra_core::stream::{StreamEvent, TokenUsage};
use hydra_core::tool::{
    ApprovalRequirement, PermissionDecision, PermissionStore, Tool, ToolCall, ToolContext, ToolDef,
    ToolRegistry, ToolResult,
};
use hydra_core::turn::event::{TurnEvent, TurnResult};
use hydra_core::turn::permission::{InteractivePermissionDecider, PermissionDecider};
use hydra_core::turn::runner::TurnRunner;

struct MockProvider {
    events: Vec<StreamEvent>,
}

impl MockProvider {
    fn with_tool_call(tool_name: &str, args: &str) -> Self {
        Self {
            events: vec![
                StreamEvent::ToolCallStart {
                    id: "call_1".to_string(),
                    name: tool_name.to_string(),
                },
                StreamEvent::ToolCallDelta(args.to_string()),
                StreamEvent::ToolCallDone(ToolCall {
                    id: "call_1".to_string(),
                    name: tool_name.to_string(),
                    arguments: args.to_string(),
                }),
                StreamEvent::Usage(TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 8,
                    cached_tokens: 0,
                }),
                StreamEvent::Done { truncated: false },
            ],
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn chat_stream(
        &self,
        _messages: &[hydra_core::conversation::message::Message],
        _tools: Option<&[ToolDef]>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        let events: Vec<Result<StreamEvent>> = self.events.iter().cloned().map(Ok).collect();
        Ok(Box::pin(stream::iter(events)))
    }

    fn model_name(&self) -> &str {
        "mock-model"
    }
}

struct DangerousEchoTool;

#[async_trait]
impl Tool for DangerousEchoTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "dangerous",
            description: "Dangerous tool requiring approval".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }
    }

    fn approval(&self, _args: &str) -> ApprovalRequirement {
        ApprovalRequirement::RequireApproval("Needs explicit user confirmation".to_string())
    }

    async fn execute(&self, args: &str, _ctx: &ToolContext) -> Result<ToolResult> {
        Ok(ToolResult {
            call_id: String::new(),
            output: format!("approved execution with {}", args),
            success: true,
        })
    }
}

fn test_config() -> Config {
    let mut providers = HashMap::new();
    providers.insert(
        "test".to_string(),
        ProviderConfig {
            provider_type: "mock".to_string(),
            api_key: None,
            model: "mock-model".to_string(),
            base_url: None,
            system_prompt: None,
            user_agent: None,
            context_window: 16000,
            max_tokens: None,
            thinking_type: None,
            thinking_keep: None,
            reasoning_history: None,
            thinking_enabled: None,
            thinking_budget: None,
            skip_tls_verify: false,
            ephemeral: false,
        },
    );
    Config {
        default_provider: "test".to_string(),
        default_workdir: None,
        providers,
        datalog: Default::default(),
        notifications: Default::default(),
        auto_update: false,
        telemetry: Default::default(),
        lsp: Default::default(),
        auto_commit: false,
        subagent: Default::default(),
        vision_preprocessor_provider: None,
        language: None,
        ui: Default::default(),
        plugin: Default::default(),
    }
}

fn make_runner(
    provider: MockProvider,
    tools: ToolRegistry,
    permission: Box<dyn PermissionDecider>,
    working_dir: PathBuf,
) -> TurnRunner {
    let test_provider = ProviderConfig {
        provider_type: "test".into(),
        api_key: None,
        model: "test-model".into(),
        base_url: None,
        system_prompt: None,
        user_agent: None,
        context_window: 128_000,
        max_tokens: None,
        thinking_type: None,
        thinking_keep: None,
        reasoning_history: None,
        thinking_enabled: None,
        thinking_budget: None,
        skip_tls_verify: false,
        ephemeral: true,
    };
    let ctx: std::sync::Arc<dyn CtxBuilder> = std::sync::Arc::new(DefaultCtx::new(&test_provider));

    TurnRunner {
        provider: std::sync::Arc::new(provider),
        tools: std::sync::Arc::new(tools),
        context: ToolContext::new(working_dir),
        config: test_config(),
        ctx,
        permission,
        recently_edited_files: Vec::new(),
        hook_executor: std::sync::Arc::new(hydra_core::hook::executor::HookExecutor::empty()),
        loop_guard: Default::default(),
    }
}

fn collect_events(rx: &mut mpsc::UnboundedReceiver<TurnEvent>) -> Vec<TurnEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn last_tool_result(conv: &Conversation) -> &ToolResult {
    match &conv.messages.last().expect("last message").content {
        MessageContent::ToolResult(result) => result,
        other => panic!("expected ToolResult, got {:?}", other),
    }
}

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            std::env::set_var(self.key, old);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn cli_interactive_approval_emits_prompt_and_denies_tool_execution() {
    let tools = ToolRegistry::new();
    tools.register(Box::new(DangerousEchoTool)).await;

    let (req_tx, mut req_rx) = mpsc::unbounded_channel();
    let (resp_tx, resp_rx) = mpsc::unbounded_channel();
    let store = std::sync::Arc::new(std::sync::RwLock::new(PermissionStore::new()));
    let permission = Box::new(InteractivePermissionDecider::new(req_tx, resp_rx, store));

    let temp = tempfile::tempdir().expect("tempdir");
    let provider = MockProvider::with_tool_call("dangerous", r#"{"path":"demo.txt"}"#);
    let mut runner = make_runner(provider, tools, permission, temp.path().to_path_buf());
    let mut conv = Conversation::new();
    conv.add_user_message("please do the dangerous thing");
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let approval_task = tokio::spawn(async move {
        let req = req_rx.recv().await.expect("approval request");
        assert_eq!(req.call.name, "dangerous");
        assert!(req.reason.contains("explicit user confirmation"));
        resp_tx.send(PermissionDecision::Deny).expect("send deny");
    });

    let result = runner
        .run(&mut conv, "system", &event_tx, CancellationToken::new())
        .await;
    approval_task.await.expect("join approval task");

    match result {
        TurnResult::UsedTools { tool_count, .. } => assert_eq!(tool_count, 1),
        other => panic!("expected UsedTools, got {:?}", other),
    }

    let events = collect_events(&mut event_rx);
    assert!(
        events.iter().any(|event| matches!(
            event,
            TurnEvent::ApprovalRequested { tool_name, .. } if tool_name == "dangerous"
        )),
        "expected ApprovalRequested event"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            TurnEvent::ToolCallResult { name, success, output, .. }
                if name == "dangerous" && !success && output.contains("denied by the user")
        )),
        "expected denied ToolCallResult event"
    );

    let tool_result = last_tool_result(&conv);
    assert!(!tool_result.success);
    assert!(tool_result.output.contains("denied by the user"));
}

#[tokio::test]
async fn cli_interactive_approval_allows_tool_execution_after_confirmation() {
    let tools = ToolRegistry::new();
    tools.register(Box::new(DangerousEchoTool)).await;

    let (req_tx, mut req_rx) = mpsc::unbounded_channel();
    let (resp_tx, resp_rx) = mpsc::unbounded_channel();
    let store = std::sync::Arc::new(std::sync::RwLock::new(PermissionStore::new()));
    let permission = Box::new(InteractivePermissionDecider::new(req_tx, resp_rx, store));

    let temp = tempfile::tempdir().expect("tempdir");
    let provider = MockProvider::with_tool_call("dangerous", r#"{"path":"demo.txt"}"#);
    let mut runner = make_runner(provider, tools, permission, temp.path().to_path_buf());
    let mut conv = Conversation::new();
    conv.add_user_message("please do the dangerous thing");
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let approval_task = tokio::spawn(async move {
        let req = req_rx.recv().await.expect("approval request");
        assert_eq!(req.call.name, "dangerous");
        resp_tx.send(PermissionDecision::Allow).expect("send allow");
    });

    let result = runner
        .run(&mut conv, "system", &event_tx, CancellationToken::new())
        .await;
    approval_task.await.expect("join approval task");

    match result {
        TurnResult::UsedTools { tool_count, .. } => assert_eq!(tool_count, 1),
        other => panic!("expected UsedTools, got {:?}", other),
    }

    let events = collect_events(&mut event_rx);
    assert!(
        events.iter().any(|event| matches!(
            event,
            TurnEvent::ApprovalRequested { tool_name, .. } if tool_name == "dangerous"
        )),
        "expected ApprovalRequested event"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            TurnEvent::ToolCallResult { name, success, output, .. }
                if name == "dangerous" && *success && output.contains("approved execution")
        )),
        "expected successful ToolCallResult event"
    );

    let tool_result = last_tool_result(&conv);
    assert!(tool_result.success);
    assert!(tool_result.output.contains("approved execution"));
}

#[tokio::test]
async fn cli_session_grant_skips_second_prompt_and_executes_directly() {
    let tools = ToolRegistry::new();
    tools.register(Box::new(DangerousEchoTool)).await;

    let (req_tx, mut req_rx) = mpsc::unbounded_channel();
    let (_resp_tx, resp_rx) = mpsc::unbounded_channel();
    let store = std::sync::Arc::new(std::sync::RwLock::new(PermissionStore::new()));
    store.write().expect("write store").grant_session("dangerous");
    let permission = Box::new(InteractivePermissionDecider::new(req_tx, resp_rx, store));

    let temp = tempfile::tempdir().expect("tempdir");
    let provider = MockProvider::with_tool_call("dangerous", r#"{"path":"demo.txt"}"#);
    let mut runner = make_runner(provider, tools, permission, temp.path().to_path_buf());
    let mut conv = Conversation::new();
    conv.add_user_message("please do the dangerous thing");
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let result = runner
        .run(&mut conv, "system", &event_tx, CancellationToken::new())
        .await;

    match result {
        TurnResult::UsedTools { tool_count, .. } => assert_eq!(tool_count, 1),
        other => panic!("expected UsedTools, got {:?}", other),
    }

    assert!(req_rx.try_recv().is_err(), "did not expect interactive approval request");
    let events = collect_events(&mut event_rx);
    assert!(
        !events.iter().any(|event| matches!(event, TurnEvent::ApprovalRequested { .. })),
        "session grant should suppress ApprovalRequested"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            TurnEvent::ToolCallResult { name, success, .. } if name == "dangerous" && *success
        )),
        "expected successful ToolCallResult event"
    );

    let tool_result = last_tool_result(&conv);
    assert!(tool_result.success);
}

#[test]
fn worktree_rollback_cleanup_discards_isolated_changes_without_touching_main_repo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp
        .path()
        .join(format!("repo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&repo).expect("create repo");

    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.email", "contract@example.com"]);
    run_git(&repo, &["config", "user.name", "Contract Test"]);

    let tracked = repo.join("tracked.txt");
    std::fs::write(&tracked, "base\n").expect("write tracked file");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "base"]);

    let manager = WorktreeManager::new(repo.clone());
    let branch = format!("contract-{}", uuid::Uuid::new_v4().simple());
    let worktree = manager.create(&branch, "HEAD").expect("create worktree");

    std::fs::write(worktree.path.join("tracked.txt"), "worktree change\n")
        .expect("mutate worktree file");
    std::fs::write(worktree.path.join("ephemeral.txt"), "temp artifact\n")
        .expect("write extra file");

    manager.remove(&branch, true).expect("remove worktree");

    assert!(!worktree.path.exists(), "worktree directory should be removed");
    let listed = manager.list().expect("list worktrees");
    assert!(
        !listed.iter().any(|(candidate, _, _)| candidate == &branch),
        "removed worktree must disappear from git worktree list"
    );
    assert_eq!(
        std::fs::read_to_string(&tracked).expect("read main repo file"),
        "base\n",
        "main repo content must remain unchanged after worktree rollback"
    );
}

#[test]
#[serial]
fn session_manager_persists_session_under_hydra_home_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hydra_home = temp.path().join("hydra-home");
    let _guard = EnvVarGuard::set("HYDRA_HOME", &hydra_home);

    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("create project dir");

    let manager = SessionManager::new(&project_dir);
    let mut session = Session::new(project_dir.clone());
    session.rename("contract-session".to_string());
    session.messages.push(Message::new(Role::User, "verify persistence"));
    session.messages.push(Message::new(Role::Assistant, "persistence ok"));

    manager.save(&session).expect("save session");

    let sessions_root = SessionManager::sessions_root_dir();
    assert!(sessions_root.exists(), "sessions root should be created under HYDRA_HOME");

    let project_dirs: Vec<_> = std::fs::read_dir(&sessions_root)
        .expect("read sessions root")
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    assert_eq!(project_dirs.len(), 1, "expected exactly one project session directory");

    let files: Vec<_> = std::fs::read_dir(&project_dirs[0])
        .expect("read project dir")
        .map(|entry| entry.expect("file entry").path())
        .collect();
    assert_eq!(files.len(), 1, "expected exactly one session file");
    assert!(
        files[0]
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name == format!("{}.json", session.id))
            .unwrap_or(false),
        "session file name should be derived from session id"
    );

    let loaded = manager.load(&session.id).expect("load session");
    assert_eq!(loaded.id, session.id);
    assert_eq!(loaded.name, "contract-session");
    assert_eq!(loaded.working_dir, project_dir);
    assert_eq!(loaded.messages.len(), 2);
    assert!(matches!(loaded.messages[0].role, Role::User));
    assert_eq!(loaded.messages[0].text(), Some("verify persistence"));
    assert!(matches!(loaded.messages[1].role, Role::Assistant));
    assert_eq!(loaded.messages[1].text(), Some("persistence ok"));

    let metas = manager.list().expect("list sessions");
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].id, session.id);
    assert_eq!(metas[0].message_count, 2);
}
