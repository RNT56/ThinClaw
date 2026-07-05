//! Request and response DTOs for the web gateway API.

pub use crate::api::experiments::{
    ExperimentArtifactListResponse, ExperimentCampaignActionResponse,
    ExperimentCampaignListResponse, ExperimentGpuCloudProviderInfo,
    ExperimentGpuCloudProviderListResponse, ExperimentLaunchDetails,
    ExperimentLeaseCredentialsResponse, ExperimentLeaseJobResponse,
    ExperimentModelUsageListResponse, ExperimentOpportunityListResponse,
    ExperimentProjectListResponse, ExperimentRunnerListResponse,
    ExperimentRunnerValidationResponse, ExperimentTargetListResponse, ExperimentTrialListResponse,
};
pub use crate::api::learning::{
    LearningArtifactVersionItem, LearningArtifactVersionResponse, LearningCandidateItem,
    LearningCandidateResponse, LearningCodeProposalItem, LearningCodeProposalResponse,
    LearningCodeProposalReviewRequest, LearningCodeProposalReviewResponse, LearningEvaluationItem,
    LearningEventItem, LearningFeedbackActionResponse, LearningFeedbackItem,
    LearningFeedbackRequest, LearningFeedbackResponse, LearningHistoryResponse, LearningListQuery,
    LearningProviderHealthItem, LearningProviderHealthResponse, LearningProviderHealthSummary,
    LearningRecentCounts, LearningRollbackActionResponse, LearningRollbackItem,
    LearningRollbackRequest, LearningRollbackResponse, LearningStatusResponse,
};
pub use crate::api::mcp::{
    McpInteractionListResponse, McpInteractionRespondRequest, McpLogLevelRequest,
    McpOAuthDiscoveryResponse, McpPromptRequest, McpPromptResponse, McpPromptsResponse,
    McpReadResourceQuery, McpReadResourceResponse, McpResourceTemplatesResponse,
    McpResourcesResponse, McpServerInfo, McpServerListResponse, McpToolsResponse,
};
pub use thinclaw_gateway::web::types::{
    ActionResponse, ApprovalRequest, AuthCancelRequest, AuthTokenRequest, AutonomyPauseRequest,
    AutonomyPauseResponse, CacheStatsResponse, ChannelSetupStatus,
    ExperimentGpuCloudConnectRequest, ExperimentGpuCloudLaunchTestRequest,
    ExperimentGpuCloudTemplateRequest, ExperimentsLimitQuery, ExperimentsQuery, ExtensionInfo,
    ExtensionListResponse, ExtensionSetupRequest, ExtensionSetupResponse, FilePathQuery,
    GatewayStatusResponse, HealthResponse, HistoryQuery, HistoryResponse, HookInfo,
    HookListResponse, HookRegisterRequest, HookRegisterResponse, HookUnregisterResponse,
    InstallExtensionRequest, JobDetailResponse, JobInfo, JobListResponse, JobSummaryResponse,
    ListEntry, ListQuery, LogLevelRequest, LogLevelResponse, LogsRecentResponse,
    MemoryDeleteRequest, MemoryDeleteResponse, MemoryListResponse, MemoryReadResponse,
    MemorySearchRequest, MemorySearchResponse, MemoryTreeResponse, MemoryWriteRequest,
    MemoryWriteResponse, ModelInfo, ModelUsageEntry, NostrPrivateKeyRequest, PairingApproveRequest,
    PairingApprovedInfo, PairingListResponse, PairingRequestInfo, PartialChannelSetupStatus,
    PendingApprovalEntry, PendingApprovalsResponse, ProjectFileEntry, ProjectFileReadResponse,
    ProjectFilesResponse, ReadQuery, RegistryEntryInfo, RegistrySearchQuery,
    RegistrySearchResponse, ResponseAttachment, RoutineClearRunsRequest, RoutineCreateRequest,
    RoutineDetailResponse, RoutineEventActivityInfo, RoutineEventActivityResponse,
    RoutineEventCheckInfo, RoutineInfo, RoutineListResponse, RoutineRunInfo,
    RoutineSummaryResponse, RoutineTriggerCheckInfo, SearchHit, SecretFieldInfo,
    SendMessageRequest, SendMessageResponse, SettingResponse, SettingWriteRequest,
    SettingsExportResponse, SettingsImportRequest, SettingsListResponse, SkillCatalogSearchResult,
    SkillInfo, SkillInspectRequest, SkillInstallRequest, SkillListResponse, SkillPublishRequest,
    SkillSearchRequest, SkillSearchResponse, SkillTapAddRequest, SkillTapRefreshRequest,
    SkillTapRemoveRequest, SkillTrustRequest, SseEvent, ThreadCommandRequest,
    ThreadCommandResponse, ThreadExportQuery, ThreadExportResponse, ThreadInfo, ThreadListResponse,
    ToggleRequest, ToolCallInfo, ToolInfo, ToolListResponse, TransitionInfo, TreeEntry, TreeQuery,
    TurnInfo, WsClientMessage, WsServerMessage,
};

// --- Autonomy ---

pub type AutonomyStatusResponse = crate::desktop_autonomy::AutonomyStatus;
pub type AutonomyBootstrapResponse = crate::desktop_autonomy::AutonomyBootstrapReport;
pub type AutonomyRolloutsResponse = crate::desktop_autonomy::AutonomyRolloutSummary;
pub type AutonomyChecksResponse = crate::desktop_autonomy::AutonomyChecksSummary;
pub type AutonomyEvidenceResponse = crate::desktop_autonomy::AutonomyEvidenceSummary;

#[cfg(test)]
mod tests {
    use super::*;

    // ---- WsClientMessage deserialization tests ----

    #[test]
    fn test_ws_client_message_parse() {
        let json = r#"{"type":"message","content":"hello","thread_id":"t1"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Message { content, thread_id } => {
                assert_eq!(content, "hello");
                assert_eq!(thread_id.as_deref(), Some("t1"));
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_ws_client_message_no_thread() {
        let json = r#"{"type":"message","content":"hi"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Message { content, thread_id } => {
                assert_eq!(content, "hi");
                assert!(thread_id.is_none());
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_ws_client_approval_parse() {
        let json =
            r#"{"type":"approval","request_id":"abc-123","action":"approve","thread_id":"t1"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Approval {
                request_id,
                action,
                thread_id,
            } => {
                assert_eq!(request_id, "abc-123");
                assert_eq!(action, "approve");
                assert_eq!(thread_id.as_deref(), Some("t1"));
            }
            _ => panic!("Expected Approval variant"),
        }
    }

    #[test]
    fn test_ws_client_approval_parse_no_thread() {
        let json = r#"{"type":"approval","request_id":"abc-123","action":"deny"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Approval {
                request_id,
                action,
                thread_id,
            } => {
                assert_eq!(request_id, "abc-123");
                assert_eq!(action, "deny");
                assert!(thread_id.is_none());
            }
            _ => panic!("Expected Approval variant"),
        }
    }

    #[test]
    fn test_ws_client_ping_parse() {
        let json = r#"{"type":"ping"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, WsClientMessage::Ping));
    }

    #[test]
    fn test_ws_client_unknown_type_fails() {
        let json = r#"{"type":"unknown"}"#;
        let result: Result<WsClientMessage, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // ---- WsServerMessage serialization tests ----

    #[test]
    fn test_ws_server_pong_serialize() {
        let msg = WsServerMessage::Pong;
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"pong"}"#);
    }

    #[test]
    fn test_ws_server_error_serialize() {
        let msg = WsServerMessage::Error {
            message: "bad request".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["message"], "bad request");
    }

    #[test]
    fn test_ws_server_from_sse_response() {
        let sse = SseEvent::Response {
            content: "hello".to_string(),
            thread_id: "t1".to_string(),
            attachments: Vec::new(),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "response");
                assert_eq!(data["content"], "hello");
                assert_eq!(data["thread_id"], "t1");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_sse_conversation_updated_serialize() {
        let event = SseEvent::ConversationUpdated {
            thread_id: "thread-9".to_string(),
            reason: "user_message".to_string(),
            channel: Some("telegram".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "conversation_updated");
        assert_eq!(parsed["thread_id"], "thread-9");
        assert_eq!(parsed["reason"], "user_message");
        assert_eq!(parsed["channel"], "telegram");
    }

    #[test]
    fn test_sse_conversation_deleted_omits_identity_fields() {
        let event = SseEvent::ConversationDeleted {
            thread_id: "thread-7".to_string(),
            principal_id: "user-1".to_string(),
            actor_id: "actor-1".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "conversation_deleted");
        assert_eq!(parsed["thread_id"], "thread-7");
        assert!(parsed.get("principal_id").is_none());
        assert!(parsed.get("actor_id").is_none());
    }

    #[test]
    fn test_ws_server_from_sse_thinking() {
        let sse = SseEvent::Thinking {
            message: "reasoning...".to_string(),
            thread_id: None,
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "thinking");
                assert_eq!(data["message"], "reasoning...");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_conversation_updated() {
        let sse = SseEvent::ConversationUpdated {
            thread_id: "t2".to_string(),
            reason: "assistant_response".to_string(),
            channel: Some("repl".to_string()),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "conversation_updated");
                assert_eq!(data["thread_id"], "t2");
                assert_eq!(data["reason"], "assistant_response");
                assert_eq!(data["channel"], "repl");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_sse_subagent_spawned_serialize() {
        let event = SseEvent::SubagentSpawned {
            agent_id: "agent-1".to_string(),
            name: "researcher".to_string(),
            task: "Check docs".to_string(),
            task_packet: crate::agent::subagent_executor::SubagentTaskPacket {
                objective: "Check docs".to_string(),
                ..Default::default()
            },
            allowed_tools: vec!["read_file".to_string()],
            allowed_skills: vec![],
            memory_mode: "provided_context_only".to_string(),
            tool_mode: "explicit_only".to_string(),
            skill_mode: "explicit_only".to_string(),
            timestamp: "2026-04-12T12:00:00Z".to_string(),
            thread_id: Some("thread-1".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "subagent_spawned");
        assert_eq!(parsed["agent_id"], "agent-1");
        assert_eq!(parsed["name"], "researcher");
        assert_eq!(parsed["task"], "Check docs");
        assert_eq!(parsed["task_packet"]["objective"], "Check docs");
        assert_eq!(parsed["allowed_tools"][0], "read_file");
        assert_eq!(parsed["timestamp"], "2026-04-12T12:00:00Z");
        assert_eq!(parsed["thread_id"], "thread-1");
    }

    #[test]
    fn test_sse_subagent_completed_serialize() {
        let event = SseEvent::SubagentCompleted {
            agent_id: "agent-2".to_string(),
            name: "summarizer".to_string(),
            success: true,
            response: "Done".to_string(),
            duration_ms: 1250,
            iterations: 3,
            task_packet: crate::agent::subagent_executor::SubagentTaskPacket {
                objective: "Summarize findings".to_string(),
                ..Default::default()
            },
            allowed_tools: vec![],
            allowed_skills: vec![],
            memory_mode: "provided_context_only".to_string(),
            tool_mode: "explicit_only".to_string(),
            skill_mode: "explicit_only".to_string(),
            timestamp: "2026-04-12T12:00:03Z".to_string(),
            thread_id: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "subagent_completed");
        assert_eq!(parsed["agent_id"], "agent-2");
        assert_eq!(parsed["response"], "Done");
        assert_eq!(parsed["duration_ms"], 1250);
        assert_eq!(parsed["iterations"], 3);
        assert_eq!(parsed["timestamp"], "2026-04-12T12:00:03Z");
        assert!(parsed.get("thread_id").is_none());
    }

    #[test]
    fn test_send_message_request_accepts_legacy_message_field() {
        let json = r#"{"message":"hello","user_id":"family-1"}"#;
        let req: SendMessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "hello");
        assert_eq!(req.user_id.as_deref(), Some("family-1"));
        assert!(req.thread_id.is_none());
    }

    #[test]
    fn test_ws_server_from_sse_approval_needed() {
        let sse = SseEvent::ApprovalNeeded {
            request_id: "r1".to_string(),
            tool_name: "shell".to_string(),
            description: "Run ls".to_string(),
            parameters: "{}".to_string(),
            thread_id: Some("t1".to_string()),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "approval_needed");
                assert_eq!(data["tool_name"], "shell");
                assert_eq!(data["thread_id"], "t1");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_subagent_progress() {
        let sse = SseEvent::SubagentProgress {
            agent_id: "agent-3".to_string(),
            message: "Inspecting files".to_string(),
            category: "tool".to_string(),
            timestamp: "2026-04-12T12:00:01Z".to_string(),
            thread_id: Some("thread-2".to_string()),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "subagent_progress");
                assert_eq!(data["agent_id"], "agent-3");
                assert_eq!(data["message"], "Inspecting files");
                assert_eq!(data["category"], "tool");
                assert_eq!(data["timestamp"], "2026-04-12T12:00:01Z");
                assert_eq!(data["thread_id"], "thread-2");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_heartbeat() {
        let sse = SseEvent::Heartbeat;
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, .. } => {
                assert_eq!(event_type, "heartbeat");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    // ---- Auth type tests ----

    #[test]
    fn test_ws_client_auth_token_parse() {
        let json = r#"{"type":"auth_token","extension_name":"notion","token":"sk-123"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::AuthToken {
                extension_name,
                token,
            } => {
                assert_eq!(extension_name, "notion");
                assert_eq!(token, "sk-123");
            }
            _ => panic!("Expected AuthToken variant"),
        }
    }

    #[test]
    fn test_ws_client_auth_cancel_parse() {
        let json = r#"{"type":"auth_cancel","extension_name":"notion"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::AuthCancel { extension_name } => {
                assert_eq!(extension_name, "notion");
            }
            _ => panic!("Expected AuthCancel variant"),
        }
    }

    #[test]
    fn test_sse_auth_required_serialize() {
        let event = SseEvent::AuthRequired {
            extension_name: "notion".to_string(),
            instructions: Some("Get your token from...".to_string()),
            auth_url: None,
            setup_url: Some("https://notion.so/integrations".to_string()),
            auth_mode: "manual_token".to_string(),
            auth_status: "awaiting_token".to_string(),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: Some("thread-1".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "auth_required");
        assert_eq!(parsed["extension_name"], "notion");
        assert_eq!(parsed["instructions"], "Get your token from...");
        assert!(parsed.get("auth_url").is_none());
        assert_eq!(parsed["setup_url"], "https://notion.so/integrations");
        assert_eq!(parsed["auth_mode"], "manual_token");
        assert_eq!(parsed["thread_id"], "thread-1");
    }

    #[test]
    fn test_sse_auth_completed_serialize() {
        let event = SseEvent::AuthCompleted {
            extension_name: "notion".to_string(),
            success: true,
            message: "notion authenticated (3 tools loaded)".to_string(),
            auth_mode: Some("manual_token".to_string()),
            auth_status: Some("authenticated".to_string()),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: Some("thread-1".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "auth_completed");
        assert_eq!(parsed["extension_name"], "notion");
        assert_eq!(parsed["success"], true);
    }

    #[test]
    fn test_ws_server_from_sse_auth_required() {
        let sse = SseEvent::AuthRequired {
            extension_name: "openai".to_string(),
            instructions: Some("Enter API key".to_string()),
            auth_url: None,
            setup_url: None,
            auth_mode: "manual_token".to_string(),
            auth_status: "awaiting_token".to_string(),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: None,
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "auth_required");
                assert_eq!(data["extension_name"], "openai");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_auth_completed() {
        let sse = SseEvent::AuthCompleted {
            extension_name: "slack".to_string(),
            success: false,
            message: "Invalid token".to_string(),
            auth_mode: None,
            auth_status: None,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: None,
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "auth_completed");
                assert_eq!(data["success"], false);
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_auth_token_request_deserialize() {
        let json = r#"{"extension_name":"telegram","token":"bot12345"}"#;
        let req: AuthTokenRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extension_name, "telegram");
        assert_eq!(req.token, "bot12345");
    }

    #[test]
    fn test_auth_cancel_request_deserialize() {
        let json = r#"{"extension_name":"telegram"}"#;
        let req: AuthCancelRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extension_name, "telegram");
    }
}
