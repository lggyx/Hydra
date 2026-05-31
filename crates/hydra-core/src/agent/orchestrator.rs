use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::{Notify, RwLock};
use tokio_util::sync::CancellationToken;

use super::commands::AgentEvent;
use super::traits::{Agent, AgentId, AgentKind, AgentOutcome, AgentResponse, AgentState, AgentStatus, ResourceHandle};
use crate::config::Config;
use crate::conversation::Conversation;
use crate::provider::LlmProvider;
use crate::tool::{ApprovalRequirement, Tool, ToolContext, ToolDef, ToolResult};
use crate::turn::event::TurnEvent;

/// Handle that allows an OrchestratorAgent to manage child agents.
/// Implemented by the daemon's AgentRegistry.
pub trait AgentControl: Send + Sync {
    fn create_execution(&self, task: &str, branch: Option<&str>, worktree: Option<&str>) -> String;
    fn start_agent(&self, id: &str, message: &str);
    fn cancel_agent(&self, id: &str);
    fn agent_status(&self, id: &str) -> Option<String>;
    fn list_agents(&self) -> Vec<(String, String)>; // (id, status)
}

pub struct OrchestratorAgent {
    id: AgentId,
    state: Arc<RwLock<AgentState>>,
    working_dir: PathBuf,
    config: Config,
    provider_config: crate::config::provider::ProviderConfig,
    provider: Option<Box<dyn LlmProvider>>,
    system_prompt: String,
    control: Arc<dyn AgentControl>,
    cancel_token: CancellationToken,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
    pending_input: Option<String>,
    notify: Arc<Notify>,
}

impl OrchestratorAgent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AgentId,
        working_dir: PathBuf,
        config: Config,
        provider_config: crate::config::provider::ProviderConfig,
        provider: Box<dyn LlmProvider>,
        system_prompt: String,
        control: Arc<dyn AgentControl>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let state = Arc::new(RwLock::new(AgentState {
            id,
            kind: AgentKind::Orchestrator,
            status: AgentStatus::Created,
            created_at: now,
            updated_at: now,
        }));
        Self {
            id,
            state,
            working_dir,
            config,
            provider_config,
            provider: Some(provider),
            system_prompt,
            control,
            cancel_token: CancellationToken::new(),
            event_tx: None,
            pending_input: None,
            notify: Arc::new(Notify::new()),
        }
    }
}

#[async_trait]
impl Agent for OrchestratorAgent {
    fn id(&self) -> AgentId { self.id }
    fn kind(&self) -> AgentKind { AgentKind::Orchestrator }
    fn state_snapshot(&self) -> AgentState { self.state.blocking_read().clone() }
    fn branch(&self) -> Option<&str> { None }
    fn worktree(&self) -> Option<&std::path::Path> { None }

    async fn run(&mut self, _resources: ResourceHandle) -> anyhow::Result<AgentOutcome> {
        {
            let mut s = self.state.write().await;
            s.status = AgentStatus::Running { turn: 0, max_turns: 100 };
            s.updated_at = now_ts();
        }

        let working_dir_path = self.working_dir.clone();
        let tool_context = crate::tool::ToolContext::new(working_dir_path.clone());
        let mut tool_registry = crate::tool::ToolRegistry::new();

        let spawn_tool = SpawnExecutionTool { control: self.control.clone() };
        let kill_tool = KillAgentTool { control: self.control.clone() };
        let inspect_tool = InspectAgentTool { control: self.control.clone() };

        tool_registry.register_sync(Box::new(spawn_tool));
        tool_registry.register_sync(Box::new(kill_tool));
        tool_registry.register_sync(Box::new(inspect_tool));

        let shared_tools = std::sync::Arc::new(tool_registry);

        let permission: Box<dyn crate::turn::permission::PermissionDecider> =
            Box::new(crate::turn::permission::AutoPermissionDecider::new(
                crate::turn::permission::AutoPermissionMode::BypassAll,
            ));

        let daemon_ctx = crate::ctx::for_provider(&self.provider_config);
        let provider = self.provider.take().ok_or_else(|| anyhow::anyhow!("provider already consumed"))?;

        let mut turn_runner = crate::turn::runner::TurnRunner {
            provider: provider.into(),
            tools: shared_tools,
            context: tool_context,
            config: self.config.clone(),
            ctx: daemon_ctx,
            permission,
            recently_edited_files: Vec::new(),
            hook_executor: std::sync::Arc::new(
                crate::hook::executor::HookExecutor::new(
                    crate::hook::json_config::load_hooks_config(
                        std::path::Path::new(&self.working_dir),
                    ),
                ),
            ),
            loop_guard: Default::default(),
        };

        let mut conversation = Conversation::new();
        let (turn_tx, mut turn_rx) = tokio::sync::mpsc::unbounded_channel::<TurnEvent>();
        let mut last_response = String::new();

        loop {
            if self.cancel_token.is_cancelled() {
                return Ok(AgentOutcome::Failed { error: "cancelled".to_string() });
            }

            if let Some(input) = self.pending_input.take() {
                conversation.add_user_message(&input);
            }

            while let Ok(_) = turn_rx.try_recv() {}
            let result = turn_runner
                .run(&mut conversation, &self.system_prompt, &turn_tx, self.cancel_token.clone())
                .await;
            while let Ok(_) = turn_rx.try_recv() {}

            match result {
                crate::turn::event::TurnResult::Responded { text, .. } => {
                    last_response = text;
                    {
                        let mut s = self.state.write().await;
                        s.status = AgentStatus::WaitingInput;
                        s.updated_at = now_ts();
                    }
                    tokio::select! {
                        _ = self.notify.notified() => continue,
                        _ = self.cancel_token.cancelled() => {
                            return Ok(AgentOutcome::Failed { error: "cancelled".to_string() });
                        }
                    }
                }
                crate::turn::event::TurnResult::UsedTools { .. } => continue,
                crate::turn::event::TurnResult::Failed(e) => {
                    return Ok(AgentOutcome::Failed { error: e });
                }
                crate::turn::event::TurnResult::Cancelled => {
                    return Ok(AgentOutcome::Failed { error: "cancelled".to_string() });
                }
            }
        }
    }

    async fn on_command(&mut self, cmd: super::commands::AgentCommand) -> AgentResponse {
        match cmd {
            super::commands::AgentCommand::Kill { reason: _ } => {
                self.cancel_token.cancel();
                self.notify.notify_one();
                AgentResponse::Ack
            }
            super::commands::AgentCommand::InjectHint { text } => {
                self.pending_input = Some(text);
                self.notify.notify_one();
                AgentResponse::Ack
            }
            super::commands::AgentCommand::SubmitTask { description } => {
                self.pending_input = Some(description);
                self.notify.notify_one();
                AgentResponse::Ack
            }
            super::commands::AgentCommand::Pause | super::commands::AgentCommand::Resume => {
                AgentResponse::Reject { reason: "pause/resume not supported".to_string() }
            }
            _ => AgentResponse::Reject { reason: "unsupported".to_string() },
        }
    }
}

// ── Meta-tools ──

struct SpawnExecutionTool { control: Arc<dyn AgentControl> }
#[async_trait]
impl Tool for SpawnExecutionTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "spawn_execution",
            description: "Spawn a new execution agent to work on a task. Returns the agent ID.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "task": {"type": "string", "description": "Task description for the execution agent"},
                    "branch": {"type": "string", "description": "Optional git branch name"}
                },
                "required": ["task"]
            }),
        }
    }
    fn approval(&self, _args: &str) -> ApprovalRequirement { ApprovalRequirement::AutoApprove }
    async fn execute(&self, args: &str, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        #[derive(Deserialize)] struct Args { task: String, branch: Option<String> }
        let a: Args = serde_json::from_str(args).map_err(|e| anyhow::anyhow!("{}", e))?;
        let agent_id = self.control.create_execution(&a.task, a.branch.as_deref(), None);
        self.control.start_agent(&agent_id, &a.task);
        Ok(ToolResult { call_id: String::new(), output: format!("Spawned execution agent: {}", agent_id), success: true })
    }
}

struct KillAgentTool { control: Arc<dyn AgentControl> }
#[async_trait]
impl Tool for KillAgentTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "kill_agent",
            description: "Cancel a running agent by ID.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": {"type": "string", "description": "Agent ID to cancel"}
                },
                "required": ["agent_id"]
            }),
        }
    }
    fn approval(&self, _args: &str) -> ApprovalRequirement { ApprovalRequirement::AutoApprove }
    async fn execute(&self, args: &str, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        #[derive(Deserialize)] struct Args { agent_id: String }
        let a: Args = serde_json::from_str(args).map_err(|e| anyhow::anyhow!("{}", e))?;
        self.control.cancel_agent(&a.agent_id);
        Ok(ToolResult { call_id: String::new(), output: format!("Cancelled agent: {}", a.agent_id), success: true })
    }
}

struct InspectAgentTool { control: Arc<dyn AgentControl> }
#[async_trait]
impl Tool for InspectAgentTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "inspect_agent",
            description: "Get the status of an agent by ID.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": {"type": "string", "description": "Agent ID to inspect"}
                },
                "required": ["agent_id"]
            }),
        }
    }
    fn approval(&self, _args: &str) -> ApprovalRequirement { ApprovalRequirement::AutoApprove }
    async fn execute(&self, args: &str, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        #[derive(Deserialize)] struct Args { agent_id: String }
        let a: Args = serde_json::from_str(args).map_err(|e| anyhow::anyhow!("{}", e))?;
        match self.control.agent_status(&a.agent_id) {
            Some(status) => Ok(ToolResult { call_id: String::new(), output: format!("Agent {}: {}", a.agent_id, status), success: true }),
            None => Ok(ToolResult { call_id: String::new(), output: format!("Agent {} not found", a.agent_id), success: false }),
        }
    }
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
