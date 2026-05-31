use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use super::commands::{AgentCommand, AgentEvent};
use super::traits::{Agent, AgentId, AgentKind, AgentOutcome, AgentResponse, AgentState, AgentStatus, ResourceHandle};
use crate::config::Config;
use crate::conversation::Conversation;
use crate::provider::LlmProvider;
use crate::session::{Session, SessionManager};
use crate::tool::ToolRegistry;
use crate::turn::event::{TurnEvent, TurnResult};
use crate::turn::runner::TurnRunner;

pub struct ExecutionAgent {
    id: AgentId,
    state: Arc<RwLock<AgentState>>,
    branch: Option<String>,
    worktree: Option<PathBuf>,

    working_dir: PathBuf,
    config: Config,
    provider_config: crate::config::provider::ProviderConfig,
    provider: Option<Box<dyn LlmProvider>>,
    system_prompt: String,
    session: Option<Session>,
    input_text: Option<String>,

    event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
    cancel_token: CancellationToken,

    tool_registry: Option<Arc<ToolRegistry>>,
}

impl ExecutionAgent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AgentId,
        working_dir: PathBuf,
        config: Config,
        provider_config: crate::config::provider::ProviderConfig,
        provider: Option<Box<dyn LlmProvider>>,
        system_prompt: String,
        session: Option<Session>,
        input_text: Option<String>,
        branch: Option<String>,
        worktree: Option<PathBuf>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let state = Arc::new(RwLock::new(AgentState {
            id,
            kind: AgentKind::Execution,
            status: AgentStatus::Created,
            created_at: now,
            updated_at: now,
        }));
        Self {
            id,
            state,
            branch,
            worktree,
            working_dir,
            config,
            provider_config,
            provider,
            system_prompt,
            session,
            input_text,
            event_tx: None,
            cancel_token: CancellationToken::new(),
            tool_registry: None,
        }
    }
}

#[async_trait]
impl Agent for ExecutionAgent {
    fn id(&self) -> AgentId { self.id }
    fn kind(&self) -> AgentKind { AgentKind::Execution }
    fn state_snapshot(&self) -> AgentState {
        self.state.blocking_read().clone()
    }
    fn branch(&self) -> Option<&str> { self.branch.as_deref() }
    fn worktree(&self) -> Option<&std::path::Path> { self.worktree.as_deref() }

    async fn run(&mut self, _resources: ResourceHandle) -> anyhow::Result<AgentOutcome> {
        use crate::tool::{
            bash::BashTool, edit::EditFileTool, glob::GlobTool, grep::GrepTool,
            list_dir::ListDirTool, read::ReadFileTool, search_replace::SearchReplaceTool,
            todo::TodoTool, web_fetch::WebFetchTool, web_search::WebSearchTool,
            write::WriteFileTool,
        };
        // Set status to Running
        {
            let mut s = self.state.write().await;
            s.status = AgentStatus::Running { turn: 0, max_turns: 100 };
            s.updated_at = now_ts();
        }

        // Build session
        let session_manager = SessionManager::new(std::path::Path::new(&self.working_dir));
        let mut session = self.session.take().unwrap_or_else(|| {
            Session::new(self.working_dir.clone().into())
        });

        // Build conversation
        let mut conversation = Conversation::new();
        conversation.messages = session.messages.clone();
        if let Some(ref text) = self.input_text {
            conversation.add_user_message(text);
        }

        // Build tool registry
        let working_dir_path = self.working_dir.clone();
        let tool_context = crate::tool::ToolContext::new(working_dir_path.clone());
        let mut tool_registry = ToolRegistry::new();

        tool_registry.register_sync(Box::new(ReadFileTool));
        tool_registry.register_sync(Box::new(WriteFileTool));
        tool_registry.register_sync(Box::new(EditFileTool));
        tool_registry.register_sync(Box::new(BashTool));
        tool_registry.register_sync(Box::new(GrepTool));
        tool_registry.register_sync(Box::new(GlobTool));
        tool_registry.register_sync(Box::new(ListDirTool));
        tool_registry.register_sync(Box::new(SearchReplaceTool));
        tool_registry.register_sync(Box::new(WebSearchTool));
        tool_registry.register_sync(Box::new(WebFetchTool));
        tool_registry.register_sync(Box::new(TodoTool::new()));

        // Skills
        let mut skill_registry = crate::skill::SkillRegistry::new();
        skill_registry.reload(std::path::Path::new(&self.working_dir));
        let skill_registry_arc = std::sync::Arc::new(std::sync::RwLock::new(skill_registry));
        if !skill_registry_arc.read().unwrap().is_empty() {
            tool_registry.register_sync(Box::new(crate::tool::use_skill::UseSkillTool {
                registry: skill_registry_arc.clone(),
            }));
        }

        let shared_tools = std::sync::Arc::new(tool_registry);
        self.tool_registry = Some(shared_tools.clone());

        // Permission
        let permission: Box<dyn crate::turn::permission::PermissionDecider> =
            Box::new(crate::turn::permission::AutoPermissionDecider::new(
                crate::turn::permission::AutoPermissionMode::BypassAll,
            ));

        // Context
        let daemon_ctx = crate::ctx::for_provider(&self.provider_config);

        // TurnRunner
        let provider = self.provider.take().ok_or_else(|| anyhow::anyhow!("provider already consumed"))?;
        let mut turn_runner = TurnRunner {
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

        // Event channel
        let (turn_tx, mut turn_rx) = mpsc::unbounded_channel::<TurnEvent>();
        let mut total_tool_calls: usize = 0;

        // Turn loop
        let final_summary = loop {
            if self.cancel_token.is_cancelled() {
                let mut s = self.state.write().await;
                s.status = AgentStatus::Killed { reason: "cancelled".to_string(), at: now_ts() };
                s.updated_at = now_ts();
                return Ok(AgentOutcome::Failed { error: "cancelled".to_string() });
            }

            while let Ok(evt) = turn_rx.try_recv() {
                Self::forward_event(&self.event_tx, self.id, &evt, &mut total_tool_calls);
            }

            let result = turn_runner
                .run(&mut conversation, &self.system_prompt, &turn_tx, self.cancel_token.clone())
                .await;

            while let Ok(evt) = turn_rx.try_recv() {
                Self::forward_event(&self.event_tx, self.id, &evt, &mut total_tool_calls);
            }

            match result {
                TurnResult::Responded { text, .. } => break text,
                TurnResult::UsedTools { .. } => continue,
                TurnResult::Failed(e) => {
                    let mut s = self.state.write().await;
                    s.status = AgentStatus::Failed { error: e.clone() };
                    s.updated_at = now_ts();
                    return Ok(AgentOutcome::Failed { error: e });
                }
                TurnResult::Cancelled => {
                    let mut s = self.state.write().await;
                    s.status = AgentStatus::Killed { reason: "cancelled".to_string(), at: now_ts() };
                    s.updated_at = now_ts();
                    return Ok(AgentOutcome::Failed { error: "cancelled".to_string() });
                }
            }
        };

        // Save session
        session.messages = conversation.messages;
        session.auto_name_from_messages();
        session.touch();
        let _ = session_manager.save(&session);

        // Mark completed
        {
            let mut s = self.state.write().await;
            s.status = AgentStatus::Completed {
                outcome: AgentOutcome::Success {
                    summary: final_summary.clone(),
                },
            };
            s.updated_at = now_ts();
        }

        Ok(AgentOutcome::Success { summary: final_summary })
    }

    async fn on_command(&mut self, cmd: AgentCommand) -> AgentResponse {
        match cmd {
            AgentCommand::Kill { reason: _ } => {
                self.cancel_token.cancel();
                AgentResponse::Ack
            }
            AgentCommand::Pause => AgentResponse::Reject {
                reason: "ExecutionAgent does not support pause".to_string(),
            },
            AgentCommand::Resume => AgentResponse::Reject {
                reason: "ExecutionAgent does not support resume".to_string(),
            },
            AgentCommand::InjectHint { text } => {
                self.input_text = Some(text);
                AgentResponse::Ack
            }
            _ => AgentResponse::Reject {
                reason: "unsupported command for ExecutionAgent".to_string(),
            },
        }
    }
}

impl ExecutionAgent {
    fn forward_event(
        tx: &Option<mpsc::UnboundedSender<AgentEvent>>,
        agent_id: AgentId,
        evt: &TurnEvent,
        tool_calls: &mut usize,
    ) {
        if let Some(ref tx) = tx {
            match evt {
                TurnEvent::TextDelta(text) => {
                    let _ = tx.send(AgentEvent::Turn {
                        agent_id,
                        data: serde_json::json!({"delta": text}),
                    });
                }
                TurnEvent::ToolCallStreaming { name, .. } | TurnEvent::ToolCallStarted { name, .. } => {
                    let _ = tx.send(AgentEvent::ToolCall {
                        agent_id,
                        tool: name.clone(),
                        success: true, // will be overwritten by result
                    });
                }
                TurnEvent::ToolCallResult { name, success, .. } => {
                    *tool_calls += 1;
                    let _ = tx.send(AgentEvent::ToolCall {
                        agent_id,
                        tool: name.clone(),
                        success: *success,
                    });
                }
                _ => {}
            }
        }
    }
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
