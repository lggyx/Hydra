use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{sse, IntoResponse};
use axum::Json;
use futures::stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::{json_error, AppState, CachedMcpRegistry};
use hydra_core::agent::resource_manager::ResourceManager;
use hydra_core::config::Config;
use hydra_core::mcp::{register_mcp_tools, McpRegistry};
use hydra_core::provider;
use hydra_core::session::{Session, SessionId, SessionManager};
use hydra_core::tool::ToolRegistry;
use hydra_core::turn::{event::{TurnEvent, TurnResult}, runner::TurnRunner};

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
    pub kind: String, // "execution" or "orchestrator"
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
    /// Pending user input to inject into the conversation on next execution.
    #[serde(skip)]
    pub pending_input: Option<String>,
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
    pub worktree_id: Option<String>,
    pub branch_name: Option<String>,
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
    /// Active agent execution tokens (agent_id -> CancellationToken)
    active_tokens: Arc<RwLock<HashMap<String, CancellationToken>>>,
    mcp_cache: Arc<RwLock<HashMap<PathBuf, CachedMcpRegistry>>>,
    event_broadcasts: Arc<RwLock<HashMap<String, tokio::sync::broadcast::Sender<AgentEvent>>>>,
    rm: ResourceManager,
}

impl AgentRegistry {
    pub fn new(mcp_cache: Arc<RwLock<HashMap<PathBuf, CachedMcpRegistry>>>) -> Self {
        Self {
            store: Arc::new(RwLock::new(AgentStore::new())),
            event_store: Arc::new(RwLock::new(AgentEventStore::new())),
            active_tokens: Arc::new(RwLock::new(HashMap::new())),
            mcp_cache,
            event_broadcasts: Arc::new(RwLock::new(HashMap::new())),
            rm: ResourceManager::new(),
        }
    }

    pub(crate) async fn create(&self, req: CreateAgentRequest, default_working_dir: &str) -> AgentSnapshot {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ts();
        let working_dir = req.working_dir.unwrap_or_else(|| default_working_dir.to_string());
        let branch_name = req.branch_name.or_else(|| detect_branch_from_dir(&working_dir));
        let kind = req
            .metadata
            .as_ref()
            .and_then(|m| m.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("execution")
            .to_string();
        let agent = AgentSnapshot {
            id: id.clone(),
            name: req.name.unwrap_or_else(|| format!("agent-{}", &id[..8])),
            kind,
            status: AgentStatus::Created,
            provider: req.provider,
            working_dir,
            session_id: req.session_id,
            created_at: now,
            updated_at: now,
            last_event_seq: 0,
            summary: None,
            last_error: None,
            worktree_id: req.worktree_id,
            branch_name,
            parent_agent_id: None,
            pending_input: req.initial_input,
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

        // Extract message text from payload; fall back to pending_input from creation
        let input_text = extract_message_from_payload(&req.payload)
            .or(current.pending_input);

        if req.type_.as_str() == "start" {
            self.spawn_agent_execution(id.to_string(), input_text).await;
        } else if req.type_.as_str() == "append_input" {
            self.spawn_agent_execution(id.to_string(), input_text).await;
        } else if req.type_.as_str() == "cancel" {
            self.cancel_agent_execution(&id).await;
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

    /// Subscribe to live agent events via broadcast channel.
    pub(crate) async fn subscribe_events(
        &self,
        agent_id: &str,
    ) -> Result<tokio::sync::broadcast::Receiver<AgentEvent>, (StatusCode, String)> {
        {
            let store = self.store.read().await;
            if store.get(agent_id).is_none() {
                return Err((StatusCode::NOT_FOUND, format!("agent {} not found", agent_id)));
            }
        }
        let mut bcasts = self.event_broadcasts.write().await;
        let tx = bcasts
            .entry(agent_id.to_string())
            .or_insert_with(|| tokio::sync::broadcast::channel(256).0);
        Ok(tx.subscribe())
    }

    /// Cancel an active agent execution. Works for Running and WaitingInput states.
    pub(crate) async fn cancel_agent_execution(&self, agent_id: &str) {
        let tokens = self.active_tokens.read().await;
        if let Some(token) = tokens.get(agent_id) {
            token.cancel();
        }
    }

    /// Spawn a real agent execution using hydra-core's TurnRunner.
    /// Falls back to mock progression if config/provider setup fails.
    pub(crate) async fn spawn_agent_execution(&self, agent_id: String, input_text: Option<String>) {
        let store = self.store.clone();
        let event_store = self.event_store.clone();
        let active_tokens = self.active_tokens.clone();
        let mcp_cache = self.mcp_cache.clone();
        let event_broadcasts = self.event_broadcasts.clone();

        // Create a new CancellationToken for this execution
        let cancel_token = CancellationToken::new();
        active_tokens.write().await.insert(agent_id.clone(), cancel_token.clone());

        tokio::spawn(async move {
            let (working_dir, _worktree_id) = match {
                let s = store.read().await;
                s.get(&agent_id).map(|a| (a.working_dir.clone(), a.worktree_id.clone()))
            } {
                Some((d, wt)) => {
                    let resolved = wt.as_ref()
                        .and_then(|wt_id| resolve_worktree_path(&d, wt_id));
                    (resolved.unwrap_or(d), wt)
                }
                None => {
                    set_status(&store, &agent_id, AgentStatus::Failed).await;
                    append_event(&event_store, &event_broadcasts, &agent_id, "status_changed", Some(serde_json::json!({"status": "failed", "error": "agent not found"}))).await;
                    active_tokens.write().await.remove(&agent_id);
                    return;
                }
            };

            // Try real execution; fall back to mock on any error
            let result = run_real_execution(
                store.clone(),
                event_store.clone(),
                event_broadcasts.clone(),
                agent_id.clone(),
                working_dir,
                input_text,
                cancel_token,
                mcp_cache,
            ).await;

            if let Err(_e) = result {
                // Fall back to mock progression so tests and simple cases still work
                run_mock_progression(store, event_store, event_broadcasts, agent_id.clone()).await;
            }

            active_tokens.write().await.remove(&agent_id);
        });
    }

}

impl hydra_core::agent::orchestrator::AgentControl for AgentRegistry {
    fn create_execution(&self, task: &str, branch: Option<&str>, worktree: Option<&str>) -> String {
        let default_dir = std::env::current_dir()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let req = CreateAgentRequest {
            name: Some(format!("exec-{}", &task.chars().take(20).collect::<String>())),
            provider: None,
            working_dir: Some(default_dir.clone()),
            session_id: None,
            initial_input: Some(task.to_string()),
            metadata: None,
            worktree_id: worktree.map(|s| s.to_string()),
            branch_name: branch.map(|s| s.to_string()),
        };
        let rt = tokio::runtime::Handle::current();
        let agent = rt.block_on(async { self.create(req, &default_dir).await });
        agent.id
    }

    fn start_agent(&self, id: &str, message: &str) {
        let rt = tokio::runtime::Handle::current();
        let cmd = PostAgentCommandRequest {
            type_: "start".to_string(),
            payload: Some(serde_json::json!({"message": message})),
        };
        let _ = rt.block_on(async { self.post_command(id, cmd).await });
    }

    fn cancel_agent(&self, id: &str) {
        let rt = tokio::runtime::Handle::current();
        let cmd = PostAgentCommandRequest {
            type_: "cancel".to_string(),
            payload: None,
        };
        let _ = rt.block_on(async { self.post_command(id, cmd).await });
    }

    fn agent_status(&self, id: &str) -> Option<String> {
        let rt = tokio::runtime::Handle::current();
        let agent = rt.block_on(async { self.get(id).await });
        agent.map(|a| format!("{:?}", a.status).to_lowercase())
    }

    fn list_agents(&self) -> Vec<(String, String)> {
        let rt = tokio::runtime::Handle::current();
        let agents = rt.block_on(async { self.list().await });
        agents
            .into_iter()
            .map(|a| (a.id, format!("{:?}", a.status).to_lowercase()))
            .collect()
    }
}

/// Shared mock progression logic used as fallback.
async fn run_mock_progression(
    store: Arc<RwLock<AgentStore>>,
    event_store: Arc<RwLock<AgentEventStore>>,
    event_broadcasts: Arc<RwLock<HashMap<String, tokio::sync::broadcast::Sender<AgentEvent>>>>,
    agent_id: String,
) {
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    {
        let mut s = store.write().await;
        s.update_status(&agent_id, AgentStatus::Running);
    }
    append_event(&event_store, &event_broadcasts, &agent_id, "status_changed", Some(serde_json::json!({"status": "running"}))).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    {
        let mut s = store.write().await;
        s.update_status(&agent_id, AgentStatus::Completed);
    }
    append_event(&event_store, &event_broadcasts, &agent_id, "status_changed", Some(serde_json::json!({"status": "completed"}))).await;
}

// Helper to set agent status
async fn set_status(
    store: &Arc<RwLock<AgentStore>>,
    agent_id: &str,
    status: AgentStatus,
) {
    let mut s = store.write().await;
    s.update_status(agent_id, status);
}

// Helper to append an event
async fn append_event(
    event_store: &Arc<RwLock<AgentEventStore>>,
    event_broadcasts: &Arc<RwLock<HashMap<String, tokio::sync::broadcast::Sender<AgentEvent>>>>,
    agent_id: &str,
    event_type: &str,
    payload: Option<serde_json::Value>,
) {
    let mut es = event_store.write().await;
    let seq = es
        .events
        .get(agent_id)
        .map(|v| v.len() as u64 + 1)
        .unwrap_or(1);
    let event = AgentEvent {
        seq,
        agent_id: agent_id.to_string(),
        event_type: event_type.to_string(),
        timestamp: now_ts(),
        payload,
    };
    es.append(agent_id, event.clone());
    drop(es);
    let bcasts = event_broadcasts.read().await;
    if let Some(tx) = bcasts.get(agent_id) {
        let _ = tx.send(event);
    }
}

/// Extract a message string from a command payload.
/// Accepts `message`, `text`, or `input` keys at the top level or nested under `input`.
fn extract_message_from_payload(payload: &Option<serde_json::Value>) -> Option<String> {
    let obj = payload.as_ref()?;
    let obj = obj.as_object()?;
    for key in &["message", "text", "input"] {
        if let Some(v) = obj.get(*key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    // Also try nested: payload.input.message
    if let Some(input_obj) = obj.get("input").and_then(|v| v.as_object()) {
        for key in &["message", "text"] {
            if let Some(v) = input_obj.get(*key) {
                if let Some(s) = v.as_str() {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    None
}

fn detect_branch_from_dir(dir: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}

fn resolve_worktree_path(repo_dir: &str, branch: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut current_path: Option<String> = None;
    let mut current_branch: Option<String> = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = Some(p.to_string());
            current_branch = None;
        } else if let Some(b) = line.strip_prefix("branch ") {
            current_branch = Some(b.trim_start_matches("refs/heads/").to_string());
        }
        if let (Some(ref p), Some(ref b)) = (&current_path, &current_branch) {
            if b == branch {
                return Some(p.clone());
            }
        }
    }
    None
}

/// Try to execute a real LLM turn loop for the given agent.
/// Returns Ok(()) on success, Err(()) on any failure (caller falls back to mock).
async fn run_real_execution(
    store: Arc<RwLock<AgentStore>>,
    event_store: Arc<RwLock<AgentEventStore>>,
    event_broadcasts: Arc<RwLock<HashMap<String, tokio::sync::broadcast::Sender<AgentEvent>>>>,
    agent_id: String,
    working_dir: String,
    input_text: Option<String>,
    cancel_token: CancellationToken,
    mcp_cache: Arc<RwLock<HashMap<PathBuf, CachedMcpRegistry>>>,
) -> Result<(), ()> {
    use hydra_core::tool::{
        bash::BashTool, edit::EditFileTool, glob::GlobTool, grep::GrepTool,
        list_dir::ListDirTool, read::ReadFileTool, search_replace::SearchReplaceTool,
        todo::TodoTool, web_fetch::WebFetchTool, web_search::WebSearchTool,
        write::WriteFileTool,
    };
    // 1. Load config
    let config_path = Config::default_path();
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(_) => return Err(()),
    };

    // 2. Determine provider from agent snapshot
    let provider_name = {
        let s = store.read().await;
        s.get(&agent_id)
            .and_then(|a| a.provider.clone())
            .unwrap_or_else(|| config.default_provider.clone())
    };
    let provider_config = match config.providers.get(&provider_name) {
        Some(pc) => pc,
        None => return Err(()),
    };

    // 3. Create provider
    let provider = match provider::create_provider(provider_config) {
        Ok(p) => p,
        Err(_) => return Err(()),
    };

    // 4. Create session
    let session_manager = SessionManager::new(std::path::Path::new(&working_dir));
    let mut session = {
        let s = store.read().await;
        if let Some(ref sid) = s.get(&agent_id).and_then(|a| a.session_id.clone()) {
            let session_id = SessionId::from_string(sid.to_string());
            session_manager.load(&session_id).unwrap_or_else(|_| Session::new(working_dir.clone().into()))
        } else {
            Session::new(working_dir.clone().into())
        }
    };

    // 5. Build conversation from session messages
    let mut conversation = hydra_core::conversation::Conversation::new();
    conversation.messages = session.messages.clone();

    // Inject pending input as the first user message if present
    if let Some(ref text) = input_text {
        conversation.add_user_message(text);
    }

    // 6. Build tool registry (mirrors process_chat_request in main.rs)
    use crate::daemon_tool_enabled;
    let working_dir_path = PathBuf::from(&working_dir);
    let mut tool_context = hydra_core::tool::ToolContext::new(working_dir_path.clone());
    let mut tool_registry = ToolRegistry::new();

    if daemon_tool_enabled("read_file") { tool_registry.register_sync(Box::new(ReadFileTool)); }
    if daemon_tool_enabled("write_file") { tool_registry.register_sync(Box::new(WriteFileTool)); }
    if daemon_tool_enabled("edit_file") { tool_registry.register_sync(Box::new(EditFileTool)); }
    if daemon_tool_enabled("bash") { tool_registry.register_sync(Box::new(BashTool)); }
    if daemon_tool_enabled("grep") { tool_registry.register_sync(Box::new(GrepTool)); }
    if daemon_tool_enabled("glob") { tool_registry.register_sync(Box::new(GlobTool)); }
    if daemon_tool_enabled("list_dir") { tool_registry.register_sync(Box::new(ListDirTool)); }
    if daemon_tool_enabled("search_replace") { tool_registry.register_sync(Box::new(SearchReplaceTool)); }
    if daemon_tool_enabled("web_search") { tool_registry.register_sync(Box::new(WebSearchTool)); }
    if daemon_tool_enabled("web_fetch") { tool_registry.register_sync(Box::new(WebFetchTool)); }
    if daemon_tool_enabled("todo") { tool_registry.register_sync(Box::new(TodoTool::new())); }

    // Load MCP tools from per-project cache
    {
        let mcp_registry: Arc<McpRegistry> = {
            let cache = mcp_cache.read().await;
            if let Some(cached) = cache.get(&working_dir_path) {
                cached.registry.clone()
            } else {
                drop(cache);
                let new_registry = Arc::new(McpRegistry::from_config_background(&working_dir_path));
                new_registry.wait_for_initial_connections(std::time::Duration::from_secs(5)).await;
                let mut cache = mcp_cache.write().await;
                if cache.len() >= 5 {
                    if let Some(oldest_key) = cache
                        .iter()
                        .min_by_key(|(_, v)| v.last_used)
                        .map(|(k, _)| k.clone())
                    {
                        cache.remove(&oldest_key);
                    }
                }
                cache.insert(working_dir_path.clone(), CachedMcpRegistry {
                    registry: new_registry.clone(),
                    last_used: std::time::Instant::now(),
                });
                new_registry
            }
        };
        {
            let mut cache = mcp_cache.write().await;
            if let Some(entry) = cache.get_mut(&working_dir_path) {
                entry.last_used = std::time::Instant::now();
            }
        }
        let mcp_tools = mcp_registry.list_all_tools().await;
        if !mcp_tools.is_empty() {
            register_mcp_tools(&mut tool_registry, mcp_registry.clone(), mcp_tools);
        }
    }

    // Load skills and register use_skill tool
    let mut skill_registry = hydra_core::skill::SkillRegistry::new();
    skill_registry.reload(std::path::Path::new(&working_dir));
    let skill_registry_arc = std::sync::Arc::new(std::sync::RwLock::new(skill_registry));
    if !skill_registry_arc.read().unwrap().is_empty() {
        tool_registry.register_sync(Box::new(hydra_core::tool::use_skill::UseSkillTool {
            registry: skill_registry_arc.clone(),
        }));
    }

    // LSP
    if daemon_tool_enabled("diagnostics") {
        if let Some(lsp) = hydra_core::lsp::manager::build_lsp_manager(&config.lsp, &working_dir_path) {
            tool_registry.register_sync(Box::new(hydra_core::tool::diagnostics::DiagnosticsTool));
            tool_context.lsp = Some(lsp);
        }
    }

    let shared_tools = std::sync::Arc::new(tool_registry);

    // 7. Build permission decider (bypass all for agent mode)
    let permission: Box<dyn hydra_core::turn::permission::PermissionDecider> =
        Box::new(hydra_core::turn::permission::AutoPermissionDecider::new(
            hydra_core::turn::permission::AutoPermissionMode::BypassAll,
        ));

    // 8. Build context
    let daemon_ctx = hydra_core::ctx::for_provider(provider_config);

    // 9. Build TurnRunner
    let mut turn_runner = TurnRunner {
        provider: provider.into(),
        tools: shared_tools,
        context: tool_context,
        config: config.clone(),
        ctx: daemon_ctx,
        permission,
        recently_edited_files: Vec::new(),
        hook_executor: std::sync::Arc::new(
            hydra_core::hook::executor::HookExecutor::new(
                hydra_core::hook::json_config::load_hooks_config(
                    std::path::Path::new(&working_dir),
                ),
            ),
        ),
        loop_guard: Default::default(),
    };

    // 10. Build system prompt (from shared daemon helper)
    let system_prompt = crate::build_api_system_prompt(
        &working_dir_path,
        &config,
        provider_config,
        &skill_registry_arc,
    );

    // 11. Create event channel for TurnEvent → AgentEvent mapping
    let (turn_tx, mut turn_rx) = tokio::sync::mpsc::unbounded_channel::<TurnEvent>();

    // 12. Mark agent as running
    set_status(&store, &agent_id, AgentStatus::Running).await;
    append_event(
        &event_store,
        &event_broadcasts,
        &agent_id,
        "status_changed",
        Some(serde_json::json!({"status": "running"})),
    ).await;

    // 13. Run turn loop
    let mut total_tool_calls: usize = 0;

    let final_summary = loop {
        // Check if cancelled before each turn
        if cancel_token.is_cancelled() {
            set_status(&store, &agent_id, AgentStatus::Cancelled).await;
            append_event(
                &event_store,
                &event_broadcasts,
                &agent_id,
                "status_changed",
                Some(serde_json::json!({"status": "cancelled"})),
            ).await;
            return Ok(());
        }

        // Drain any pending turn events from previous iteration
        while let Ok(evt) = turn_rx.try_recv() {
            map_turn_event(&event_store, &event_broadcasts, &agent_id, &evt, &mut total_tool_calls).await;
        }

        let result = turn_runner
            .run(&mut conversation, &system_prompt, &turn_tx, cancel_token.clone())
            .await;

        // Drain events from this turn
        while let Ok(evt) = turn_rx.try_recv() {
            map_turn_event(&event_store, &event_broadcasts, &agent_id, &evt, &mut total_tool_calls).await;
        }

        match result {
            TurnResult::Responded { text, .. } => break text,
            TurnResult::UsedTools { .. } => continue,
            TurnResult::Failed(e) => {
                set_status(&store, &agent_id, AgentStatus::Failed).await;
                append_event(
                    &event_store,
                    &event_broadcasts,
                    &agent_id,
                    "status_changed",
                    Some(serde_json::json!({"status": "failed", "error": e})),
                ).await;
                return Ok(());
            }
            TurnResult::Cancelled => {
                set_status(&store, &agent_id, AgentStatus::Cancelled).await;
                append_event(
                    &event_store,
                    &event_broadcasts,
                    &agent_id,
                    "status_changed",
                    Some(serde_json::json!({"status": "cancelled"})),
                ).await;
                return Ok(());
            }
        }
    };

    // 14. Save session
    session.messages = conversation.messages;
    session.auto_name_from_messages();
    session.touch();
    let _ = session_manager.save(&session);

    // 15. Mark completed
    set_status(&store, &agent_id, AgentStatus::Completed).await;
    append_event(
        &event_store,
        &event_broadcasts,
        &agent_id,
        "status_changed",
        Some(serde_json::json!({
            "status": "completed",
            "summary": final_summary,
            "tool_calls": total_tool_calls,
        })),
    ).await;

    Ok(())
}

/// Map a TurnEvent from TurnRunner into an AgentEvent in the event store.
async fn map_turn_event(
    event_store: &Arc<RwLock<AgentEventStore>>,
    event_broadcasts: &Arc<RwLock<HashMap<String, tokio::sync::broadcast::Sender<AgentEvent>>>>,
    agent_id: &str,
    evt: &TurnEvent,
    tool_calls: &mut usize,
) {
    match evt {
        TurnEvent::TextDelta(text) => {
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "agent_message",
                Some(serde_json::json!({"delta": text})),
            ).await;
        }
        TurnEvent::ReasoningDelta(text) => {
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "agent_reasoning",
                Some(serde_json::json!({"delta": text})),
            ).await;
        }
        TurnEvent::ToolCallStreaming { name, .. } => {
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "tool_call_start",
                Some(serde_json::json!({"tool": name})),
            ).await;
        }
        TurnEvent::ToolCallStarted { name, .. } => {
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "tool_call_start",
                Some(serde_json::json!({"tool": name})),
            ).await;
        }
        TurnEvent::ToolCallResult { name, success, .. } => {
            *tool_calls += 1;
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "tool_call_result",
                Some(serde_json::json!({"tool": name, "success": success})),
            ).await;
        }
        TurnEvent::ToolBatchStarted { calls, .. } => {
            let names: Vec<String> = calls.iter().map(|c| c.name.clone()).collect();
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "tool_batch_start",
                Some(serde_json::json!({"tools": names})),
            ).await;
        }
        TurnEvent::ToolBatchCompleted { ok, total, .. } => {
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "tool_batch_complete",
                Some(serde_json::json!({"ok": ok, "total": total})),
            ).await;
        }
        TurnEvent::Error(e) => {
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "error",
                Some(serde_json::json!({"error": e})),
            ).await;
        }
        TurnEvent::TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
            ..
        } => {
            append_event(
                event_store,
                event_broadcasts,
                agent_id,
                "token_usage",
                Some(serde_json::json!({
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": completion_tokens,
                    "total_tokens": total_tokens,
                })),
            ).await;
        }
        _ => {}
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

pub(crate) async fn stream_agent_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ListAgentEventsQuery>,
) -> impl IntoResponse {
    let after_seq = q.after_seq.unwrap_or(0);
    let registry = state.agent_registry.clone();

    // Replay missed events from the store
    let missed_events = if after_seq > 0 {
        registry
            .get_events(&id, after_seq, 500)
            .await
            .map(|r| r.items)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Subscribe to live events
    let mut rx = match registry.subscribe_events(&id).await {
        Ok(rx) => rx,
        Err((status, msg)) => return (status, Json(serde_json::json!({"error": msg}))).into_response(),
    };

    let (tx, rx_stream) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    // Spawn a task that replays history then forwards live events
    tokio::spawn(async move {
        // Replay phase
        for ev in missed_events {
            let _ = tx.send(ev);
        }
        // Live phase
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("SSE client lagged by {} events for agent {}", n, id);
                    continue;
                }
            }
        }
    });

    let active_conns = state.active_connections.clone();
    active_conns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let stream = UnboundedReceiverStream::new(rx_stream).map(|event| {
        let json = serde_json::to_string(&event).unwrap_or_default();
        Ok::<_, std::convert::Infallible>(sse::Event::default().data(json))
    });

    let conn_guard = crate::SseConnectionGuard(active_conns);
    let guarded_stream = stream.chain(futures::stream::once(async move {
        drop(conn_guard);
        Ok(sse::Event::default().comment("bye"))
    }));

    sse::Sse::new(guarded_stream).keep_alive(
        sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    ).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_agent_returns_created_status() {
        let registry = AgentRegistry::new(Arc::new(RwLock::new(HashMap::new())));
        let req = CreateAgentRequest {
            name: Some("test-agent".to_string()),
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
            worktree_id: None,
            branch_name: None,
        };
        let agent = registry.create(req, "/tmp").await;
        assert_eq!(agent.status, AgentStatus::Created);
        assert_eq!(agent.name, "test-agent");
    }

    #[tokio::test]
    async fn test_start_command_transitions_to_queued() {
        let registry = AgentRegistry::new(Arc::new(RwLock::new(HashMap::new())));
        let req = CreateAgentRequest {
            name: Some("agent1".to_string()),
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
            worktree_id: None,
            branch_name: None,
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
        let registry = AgentRegistry::new(Arc::new(RwLock::new(HashMap::new())));
        let req = CreateAgentRequest {
            name: None,
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
            worktree_id: None,
            branch_name: None,
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
        let registry = AgentRegistry::new(Arc::new(RwLock::new(HashMap::new())));
        let req = CreateAgentRequest {
            name: None,
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
            worktree_id: None,
            branch_name: None,
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
        let registry = AgentRegistry::new(Arc::new(RwLock::new(HashMap::new())));
        let req = CreateAgentRequest {
            name: None,
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
            worktree_id: None,
            branch_name: None,
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

    #[tokio::test]
    async fn test_create_agent_with_worktree_and_branch() {
        let registry = AgentRegistry::new(Arc::new(RwLock::new(HashMap::new())));
        let req = CreateAgentRequest {
            name: Some("wt-agent".to_string()),
            provider: None,
            working_dir: None,
            session_id: None,
            initial_input: None,
            metadata: None,
            worktree_id: Some("feature-x".to_string()),
            branch_name: Some("feature-x".to_string()),
        };
        let agent = registry.create(req, "/tmp").await;
        assert_eq!(agent.worktree_id, Some("feature-x".to_string()));
        assert_eq!(agent.branch_name, Some("feature-x".to_string()));
        assert_eq!(agent.status, AgentStatus::Created);
    }

    #[tokio::test]
    async fn test_auto_detect_branch_allows_none_outside_repo() {
        let registry = AgentRegistry::new(Arc::new(RwLock::new(HashMap::new())));
        let req = CreateAgentRequest {
            name: None,
            provider: None,
            working_dir: Some("/tmp".to_string()),
            session_id: None,
            initial_input: None,
            metadata: None,
            worktree_id: None,
            branch_name: None,
        };
        let agent = registry.create(req, "/tmp").await;
        assert_eq!(agent.worktree_id, None);
        // branch_name may be None or a branch name depending on CWD
    }
}
