//! Hardware Bridge — in-process sensor access for desktop mode.
//!
//! When ThinClaw runs as a library inside Scrappy (Tauri), sensor access
//! (camera, microphone, screen) is handled by the host application rather
//! than by ThinClaw directly.
//!
//! Instead of WebSocket RPC (the original design for remote orchestrators),
//! the bridge uses a simple async Rust trait that Scrappy implements and
//! injects at startup. This is simpler and faster since ThinClaw is in-process.
//!
//! Architecture:
//! ```text
//! LLM calls "capture_camera" tool
//!   → BridgedTool::call()
//!     → ToolBridge::request_sensor_access()
//!       → Scrappy shows ApprovalCard (Approve/Deny/Allow Session)
//!       → If approved: Scrappy captures via native API
//!       → Returns SensorResponse to ThinClaw
//!     → BridgedTool returns result to LLM
//! ```
//!
//! Security model:
//! - Every sensor access requires explicit user approval
//! - Three approval tiers: Deny / Allow Once / Allow Session
//! - Session approvals expire when the app restarts
//! - Reason string (from LLM) is shown in the approval dialog
//! - 30-second timeout on all bridge requests

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Sensor types that can be accessed via the hardware bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SensorType {
    /// Camera (webcam) capture.
    Camera,
    /// Microphone audio recording.
    Microphone,
    /// Screen capture (screenshot).
    Screen,
}

impl std::fmt::Display for SensorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SensorType::Camera => write!(f, "Camera"),
            SensorType::Microphone => write!(f, "Microphone"),
            SensorType::Screen => write!(f, "Screen"),
        }
    }
}

/// A request to access a sensor via the hardware bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorRequest {
    /// Unique request ID for correlation.
    pub id: String,
    /// Which sensor to access.
    pub sensor: SensorType,
    /// The specific action to perform.
    pub action: String,
    /// Action-specific parameters (JSON object).
    pub params: serde_json::Value,
    /// Human-readable reason from the LLM/tool.
    /// Shown in the approval dialog so the user understands *why*.
    pub reason: String,
}

/// How a sensor access request was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SensorResponseKind {
    /// Request succeeded — sensor data is available.
    Success,
    /// User explicitly denied the request.
    Denied,
    /// A system/hardware error prevented access.
    Error,
}

/// Response from a sensor access request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorResponse {
    /// Whether the request was approved and succeeded.
    pub success: bool,
    /// How the request was resolved (success / user denial / system error).
    pub kind: SensorResponseKind,
    /// The sensor data (e.g., base64-encoded image, audio bytes).
    /// None if denied or failed.
    pub data: Option<serde_json::Value>,
    /// Error or denial message.
    pub error: Option<String>,
    /// Whether the user granted session-level approval.
    pub session_approved: bool,
}

impl SensorResponse {
    /// Create a successful response.
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            success: true,
            kind: SensorResponseKind::Success,
            data: Some(data),
            error: None,
            session_approved: false,
        }
    }

    /// Create a denied response (user explicitly refused access).
    ///
    /// Structurally identical to [`error()`](Self::error) but uses
    /// [`SensorResponseKind::Denied`] so the host application (Scrappy/Tauri)
    /// can show "permission denied" UI instead of a system error dialog.
    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            success: false,
            kind: SensorResponseKind::Denied,
            data: None,
            error: Some(message.into()),
            session_approved: false,
        }
    }

    /// Create an error response (system/hardware failure, not user denial).
    ///
    /// Structurally identical to [`denied()`](Self::denied) but uses
    /// [`SensorResponseKind::Error`] so the host application (Scrappy/Tauri)
    /// can show a system error dialog with retry options instead of a
    /// permission-denied message.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            kind: SensorResponseKind::Error,
            data: None,
            error: Some(message.into()),
            session_approved: false,
        }
    }

    /// Mark as session-approved.
    pub fn with_session_approval(mut self) -> Self {
        self.session_approved = true;
        self
    }
}

/// The bridge trait that Scrappy (or any host) implements to provide
/// hardware sensor access to ThinClaw.
///
/// Scrappy implements this by:
/// 1. Showing its `ApprovalCard` component
/// 2. If approved, capturing via the appropriate native API
/// 3. Returning the sensor data
#[async_trait]
pub trait ToolBridge: Send + Sync + Debug {
    /// Request access to a sensor.
    ///
    /// The implementor should:
    /// 1. Show an approval dialog with the `reason` string
    /// 2. If approved, execute the sensor action
    /// 3. Return the result
    ///
    /// The 30-second timeout is enforced by the caller (`BridgedTool`),
    /// but implementors should also handle their own timeouts gracefully.
    async fn request_sensor_access(&self, request: SensorRequest) -> SensorResponse;

    /// Check if a sensor type has been pre-approved for this session.
    ///
    /// If true, `request_sensor_access` should skip the approval dialog.
    async fn is_session_approved(&self, sensor: SensorType) -> bool;
}

/// In-memory session approval tracker.
///
/// Tracks which sensors have been approved for the current session.
/// Cleared on app restart (not persisted).
#[derive(Debug, Default)]
pub struct SessionApprovals {
    approved: RwLock<HashMap<SensorType, bool>>,
}

impl SessionApprovals {
    /// Create a new empty approval tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a sensor is session-approved.
    pub async fn is_approved(&self, sensor: SensorType) -> bool {
        self.approved
            .read()
            .await
            .get(&sensor)
            .copied()
            .unwrap_or(false)
    }

    /// Grant session approval for a sensor.
    pub async fn approve(&self, sensor: SensorType) {
        self.approved.write().await.insert(sensor, true);
    }

    /// Revoke session approval for a sensor.
    #[allow(dead_code)]
    pub async fn revoke(&self, sensor: SensorType) {
        self.approved.write().await.remove(&sensor);
    }

    /// Clear all session approvals.
    #[allow(dead_code)]
    pub async fn clear(&self) {
        self.approved.write().await.clear();
    }
}

/// A tool wrapper that delegates sensor access through the hardware bridge.
///
/// Implements the `Tool` trait by forwarding calls to the `ToolBridge` with
/// a 30-second timeout and session approval caching.
pub struct BridgedTool {
    /// The bridge implementation (provided by Scrappy).
    bridge: Arc<dyn ToolBridge>,
    /// Session approval cache.
    approvals: Arc<SessionApprovals>,
    /// What sensor this tool accesses.
    sensor: SensorType,
    /// The action name (e.g., "capture_camera_frame").
    action: String,
    /// Human-readable tool description.
    description: String,
    /// Timeout for bridge requests.
    timeout: Duration,
}

impl BridgedTool {
    /// Create a new bridged tool.
    pub fn new(
        bridge: Arc<dyn ToolBridge>,
        approvals: Arc<SessionApprovals>,
        sensor: SensorType,
        action: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            bridge,
            approvals,
            sensor,
            action: action.into(),
            description: description.into(),
            timeout: Duration::from_secs(30),
        }
    }

    /// Set a custom timeout.
    #[allow(dead_code)]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Execute a sensor access request through the bridge.
    pub async fn call(
        &self,
        params: serde_json::Value,
        reason: String,
    ) -> Result<serde_json::Value, String> {
        let request = SensorRequest {
            id: uuid::Uuid::new_v4().to_string(),
            sensor: self.sensor,
            action: self.action.clone(),
            params,
            reason,
        };

        tracing::info!(
            sensor = %self.sensor,
            action = %self.action,
            "Hardware bridge request"
        );

        // Execute with timeout
        let result = tokio::time::timeout(self.timeout, async {
            self.bridge.request_sensor_access(request).await
        })
        .await;

        match result {
            Ok(response) => {
                // Cache session approval if granted
                if response.session_approved {
                    self.approvals.approve(self.sensor).await;
                    tracing::info!(
                        sensor = %self.sensor,
                        "Session approval granted for sensor"
                    );
                }

                if response.success {
                    response
                        .data
                        .ok_or_else(|| "No data in response".to_string())
                } else {
                    let message = response
                        .error
                        .unwrap_or_else(|| "Unknown bridge error".to_string());

                    match response.kind {
                        SensorResponseKind::Denied => {
                            Err(format!("Hardware bridge request denied: {message}"))
                        }
                        SensorResponseKind::Error => {
                            Err(format!("Hardware bridge request failed: {message}"))
                        }
                        SensorResponseKind::Success => {
                            Err(format!("Hardware bridge request failed: {message}"))
                        }
                    }
                }
            }
            Err(_) => {
                tracing::warn!(
                    sensor = %self.sensor,
                    action = %self.action,
                    timeout_secs = self.timeout.as_secs(),
                    "Hardware bridge request timed out"
                );
                Err(format!(
                    "Hardware bridge request timed out after {}s. The user may not have responded to the approval dialog.",
                    self.timeout.as_secs()
                ))
            }
        }
    }

    /// Get the sensor type.
    pub fn sensor(&self) -> SensorType {
        self.sensor
    }

    /// Get the action name.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Get the description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Check if this sensor is session-approved.
    pub async fn is_session_approved(&self) -> bool {
        self.approvals.is_approved(self.sensor).await
    }
}

impl Debug for BridgedTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgedTool")
            .field("sensor", &self.sensor)
            .field("action", &self.action)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Tool trait implementation so BridgedTool can be registered in the ToolRegistry
/// and invoked by the LLM like any other tool.
///
/// The LLM should provide a `reason` parameter explaining why it needs sensor access.
/// This reason is shown in the user's approval dialog.
///
/// Parameters schema:
/// ```json
/// {
///   "type": "object",
///   "properties": {
///     "reason": { "type": "string", "description": "Why you need this sensor data" }
///   },
///   "required": ["reason"]
/// }
/// ```
use crate::tools::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

#[async_trait]
impl Tool for BridgedTool {
    fn name(&self) -> &str {
        &self.action
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "A brief explanation of why you need this sensor data. This will be shown to the user in an approval dialog."
                }
            },
            "required": ["reason"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &crate::context::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let reason = params
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Agent requested sensor access")
            .to_string();

        let start = std::time::Instant::now();

        match self.call(params.clone(), reason).await {
            Ok(data) => Ok(ToolOutput::success(data, start.elapsed())),
            Err(e) => Err(ToolError::ExecutionFailed(e)),
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Sensor access always requires explicit user approval.
        // The bridge's own approval dialog handles the actual consent flow,
        // but we mark it here so the orchestrator knows this tool has side effects.
        ApprovalRequirement::Always
    }

    fn execution_timeout(&self) -> Duration {
        // Match the bridge timeout (30s default) + buffer for the approval dialog
        self.timeout + Duration::from_secs(5)
    }

    fn requires_sanitization(&self) -> bool {
        false // Bridge responses are trusted (from local host app)
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

/// Create the standard set of bridged sensor tools.
///
/// Returns three `BridgedTool` instances for camera, microphone, and screen.
/// These should be registered in the `ToolRegistry` when a bridge is available.
pub fn create_bridged_tools(
    bridge: Arc<dyn ToolBridge>,
    approvals: Arc<SessionApprovals>,
) -> Vec<BridgedTool> {
    vec![
        BridgedTool::new(
            Arc::clone(&bridge),
            Arc::clone(&approvals),
            SensorType::Camera,
            "capture_camera_frame",
            "Capture a single frame from the user's webcam. Requires user approval.",
        ),
        BridgedTool::new(
            Arc::clone(&bridge),
            Arc::clone(&approvals),
            SensorType::Microphone,
            "record_audio_clip",
            "Record a short audio clip from the user's microphone. Requires user approval.",
        ),
        BridgedTool::new(
            bridge,
            approvals,
            SensorType::Screen,
            "capture_screenshot",
            "Capture a screenshot of the user's screen. Requires user approval.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock bridge for testing.
    #[derive(Debug)]
    struct MockBridge {
        response: SensorResponse,
    }

    #[async_trait]
    impl ToolBridge for MockBridge {
        async fn request_sensor_access(&self, _request: SensorRequest) -> SensorResponse {
            self.response.clone()
        }

        async fn is_session_approved(&self, _sensor: SensorType) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn test_bridged_tool_success() {
        let bridge = Arc::new(MockBridge {
            response: SensorResponse::ok(serde_json::json!({
                "format": "jpeg",
                "base64": "dGVzdA=="
            })),
        });
        let approvals = Arc::new(SessionApprovals::new());
        let tool = BridgedTool::new(
            bridge,
            approvals,
            SensorType::Camera,
            "capture_camera_frame",
            "Test camera tool",
        );

        let result = tool
            .call(serde_json::json!({}), "Testing".to_string())
            .await;
        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(data["format"], "jpeg");
    }

    #[tokio::test]
    async fn test_bridged_tool_denied() {
        let bridge = Arc::new(MockBridge {
            response: SensorResponse::denied("User denied camera access"),
        });
        let approvals = Arc::new(SessionApprovals::new());
        let tool = BridgedTool::new(
            bridge,
            approvals,
            SensorType::Camera,
            "capture_camera_frame",
            "Test camera tool",
        );

        let result = tool
            .call(serde_json::json!({}), "Testing".to_string())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("denied"));
    }

    #[tokio::test]
    async fn test_bridged_tool_system_error() {
        let bridge = Arc::new(MockBridge {
            response: SensorResponse::error("Camera unavailable"),
        });
        let approvals = Arc::new(SessionApprovals::new());
        let tool = BridgedTool::new(
            bridge,
            approvals,
            SensorType::Camera,
            "capture_camera_frame",
            "Test camera tool",
        );

        let result = tool
            .call(serde_json::json!({}), "Testing".to_string())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed"));
    }

    #[tokio::test]
    async fn test_session_approvals() {
        let approvals = SessionApprovals::new();
        assert!(!approvals.is_approved(SensorType::Camera).await);

        approvals.approve(SensorType::Camera).await;
        assert!(approvals.is_approved(SensorType::Camera).await);
        assert!(!approvals.is_approved(SensorType::Microphone).await);

        approvals.revoke(SensorType::Camera).await;
        assert!(!approvals.is_approved(SensorType::Camera).await);
    }

    #[tokio::test]
    async fn test_session_approval_caching() {
        let response =
            SensorResponse::ok(serde_json::json!({"test": true})).with_session_approval();
        let bridge = Arc::new(MockBridge { response });
        let approvals = Arc::new(SessionApprovals::new());
        let tool = BridgedTool::new(
            bridge,
            Arc::clone(&approvals),
            SensorType::Screen,
            "capture_screenshot",
            "Test screen tool",
        );

        assert!(!tool.is_session_approved().await);
        let _ = tool.call(serde_json::json!({}), "Test".to_string()).await;
        assert!(tool.is_session_approved().await);
    }

    #[tokio::test]
    async fn test_bridged_tool_timeout() {
        #[derive(Debug)]
        struct SlowBridge;

        #[async_trait]
        impl ToolBridge for SlowBridge {
            async fn request_sensor_access(&self, _request: SensorRequest) -> SensorResponse {
                tokio::time::sleep(Duration::from_secs(5)).await;
                SensorResponse::ok(serde_json::json!({}))
            }
            async fn is_session_approved(&self, _sensor: SensorType) -> bool {
                false
            }
        }

        let bridge: Arc<dyn ToolBridge> = Arc::new(SlowBridge);
        let approvals = Arc::new(SessionApprovals::new());
        let tool = BridgedTool::new(
            bridge,
            approvals,
            SensorType::Camera,
            "capture_camera_frame",
            "Test",
        )
        .with_timeout(Duration::from_millis(100));

        let result = tool.call(serde_json::json!({}), "Test".to_string()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timed out"));
    }

    #[test]
    fn test_create_bridged_tools() {
        #[derive(Debug)]
        struct NoopBridge;

        #[async_trait]
        impl ToolBridge for NoopBridge {
            async fn request_sensor_access(&self, _: SensorRequest) -> SensorResponse {
                SensorResponse::denied("noop")
            }
            async fn is_session_approved(&self, _: SensorType) -> bool {
                false
            }
        }

        let bridge: Arc<dyn ToolBridge> = Arc::new(NoopBridge);
        let approvals = Arc::new(SessionApprovals::new());
        let tools = create_bridged_tools(bridge, approvals);
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].sensor(), SensorType::Camera);
        assert_eq!(tools[1].sensor(), SensorType::Microphone);
        assert_eq!(tools[2].sensor(), SensorType::Screen);
    }

    #[test]
    fn test_sensor_type_display() {
        assert_eq!(SensorType::Camera.to_string(), "Camera");
        assert_eq!(SensorType::Microphone.to_string(), "Microphone");
        assert_eq!(SensorType::Screen.to_string(), "Screen");
    }
}
