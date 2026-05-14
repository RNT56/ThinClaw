//! ToolBridge — hardware sensor bridge between IronClaw agent and ThinClaw Desktop host.
//!
//! This module re-exports `ironclaw::hardware_bridge::ToolBridge` and provides
//! `TauriToolBridge`, which routes tool-execution requests through Tauri's
//! event system for user approval (3-tier: Deny / Allow Once / Allow Session).
//!
//! ## Architecture
//!
//! ```text
//! IronClaw Agent
//!   │ ToolBridge::request_approval()
//!   ▼
//! TauriToolBridge
//!   │ 1. Check session cache
//!   │ 2. If not cached → emit UiEvent::ApprovalRequested
//!   │ 3. Wait on oneshot channel
//!   ▼
//! Frontend ApprovalCard (3-tier)
//!   │ User clicks Deny / Allow Once / Allow Session
//!   ▼
//! Tauri command → resolve_approval(id, decision)
//!   │ Sends decision through oneshot
//!   ▼
//! TauriToolBridge receives decision
//!   │ If AllowSession → cache permission
//!   │ Returns decision to IronClaw
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::{debug, info, warn};

use ironclaw::hardware_bridge::{SensorRequest, SensorResponse, SensorType};

// ── Types ────────────────────────────────────────────────────────────────────

/// Approval decision from the user (3-tier model).
///
/// This replaces the old binary approve/deny with a richer model:
/// - `Deny` — reject this specific request
/// - `AllowOnce` — allow this request only
/// - `AllowSession` — allow all future requests from this tool until engine restart
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Deny this specific request.
    Deny,
    /// Allow this specific request only.
    AllowOnce,
    /// Allow all requests from this tool for the current session.
    /// Permission is cleared on engine restart.
    AllowSession,
}

/// A request from the agent to execute a bridged tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBridgeRequest {
    /// Unique request ID (for correlating with the response).
    pub request_id: String,
    /// Tool name (e.g. "screen_capture", "camera", "microphone").
    pub tool_name: String,
    /// Human-readable description of what the tool will do.
    pub description: String,
    /// Tool parameters (JSON).
    pub parameters: serde_json::Value,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

// Re-export the canonical ToolBridge trait from IronClaw.
// TauriToolBridge below implements this trait for the Tauri desktop app.
pub use ironclaw::hardware_bridge::ToolBridge;

// ── TauriToolBridge ──────────────────────────────────────────────────────────

/// Pending approval request — holds the oneshot sender for the response
/// and the tool name for session-permission caching.
struct PendingApproval {
    tx: oneshot::Sender<ApprovalDecision>,
    tool_name: String,
}

/// ThinClaw Desktop's implementation of `ToolBridge` for the Tauri desktop app.
///
/// Routes approval requests through Tauri events and maintains a
/// session-level permission cache.
pub struct TauriToolBridge {
    /// Tauri app handle for emitting events.
    app_handle: tauri::AppHandle<tauri::Wry>,
    /// Session-level permission cache: tool names that have been granted
    /// "Allow Session" permission. Cleared on engine restart.
    session_permissions: RwLock<HashSet<String>>,
    /// Pending approval requests awaiting user response.
    pending: Mutex<HashMap<String, PendingApproval>>,
}

impl std::fmt::Debug for TauriToolBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TauriToolBridge")
            .field("session_permissions", &"<RwLock>")
            .field("pending", &"<Mutex>")
            .finish()
    }
}

impl TauriToolBridge {
    /// Create a new TauriToolBridge.
    pub fn new(app_handle: tauri::AppHandle<tauri::Wry>) -> Arc<Self> {
        Arc::new(Self {
            app_handle,
            session_permissions: RwLock::new(HashSet::new()),
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// Resolve a pending approval request.
    ///
    /// Called from the `openclaw_resolve_approval` Tauri command when the
    /// user clicks a button in the ApprovalCard.
    ///
    /// Returns `true` if the request was found and resolved, `false` otherwise.
    pub async fn resolve(&self, request_id: &str, decision: ApprovalDecision) -> bool {
        let mut pending = self.pending.lock().await;
        if let Some(approval) = pending.remove(request_id) {
            // If AllowSession, cache the permission for this tool
            if decision == ApprovalDecision::AllowSession {
                let mut perms = self.session_permissions.write().await;
                perms.insert(approval.tool_name.clone());
                info!(
                    "[tool_bridge] Session permission granted for tool: {}",
                    approval.tool_name
                );
            }

            debug!(
                "[tool_bridge] Resolved approval {} with {:?}",
                request_id, decision
            );
            let _ = approval.tx.send(decision);
            true
        } else {
            warn!(
                "[tool_bridge] No pending approval for request_id: {}",
                request_id
            );
            false
        }
    }

    /// Clear all session-level permissions (called on engine stop/restart).
    pub async fn clear_session_permissions(&self) {
        let mut perms = self.session_permissions.write().await;
        let count = perms.len();
        perms.clear();
        if count > 0 {
            info!("[tool_bridge] Cleared {} session permissions", count);
        }
    }

    /// Check if a tool has session-level permission.
    pub async fn has_session_permission(&self, tool_name: &str) -> bool {
        self.session_permissions.read().await.contains(tool_name)
    }

    /// Get a list of tools with session-level permissions (for diagnostics).
    pub async fn session_permissions_list(&self) -> Vec<String> {
        self.session_permissions
            .read()
            .await
            .iter()
            .cloned()
            .collect()
    }
}

#[async_trait]
impl ToolBridge for TauriToolBridge {
    /// Request access to a sensor.
    ///
    /// Checks session permission cache first; if the sensor is pre-approved,
    /// returns a denied response with a note that the tool-level approval was
    /// cached (actual sensor capture is handled by the host).
    ///
    /// Otherwise emits an `ApprovalRequested` event to the frontend, waits for
    /// the user's decision (5-minute timeout), and returns the result.
    async fn request_sensor_access(&self, request: SensorRequest) -> SensorResponse {
        let sensor_name = request.sensor.to_string().to_lowercase();

        // Check session permission cache
        if self.session_permissions.read().await.contains(&sensor_name) {
            debug!(
                "[tool_bridge] Sensor '{}' has session permission, auto-approving",
                sensor_name
            );
            return SensorResponse::ok(serde_json::json!({
                "auto_approved": true,
                "sensor": sensor_name,
            }))
            .with_session_approval();
        }

        // Create oneshot channel for the response
        let (tx, rx) = oneshot::channel();
        let request_id = request.id.clone();

        // Register pending approval
        {
            let mut pending = self.pending.lock().await;
            pending.insert(
                request_id.clone(),
                PendingApproval {
                    tx,
                    tool_name: sensor_name.clone(),
                },
            );
        }

        // Emit the approval request to the frontend via Tauri event
        use tauri::Emitter;
        let event = super::ui_types::UiEvent::ApprovalRequested {
            approval_id: request_id.clone(),
            session_key: "agent:main".to_string(),
            tool_name: sensor_name.clone(),
            input: request.params.clone(),
        };

        if let Err(e) = self.app_handle.emit("openclaw-event", &event) {
            warn!("[tool_bridge] Failed to emit approval event: {}", e);
            self.pending.lock().await.remove(&request_id);
            return SensorResponse::denied(format!("Failed to request approval: {}", e));
        }

        info!(
            "[tool_bridge] Sensor access requested: {} (id: {})",
            sensor_name, request_id
        );

        // Wait for the user's response (or timeout after 5 minutes)
        match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
            Ok(Ok(decision)) => match decision {
                ApprovalDecision::AllowOnce => SensorResponse::ok(serde_json::json!({
                    "approved": true,
                    "sensor": sensor_name,
                })),
                ApprovalDecision::AllowSession => {
                    // Session permission is already cached by resolve()
                    SensorResponse::ok(serde_json::json!({
                        "approved": true,
                        "sensor": sensor_name,
                    }))
                    .with_session_approval()
                }
                ApprovalDecision::Deny => SensorResponse::denied("User denied sensor access"),
            },
            Ok(Err(_)) => {
                warn!("[tool_bridge] Approval channel closed for {}", request_id);
                SensorResponse::denied("Approval channel closed")
            }
            Err(_) => {
                warn!(
                    "[tool_bridge] Sensor access timed out after 5 minutes for {}",
                    request_id
                );
                SensorResponse::denied("Sensor access request timed out")
            }
        }
    }

    /// Check if a sensor type has session-level approval.
    async fn is_session_approved(&self, sensor: SensorType) -> bool {
        let sensor_name = sensor.to_string().to_lowercase();
        self.session_permissions.read().await.contains(&sensor_name)
    }
}

// ── Map ApprovalDecision to IronClaw's resolve_approval params ───────────────

impl ApprovalDecision {
    /// Convert to the `(approved, always)` pair used by
    /// `ironclaw::api::chat::resolve_approval()`.
    pub fn to_ironclaw_params(self) -> (bool, bool) {
        match self {
            ApprovalDecision::Deny => (false, false),
            ApprovalDecision::AllowOnce => (true, false),
            ApprovalDecision::AllowSession => (true, true),
        }
    }

    /// Create from `(approved, allow_session)` — used when receiving
    /// from the frontend.
    pub fn from_frontend(approved: bool, allow_session: bool) -> Self {
        if !approved {
            ApprovalDecision::Deny
        } else if allow_session {
            ApprovalDecision::AllowSession
        } else {
            ApprovalDecision::AllowOnce
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_to_ironclaw_params() {
        assert_eq!(ApprovalDecision::Deny.to_ironclaw_params(), (false, false));
        assert_eq!(
            ApprovalDecision::AllowOnce.to_ironclaw_params(),
            (true, false)
        );
        assert_eq!(
            ApprovalDecision::AllowSession.to_ironclaw_params(),
            (true, true)
        );
    }

    #[test]
    fn test_decision_from_frontend() {
        assert_eq!(
            ApprovalDecision::from_frontend(false, false),
            ApprovalDecision::Deny
        );
        assert_eq!(
            ApprovalDecision::from_frontend(false, true),
            ApprovalDecision::Deny
        ); // deny overrides
        assert_eq!(
            ApprovalDecision::from_frontend(true, false),
            ApprovalDecision::AllowOnce
        );
        assert_eq!(
            ApprovalDecision::from_frontend(true, true),
            ApprovalDecision::AllowSession
        );
    }
}
