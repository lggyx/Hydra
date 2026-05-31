use serde::Serialize;

use super::traits::AgentId;

#[derive(Debug, Clone, Serialize)]
pub enum AgentEvent {
    // Lifecycle (all agent kinds)
    Started { agent_id: AgentId, kind: super::traits::AgentKind },
    Completed { agent_id: AgentId, outcome: super::traits::AgentOutcome },
    Killed { agent_id: AgentId, reason: String },
    Failed { agent_id: AgentId, error: String },

    // ExecutionAgent
    Turn { agent_id: AgentId, data: serde_json::Value },
    ToolCall { agent_id: AgentId, tool: String, success: bool },

    // Orchestrator
    Decision { agent_id: AgentId, decisions: Vec<AgentCommand> },
    TaskSpawned { agent_id: AgentId, child_id: AgentId, desc: String },
}

#[derive(Debug, Clone, Serialize)]
pub enum AgentCommand {
    // Universal
    Kill { reason: String },
    Pause,
    Resume,

    // ExecutionAgent
    InjectHint { text: String },
    SwitchModel { provider: String, model: String },

    // Orchestrator
    SubmitTask { description: String },
    SetMaxConcurrency { n: usize },
}
