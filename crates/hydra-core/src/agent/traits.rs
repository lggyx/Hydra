use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::{mpsc, RwLock};

use crate::tool::ToolRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct AgentId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum AgentKind {
    Execution,
    Orchestrator,
    Reviewer,
    Custom(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentState {
    pub id: AgentId,
    pub kind: AgentKind,
    pub status: AgentStatus,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub enum AgentStatus {
    Created,
    Running { turn: usize, max_turns: usize },
    WaitingInput,
    Completed { outcome: AgentOutcome },
    Killed { reason: String, at: u64 },
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize)]
pub enum AgentOutcome {
    Success { summary: String },
    Partial { summary: String },
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize)]
pub enum AgentResponse {
    Ack,
    Reject { reason: String },
}

#[derive(Clone)]
pub struct ResourceHandle {
    pub event_tx: mpsc::UnboundedSender<super::events::AgentEvent>,
    pub control_tx: mpsc::UnboundedSender<(AgentId, super::commands::AgentCommand)>,
    pub tool_registry: Arc<RwLock<ToolRegistry>>,
    pub working_dir: PathBuf,
}

#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> AgentId;
    fn kind(&self) -> AgentKind;
    fn state(&self) -> &AgentState;
    fn branch(&self) -> Option<&str>;
    fn worktree(&self) -> Option<&Path>;

    async fn run(&mut self, resources: ResourceHandle) -> anyhow::Result<AgentOutcome>;
    async fn on_command(&mut self, cmd: super::commands::AgentCommand) -> AgentResponse;
}
