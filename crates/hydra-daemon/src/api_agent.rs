use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{json_error, AppState};

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Created,
    Queued,
    Running,
    WaitingInput,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub id: String,
    pub name: String,
    pub status: AgentStatus,
    pub provider: Option<String>,
    pub working_dir: String,
    pub session_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub last_event_seq: u64,
    pub summary: Option<String>,
    pub last_error: Option<AgentErrorInfo>,
    pub worktree_id: Option<String>,
    pub branch_name: Option<String>,
    pub parent_agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub seq: u64,
    pub agent_id: String,
    pub event_type: String,
    pub timestamp: u64,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct AgentCommand {
    pub id: String,
    pub agent_id: String,
    pub command_type: String,
    pub payload: Option<serde_json::Value>,
    pub created_at: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct CreateAgentRequest {
    pub name: Option<String>,
    pub provider: Option<String>,
    pub working_dir: Option<String>,
    pub session_id: Option<String>,
    pub initial_input: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct PostAgentCommandRequest {
    #[serde(rename = "type")]
    pub type_: String,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListAgentEventsQuery {
    pub after_seq: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListAgentsResponse {
    pub items: Vec<AgentSnapshot>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CreateAgentResponse {
    pub agent: AgentSnapshot,
}

#[derive(Debug, Serialize)]
pub(crate) struct GetAgentResponse {
    pub agent: AgentSnapshot,
}

#[derive(Debug, Serialize)]
pub(crate) struct PostAgentCommandResponse {
    pub accepted: bool,
    pub command_id: String,
    pub agent_id: String,
    pub status_before: String,
    pub status_after: String,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListAgentEventsResponse {
    pub items: Vec<AgentEvent>,
    pub next_seq: u64,
    pub has_more: bool,
}

pub struct AgentStore {
    agents: HashMap<String, AgentSnapshot>,
}

impl AgentStore {
    fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    fn insert(&mut self, agent: AgentSnapshot) {
        self.agents.insert(agent.id.clone(), agent);
    }

    fn get(&self, id: &str) -> Option<&AgentSnapshot> {
        self.agents.get(id).or_else(|| {
            let matches: Vec<_> = self.agents.values().filter(|a| a.id.starts_with(id)).collect();
            if matches.len() == 1 { Some(matches[0]) } else { None }
        })
    }

    fn list(&self) -> Vec<AgentSnapshot> {
        self.agents.values().cloned().collect()
    }

    fn resolve_id(&self, id: &str) -> Option<String> {
        if self.agents.contains_key(id) {
            return Some(id.to_string());
        }
        let matches: Vec<_> = self.agents.keys().filter(|k| k.starts_with(id)).collect();
        if matches.len() == 1 { Some(matches[0].clone()) } else { None }
    }

    fn update_status(&mut self, id: &str, status: AgentStatus) {
        let key = if self.agents.contains_key(id) {
            Some(id.to_string())
        } else {
            let matches: Vec<String> = self.agents.keys().filter(|k| k.starts_with(id)).cloned().collect();
            if matches.len() == 1 { Some(matches[0].clone()) } else { None }
        };
        if let Some(k) = key {
            if let Some(agent) = self.agents.get_mut(&k) {
                agent.status = status;
                agent.updated_at = now_ts();
            }
        }
    }
}

pub struct AgentEventStore {
    events: HashMap<String, Vec<AgentEvent>>,
}

impl AgentEventStore {
    fn new() -> Self {
        Self {
            events: HashMap::new(),
        }
    }

    fn append(&mut self, agent_id: &str, event: AgentEvent) {
        self.events
            .entry(agent_id.to_string())
            .or_default()
            .push(event);
    }

    fn query_after_seq(&self, agent_id: &str, after_seq: u64) -> Vec<AgentEvent> {
        self.events
            .get(agent_id)
            .map(|evts| evts.iter().filter(|e| e.seq > after_seq).cloned().collect())
            .unwrap_or_default()
    }
}

pub struct AgentRegistry {
    store: Arc<RwLock<AgentStore>>,
    event_store: Arc<RwLock<AgentEventStore>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(AgentStore::new())),
            event_store: Arc::new(RwLock::new(AgentEventStore::new())),
        }
    }

    pub(crate) async fn create(&self, req: CreateAgentRequest, default_working_dir: &str) -> AgentSnapshot {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ts();
        let agent = AgentSnapshot {
            id: id.clone(),
            name: req.name.unwrap_or_else(|| format!("agent-{}", &id[..8])),
            status: AgentStatus::Created,
            provider: req.provider,
            working_dir: req.working_dir.unwrap_or_else(|| default_working_dir.to_string()),
            session_id: req.session_id,
            created_at: now,
            updated_at: now,
            last_event_seq: 0,
            summary: None,
            last_error: None,
            worktree_id: None,
            branch_name: None,
            parent_agent_id: None,
        };
        self.store.write().await.insert(agent.clone());
        agent
    }

    pub(crate) async fn get(&self, id: &str) -> Option<AgentSnapshot> {
        self.store.read().await.get(id).cloned()
    }

    pub(crate) async fn list(&self) -> Vec<AgentSnapshot> {
        self.store.read().await.list()
    }

    pub(crate) async fn post_command(
        &self,
        id: &str,
        req: PostAgentCommandRequest,
    ) -> Result<PostAgentCommandResponse, (StatusCode, String)> {
        let current = {
            let store = self.store.read().await;
            store
                .get(id)
                .cloned()
                .ok_or((StatusCode::NOT_FOUND, format!("agent {} not found", id)))?
        };

        let status_before = current.status.clone();
        let new_status = match req.type_.as_str() {
            "start" => {
                if status_before != AgentStatus::Created {
                    return Err((
                        StatusCode::CONFLICT,
                        format!("cannot start agent in {:?} state", status_before),
                    ));
                }
                AgentStatus::Queued
            }
            "resume" => {
                if status_before != AgentStatus::WaitingInput {
                    return Err((
                        StatusCode::CONFLICT,
                        format!("cannot resume agent in {:?} state", status_before),
                    ));
                }
                AgentStatus::Running
            }
            "cancel" => match status_before {
                AgentStatus::Created
                | AgentStatus::Queued
                | AgentStatus::Running
                | AgentStatus::WaitingInput => AgentStatus::Cancelled,
                _ => {
                    return Err((
                        StatusCode::CONFLICT,
                        format!("cannot cancel agent in {:?} state", status_before),
                    ));
                }
            },
            "append_input" => {
                if status_before != AgentStatus::WaitingInput {
                    return Err((
                        StatusCode::CONFLICT,
                        format!("cannot append_input to agent in {:?} state", status_before),
                    ));
                }
                AgentStatus::Running
            }
            other => {
                return Err((
                    StatusCode::CONFLICT,
                    format!("unknown command type: {}", other),
                ));
            }
        };

        self.store.write().await.update_status(id, new_status.clone());

        let command_id = uuid::Uuid::new_v4().to_string();

        if req.type_.as_str() == "start" {
            self.spawn_mock_progression(id.to_string());
        } else if req.type_.as_str() == "append_input" {
            self.spawn_mock_progression(id.to_string());
        }

        Ok(PostAgentCommandResponse {
            accepted: true,
            command_id,
            agent_id: id.to_string(),
            status_before: format!("{:?}", status_before).to_lowercase(),
            status_after: format!("{:?}", new_status).to_lowercase(),
            message: None,
        })
    }

    pub(crate) async fn get_events(
        &self,
        id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Option<ListAgentEventsResponse> {
        let store = self.store.read().await;
        if store.get(id).is_none() {
            return None;
        }
        drop(store);

        let event_store = self.event_store.read().await;
        let all = event_store.query_after_seq(id, after_seq);
        let has_more = all.len() > limit;
        let items: Vec<AgentEvent> = all.into_iter().take(limit).collect();
        let next_seq = items.last().map(|e| e.seq).unwrap_or(after_seq);
        Some(ListAgentEventsResponse {
            items,
            next_seq,
            has_more,
        })
    }

    fn spawn_mock_progression(&self, agent_id: String) {
        let store = self.store.clone();
        let event_store = self.event_store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            {
                let mut s = store.write().await;
                s.update_status(&agent_id, AgentStatus::Running);
            }
            {
                let mut es = event_store.write().await;
                let seq = es
                    .events
                    .get(&agent_id)
                    .map(|v| v.len() as u64 + 1)
                    .unwrap_or(1);
                es.append(
                    &agent_id,
                    AgentEvent {
                        seq,
                        agent_id: agent_id.clone(),
                        event_type: "status_changed".to_string(),
                        timestamp: now_ts(),
                        payload: Some(serde_json::json!({"status": "running"})),
                    },
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            {
                let mut s = store.write().await;
                s.update_status(&agent_id, AgentStatus::Completed);
            }
            {
                let mut es = event_store.write().await;
                let seq = es
                    .events
                    .get(&agent_id)
                    .map(|v| v.len() as u64 + 1)
                    .unwrap_or(1);
                es.append(
                    &agent_id,
                    AgentEvent {
                        seq,
                        agent_id: agent_id.clone(),
                        event_type: "status_changed".to_string(),
                        timestamp: now_ts(),
                        payload: Some(serde_json::json!({"status": "completed"})),
                    },
                );
            }
        });
    }
}

pub(crate) async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let items = state.agent_registry.list().await;
    Json(ListAgentsResponse {
        items,
        next_cursor: None,
    })
    .into_response()
}

pub(crate) async fn create_agent(
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> impl IntoResponse {
    let default_dir = {
        let project = state.project.read().await;
        project.working_dir.to_string_lossy().to_string()
    };
    let agent = state.agent_registry.create(req, &default_dir).await;
    (StatusCode::CREATED, Json(CreateAgentResponse { agent })).into_response()
}

pub(crate) async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.agent_registry.get(&id).await {
        Some(agent) => Json(GetAgentResponse { agent }).into_response(),
        None => json_error(StatusCode::NOT_FOUND, format!("agent {} not found", id)).into_response(),
    }
}

pub(crate) async fn post_agent_command(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PostAgentCommandRequest>,
) -> impl IntoResponse {
    match state.agent_registry.post_command(&id, req).await {
        Ok(resp) => Json(resp).into_response(),
        Err((status, msg)) => json_error(status, msg).into_response(),
    }
}

pub(crate) async fn list_agent_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ListAgentEventsQuery>,
) -> impl IntoResponse {
    let after_seq = q.after_seq.unwrap_or(0);
    let limit = q.limit.unwrap_or(100);
    match state.agent_registry.get_events(&id, after_seq, limit).await {
        Some(resp) => Json(resp).into_response(),
        None => json_error(StatusCode::NOT_FOUND, format!("agent {} not found", id)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_agent_returns_created_status() {
        let registry = AgentRegistry::new();
        let req = CreateAgentRequest {
            name: Some("test-agent".to_string()),
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
        };
        let agent = registry.create(req, "/tmp").await;
        assert_eq!(agent.status, AgentStatus::Created);
        assert_eq!(agent.name, "test-agent");
    }

    #[tokio::test]
    async fn test_start_command_transitions_to_queued() {
        let registry = AgentRegistry::new();
        let req = CreateAgentRequest {
            name: Some("agent1".to_string()),
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
        };
        let agent = registry.create(req, "/tmp").await;

        let cmd = PostAgentCommandRequest {
            type_: "start".to_string(),
            payload: None,
        };
        let resp = registry.post_command(&agent.id, cmd).await.unwrap();
        assert!(resp.accepted);
        assert_eq!(resp.status_before, "created");
        assert_eq!(resp.status_after, "queued");

        let updated = registry.get(&agent.id).await.unwrap();
        assert_eq!(updated.status, AgentStatus::Queued);
    }

    #[tokio::test]
    async fn test_cancel_from_running() {
        let registry = AgentRegistry::new();
        let req = CreateAgentRequest {
            name: None,
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
        };
        let agent = registry.create(req, "/tmp").await;

        {
            let mut store = registry.store.write().await;
            store.update_status(&agent.id, AgentStatus::Running);
        }

        let cmd = PostAgentCommandRequest {
            type_: "cancel".to_string(),
            payload: None,
        };
        let resp = registry.post_command(&agent.id, cmd).await.unwrap();
        assert!(resp.accepted);
        assert_eq!(resp.status_before, "running");
        assert_eq!(resp.status_after, "cancelled");
    }

    #[tokio::test]
    async fn test_invalid_command_returns_conflict() {
        let registry = AgentRegistry::new();
        let req = CreateAgentRequest {
            name: None,
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
        };
        let agent = registry.create(req, "/tmp").await;

        let cmd = PostAgentCommandRequest {
            type_: "resume".to_string(),
            payload: None,
        };
        let result = registry.post_command(&agent.id, cmd).await;
        assert!(result.is_err());
        let (status, _msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_events_after_seq() {
        let registry = AgentRegistry::new();
        let req = CreateAgentRequest {
            name: None,
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
        };
        let agent = registry.create(req, "/tmp").await;

        {
            let mut es = registry.event_store.write().await;
            for i in 1..=5 {
                es.append(
                    &agent.id,
                    AgentEvent {
                        seq: i,
                        agent_id: agent.id.clone(),
                        event_type: "test".to_string(),
                        timestamp: now_ts(),
                        payload: None,
                    },
                );
            }
        }

        let resp = registry.get_events(&agent.id, 3, 100).await.unwrap();
        assert_eq!(resp.items.len(), 2);
        assert_eq!(resp.items[0].seq, 4);
        assert_eq!(resp.items[1].seq, 5);
        assert!(!resp.has_more);
    }
}
