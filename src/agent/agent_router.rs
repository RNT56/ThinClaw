//! Multi-agent routing with workspace isolation.
//!
//! Routes incoming messages to the correct agent based on:
//! 1. Thread ownership (first-responder wins)
//! 2. Channel binding (specific agents for specific channels)
//! 3. Mention routing (@agent_name / agent keywords)
//! 4. Default agent (fallback)
//!
//! Each agent operates in its own workspace with isolated sessions,
//! system prompts, tools, and memory.

use std::collections::HashMap;

use tokio::sync::RwLock;
use uuid::Uuid;

/// Configuration for a single agent workspace.
#[derive(Debug, Clone)]
pub struct AgentWorkspace {
    /// Unique agent identifier.
    pub agent_id: String,
    /// Display name for the agent.
    pub display_name: String,
    /// System prompt override for this agent.
    pub system_prompt: Option<String>,
    /// Channels this agent is bound to (empty = all channels).
    pub bound_channels: Vec<String>,
    /// Keywords/mentions that trigger routing to this agent.
    pub trigger_keywords: Vec<String>,
    /// Whether this is the default agent (receives unrouted messages).
    pub is_default: bool,
    /// Model override for this agent.
    pub model: Option<String>,
}

/// Routing decision: which agent should handle a message.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    /// The agent that should handle the message.
    pub agent_id: String,
    /// Reason for the routing decision.
    pub reason: RoutingReason,
}

/// Why a message was routed to a particular agent.
#[derive(Debug, Clone)]
pub enum RoutingReason {
    /// Thread is already owned by this agent.
    ThreadOwnership,
    /// Agent is bound to this channel.
    ChannelBinding,
    /// Message content matched a trigger keyword.
    KeywordMatch(String),
    /// Message explicitly mentioned the agent.
    DirectMention,
    /// No specific routing; using default agent.
    Default,
}

impl std::fmt::Display for RoutingReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ThreadOwnership => write!(f, "thread_ownership"),
            Self::ChannelBinding => write!(f, "channel_binding"),
            Self::KeywordMatch(kw) => write!(f, "keyword:{kw}"),
            Self::DirectMention => write!(f, "direct_mention"),
            Self::Default => write!(f, "default"),
        }
    }
}

/// Multi-agent router with workspace isolation.
pub struct AgentRouter {
    /// Registered agent workspaces.
    workspaces: RwLock<HashMap<String, AgentWorkspace>>,
    /// Default agent ID.
    default_agent: RwLock<Option<String>>,
    /// Thread ownership: thread UUID → agent ID.
    thread_ownership: RwLock<HashMap<Uuid, String>>,
}

impl AgentRouter {
    /// Create a new agent router.
    pub fn new() -> Self {
        Self {
            workspaces: RwLock::new(HashMap::new()),
            default_agent: RwLock::new(None),
            thread_ownership: RwLock::new(HashMap::new()),
        }
    }

    /// Register an agent workspace.
    pub async fn register_agent(&self, workspace: AgentWorkspace) {
        let agent_id = workspace.agent_id.clone();
        let is_default = workspace.is_default;

        let mut workspaces = self.workspaces.write().await;
        workspaces.insert(agent_id.clone(), workspace);

        if is_default {
            let mut default = self.default_agent.write().await;
            *default = Some(agent_id);
        }
    }

    /// Remove an agent workspace.
    pub async fn unregister_agent(&self, agent_id: &str) {
        let mut workspaces = self.workspaces.write().await;
        workspaces.remove(agent_id);

        let mut default = self.default_agent.write().await;
        if default.as_deref() == Some(agent_id) {
            *default = None;
        }
    }

    /// Route a message to the appropriate agent.
    ///
    /// Checks in order: thread ownership → direct mention → keyword match → channel binding → default.
    pub async fn route(
        &self,
        channel: &str,
        thread_id: Option<Uuid>,
        message_content: &str,
    ) -> Option<RoutingDecision> {
        // 1. Check thread ownership
        if let Some(tid) = thread_id {
            let ownership = self.thread_ownership.read().await;
            if let Some(agent_id) = ownership.get(&tid) {
                return Some(RoutingDecision {
                    agent_id: agent_id.clone(),
                    reason: RoutingReason::ThreadOwnership,
                });
            }
        }

        let workspaces = self.workspaces.read().await;

        // 2. Check for direct mentions (@agent_name)
        let content_lower = message_content.to_lowercase();
        for ws in workspaces.values() {
            let mention = format!("@{}", ws.agent_id.to_lowercase());
            if content_lower.contains(&mention) {
                return Some(RoutingDecision {
                    agent_id: ws.agent_id.clone(),
                    reason: RoutingReason::DirectMention,
                });
            }
        }

        // 3. Check keyword triggers
        for ws in workspaces.values() {
            for keyword in &ws.trigger_keywords {
                if content_lower.contains(&keyword.to_lowercase()) {
                    return Some(RoutingDecision {
                        agent_id: ws.agent_id.clone(),
                        reason: RoutingReason::KeywordMatch(keyword.clone()),
                    });
                }
            }
        }

        // 4. Check channel bindings
        for ws in workspaces.values() {
            if !ws.bound_channels.is_empty() && ws.bound_channels.contains(&channel.to_string()) {
                return Some(RoutingDecision {
                    agent_id: ws.agent_id.clone(),
                    reason: RoutingReason::ChannelBinding,
                });
            }
        }

        // 5. Default agent
        let default = self.default_agent.read().await;
        default.as_ref().map(|agent_id| RoutingDecision {
            agent_id: agent_id.clone(),
            reason: RoutingReason::Default,
        })
    }

    /// Claim thread ownership for an agent (first-responder wins).
    ///
    /// Returns `true` if ownership was claimed, `false` if already owned.
    pub async fn claim_thread(&self, thread_id: Uuid, agent_id: &str) -> bool {
        let mut ownership = self.thread_ownership.write().await;
        if ownership.contains_key(&thread_id) {
            return false;
        }
        ownership.insert(thread_id, agent_id.to_string());
        tracing::debug!(
            thread = %thread_id,
            agent = agent_id,
            "Thread ownership claimed"
        );
        true
    }

    /// Get the owner of a thread.
    pub async fn get_thread_owner(&self, thread_id: Uuid) -> Option<String> {
        let ownership = self.thread_ownership.read().await;
        ownership.get(&thread_id).cloned()
    }

    /// Transfer thread ownership to a different agent.
    ///
    /// Returns `true` if transferred, `false` if thread was not owned.
    pub async fn transfer_thread(&self, thread_id: Uuid, new_owner: &str) -> bool {
        let mut ownership = self.thread_ownership.write().await;
        if ownership.contains_key(&thread_id) {
            ownership.insert(thread_id, new_owner.to_string());
            tracing::info!(
                thread = %thread_id,
                new_owner = new_owner,
                "Thread ownership transferred"
            );
            true
        } else {
            false
        }
    }

    /// Release thread ownership.
    pub async fn release_thread(&self, thread_id: Uuid) {
        let mut ownership = self.thread_ownership.write().await;
        ownership.remove(&thread_id);
    }

    /// List all registered agents.
    pub async fn list_agents(&self) -> Vec<AgentWorkspace> {
        let workspaces = self.workspaces.read().await;
        workspaces.values().cloned().collect()
    }

    /// Get a specific agent workspace.
    pub async fn get_agent(&self, agent_id: &str) -> Option<AgentWorkspace> {
        let workspaces = self.workspaces.read().await;
        workspaces.get(agent_id).cloned()
    }

    /// Get the number of registered agents.
    pub async fn agent_count(&self) -> usize {
        let workspaces = self.workspaces.read().await;
        workspaces.len()
    }

    /// Clean up thread ownership for threads that no longer exist.
    pub async fn prune_threads(&self, active_threads: &[Uuid]) {
        let mut ownership = self.thread_ownership.write().await;
        let before = ownership.len();
        ownership.retain(|tid, _| active_threads.contains(tid));
        let pruned = before - ownership.len();
        if pruned > 0 {
            tracing::info!("Pruned {pruned} stale thread ownership entries");
        }
    }
}

impl Default for AgentRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workspace(id: &str, is_default: bool) -> AgentWorkspace {
        AgentWorkspace {
            agent_id: id.to_string(),
            display_name: id.to_string(),
            system_prompt: None,
            bound_channels: vec![],
            trigger_keywords: vec![],
            is_default,
            model: None,
        }
    }

    #[tokio::test]
    async fn test_default_routing() {
        let router = AgentRouter::new();
        router.register_agent(test_workspace("main", true)).await;

        let decision = router.route("cli", None, "hello").await;
        assert!(decision.is_some());
        assert_eq!(decision.unwrap().agent_id, "main");
    }

    #[tokio::test]
    async fn test_no_agents_returns_none() {
        let router = AgentRouter::new();
        let decision = router.route("cli", None, "hello").await;
        assert!(decision.is_none());
    }

    #[tokio::test]
    async fn test_thread_ownership_takes_priority() {
        let router = AgentRouter::new();
        router.register_agent(test_workspace("agent-a", true)).await;
        router
            .register_agent(test_workspace("agent-b", false))
            .await;

        let tid = Uuid::new_v4();
        router.claim_thread(tid, "agent-b").await;

        let decision = router.route("cli", Some(tid), "hello").await.unwrap();
        assert_eq!(decision.agent_id, "agent-b");
        assert!(matches!(decision.reason, RoutingReason::ThreadOwnership));
    }

    #[tokio::test]
    async fn test_channel_binding() {
        let router = AgentRouter::new();
        let mut ws = test_workspace("telegram-bot", false);
        ws.bound_channels = vec!["telegram".to_string()];
        router.register_agent(ws).await;
        router.register_agent(test_workspace("default", true)).await;

        let decision = router.route("telegram", None, "hello").await.unwrap();
        assert_eq!(decision.agent_id, "telegram-bot");
        assert!(matches!(decision.reason, RoutingReason::ChannelBinding));
    }

    #[tokio::test]
    async fn test_keyword_routing() {
        let router = AgentRouter::new();
        let mut ws = test_workspace("code-bot", false);
        ws.trigger_keywords = vec!["review code".to_string()];
        router.register_agent(ws).await;
        router.register_agent(test_workspace("default", true)).await;

        let decision = router
            .route("cli", None, "please review code for me")
            .await
            .unwrap();
        assert_eq!(decision.agent_id, "code-bot");
        assert!(matches!(decision.reason, RoutingReason::KeywordMatch(_)));
    }

    #[tokio::test]
    async fn test_direct_mention() {
        let router = AgentRouter::new();
        router.register_agent(test_workspace("helper", false)).await;
        router.register_agent(test_workspace("default", true)).await;

        let decision = router
            .route("cli", None, "hey @helper can you help?")
            .await
            .unwrap();
        assert_eq!(decision.agent_id, "helper");
        assert!(matches!(decision.reason, RoutingReason::DirectMention));
    }

    #[tokio::test]
    async fn test_claim_thread_first_responder_wins() {
        let router = AgentRouter::new();
        let tid = Uuid::new_v4();

        assert!(router.claim_thread(tid, "agent-a").await);
        assert!(!router.claim_thread(tid, "agent-b").await);

        assert_eq!(
            router.get_thread_owner(tid).await,
            Some("agent-a".to_string())
        );
    }

    #[tokio::test]
    async fn test_transfer_thread() {
        let router = AgentRouter::new();
        let tid = Uuid::new_v4();

        router.claim_thread(tid, "agent-a").await;
        assert!(router.transfer_thread(tid, "agent-b").await);
        assert_eq!(
            router.get_thread_owner(tid).await,
            Some("agent-b".to_string())
        );
    }

    #[tokio::test]
    async fn test_prune_threads() {
        let router = AgentRouter::new();
        let tid1 = Uuid::new_v4();
        let tid2 = Uuid::new_v4();

        router.claim_thread(tid1, "agent-a").await;
        router.claim_thread(tid2, "agent-b").await;

        router.prune_threads(&[tid1]).await;

        assert!(router.get_thread_owner(tid1).await.is_some());
        assert!(router.get_thread_owner(tid2).await.is_none());
    }

    #[tokio::test]
    async fn test_unregister_agent() {
        let router = AgentRouter::new();
        router.register_agent(test_workspace("main", true)).await;
        assert_eq!(router.agent_count().await, 1);

        router.unregister_agent("main").await;
        assert_eq!(router.agent_count().await, 0);

        // Default should be cleared
        let decision = router.route("cli", None, "hello").await;
        assert!(decision.is_none());
    }

    #[tokio::test]
    async fn test_routing_reason_display() {
        assert_eq!(
            RoutingReason::ThreadOwnership.to_string(),
            "thread_ownership"
        );
        assert_eq!(RoutingReason::Default.to_string(), "default");
        assert_eq!(
            RoutingReason::KeywordMatch("test".to_string()).to_string(),
            "keyword:test"
        );
    }
}
