use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};

use super::commands::AgentEvent;
use super::traits::{AgentId, AgentState};

pub struct ResourceManager {
    states: Arc<RwLock<HashMap<AgentId, Arc<RwLock<AgentState>>>>>,
    next_id: AtomicU64,
    event_tx: broadcast::Sender<AgentEvent>,
}

#[derive(Clone)]
pub struct AgentHandle {
    pub id: AgentId,
}

impl ResourceManager {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            states: Arc::new(RwLock::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            event_tx,
        }
    }

    /// Allocate a new AgentId.
    pub fn next_id(&self) -> AgentId {
        AgentId(self.next_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Register an agent's state in the registry.
    pub async fn register(&self, id: AgentId, state: Arc<RwLock<AgentState>>) {
        self.states.write().await.insert(id, state);
    }

    /// Remove an agent from the registry.
    pub async fn unregister(&self, id: AgentId) {
        self.states.write().await.remove(&id);
    }

    /// Emit an event to all subscribers.
    pub fn emit(&self, event: AgentEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Subscribe to agent events.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.event_tx.subscribe()
    }

    /// Get a snapshot of an agent's state.
    pub async fn snapshot(&self, id: AgentId) -> Option<AgentState> {
        let states = self.states.read().await;
        states.get(&id).map(|s| s.blocking_read().clone())
    }

    /// Get snapshots of all agents.
    pub async fn snapshots(&self) -> Vec<AgentState> {
        let states = self.states.read().await;
        states.values().map(|s| s.blocking_read().clone()).collect()
    }

    /// Lookup an agent's state lock.
    pub async fn get_state(&self, id: AgentId) -> Option<Arc<RwLock<AgentState>>> {
        self.states.read().await.get(&id).cloned()
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}
