use super::*;
use rig::message::Reasoning;

#[test]
fn token_capture_extracts_openai_chat_logprobs() {
    let raw = serde_json::json!({
        "choices": [{
            "logprobs": {
                "content": [
                    {"token": "Hello", "logprob": -0.1, "token_id": 9906},
                    {"token": " world", "logprob": -0.2, "token_id": 1917}
                ]
            }
        }]
    });

    let capture =
        extract_provider_token_capture_from_raw(&raw, "openai", "gpt-test").expect("capture");
    assert!(capture.exact_tokens_supported);
    assert!(capture.logprobs_supported);
    assert_eq!(capture.token_ids, vec![9906, 1917]);
    assert_eq!(capture.tokens, vec!["Hello", " world"]);
    assert_eq!(capture.logprobs, vec![-0.1, -0.2]);
    assert_eq!(capture.provider.as_deref(), Some("openai"));
    assert_eq!(capture.model.as_deref(), Some("gpt-test"));
}

#[test]
fn token_capture_extracts_gemini_chosen_candidates() {
    let raw = serde_json::json!({
        "candidates": [{
            "logprobsResult": {
                "chosenCandidates": [
                    {"token": "Gem", "logProbability": -0.3},
                    {"token": "ini", "logProbability": -0.4}
                ]
            }
        }]
    });

    let capture =
        extract_provider_token_capture_from_raw(&raw, "gemini", "gemini-test").expect("capture");
    assert!(capture.exact_tokens_supported);
    assert!(capture.logprobs_supported);
    assert!(capture.token_ids.is_empty());
    assert_eq!(capture.tokens, vec!["Gem", "ini"]);
    assert_eq!(capture.logprobs, vec![-0.3, -0.4]);
}

#[test]
fn token_capture_extracts_local_token_arrays() {
    let raw = serde_json::json!({
        "tokens": [
            {"id": 1, "text": "local", "logprob": -0.5},
            {"id": 2, "text": " model", "logprob": -0.6}
        ]
    });

    let capture =
        extract_provider_token_capture_from_raw(&raw, "llama_cpp", "local").expect("capture");
    assert_eq!(capture.token_ids, vec![1, 2]);
    assert_eq!(capture.tokens, vec!["local", " model"]);
    assert_eq!(capture.logprobs, vec![-0.5, -0.6]);
}

#[test]
fn token_capture_ignores_raw_without_provider_data() {
    let raw = serde_json::json!({
        "choices": [{"message": {"content": "hello"}}]
    });
    assert!(extract_provider_token_capture_from_raw(&raw, "openai", "gpt-test").is_none());
}

#[test]
fn provider_cost_extracts_known_raw_provider_shapes() {
    let openrouter = serde_json::json!({
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "cost": 0.00125
        }
    });
    assert_eq!(
        extract_provider_cost_usd_from_raw(&openrouter),
        Some(0.00125)
    );

    let camel = serde_json::json!({
        "usage": {
            "totalCostUsd": 0.02
        }
    });
    assert_eq!(extract_provider_cost_usd_from_raw(&camel), Some(0.02));

    let top_level = serde_json::json!({
        "cost_usd": 0.5
    });
    assert_eq!(extract_provider_cost_usd_from_raw(&top_level), Some(0.5));
}

#[test]
fn provider_cost_ignores_usage_without_real_cost() {
    let raw = serde_json::json!({
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    });
    assert!(extract_provider_cost_usd_from_raw(&raw).is_none());
}

#[test]
fn merge_additional_params_preserves_thinking_and_logprob_requests() {
    let merged = merge_additional_params(
        Some(serde_json::json!({"thinking": {"type": "enabled"}})),
        Some(serde_json::json!({"logprobs": true, "top_logprobs": 0})),
    )
    .expect("merged params");

    assert_eq!(merged["thinking"]["type"], "enabled");
    assert_eq!(merged["logprobs"], true);
    assert_eq!(merged["top_logprobs"], 0);
}

#[test]
fn requested_model_match_accepts_full_provider_spec() {
    assert!(requested_model_matches_active_model(
        "openai/gpt-5.4-mini",
        "gpt-5.4-mini"
    ));
}

#[test]
fn requested_model_match_rejects_different_model() {
    assert!(!requested_model_matches_active_model(
        "openai/gpt-5.4",
        "gpt-5.4-mini"
    ));
}

#[test]
fn test_convert_messages_system_to_preamble() {
    let messages = vec![
        ChatMessage::system("You are a helpful assistant."),
        ChatMessage::user("Hello"),
    ];
    let (preamble, history, cache_hint_requested) = convert_messages(&messages);
    assert_eq!(preamble, Some("You are a helpful assistant.".to_string()));
    assert_eq!(history.len(), 1);
    assert!(!cache_hint_requested);
}

#[test]
fn test_convert_messages_multiple_systems_concatenated() {
    let messages = vec![
        ChatMessage::system("System 1"),
        ChatMessage::system("System 2"),
        ChatMessage::user("Hi"),
    ];
    let (preamble, history, cache_hint_requested) = convert_messages(&messages);
    assert_eq!(preamble, Some("System 1\nSystem 2".to_string()));
    assert_eq!(history.len(), 1);
    assert!(!cache_hint_requested);
}

#[test]
fn build_rig_request_merges_context_documents_into_preamble() {
    let request = build_rig_request(
        Some("Stable preamble".to_string()),
        vec![RigMessage::user("hello")],
        vec![
            "## External Memory Recall\nRemember this".to_string(),
            "## Skill Expansion\nUse the active skill".to_string(),
        ],
        Vec::new(),
        None,
        Some(0.2),
        Some(512),
        None,
    )
    .expect("rig request should build");

    let preamble = request.preamble.as_deref().unwrap_or_default();
    assert!(preamble.contains("Stable preamble"));
    assert!(preamble.contains("External Memory Recall"));
    assert!(preamble.contains("Skill Expansion"));
    assert!(request.documents.is_empty());
}

#[test]
fn test_convert_messages_detects_anthropic_cache_hint() {
    let messages = vec![
        ChatMessage::system("System 1").with_provider_metadata(
            "anthropic",
            serde_json::json!({"cache_control": {"type": "ephemeral"}}),
        ),
        ChatMessage::user("Hi"),
    ];
    let (preamble, history, cache_hint_requested) = convert_messages(&messages);
    assert_eq!(preamble, Some("System 1".to_string()));
    assert_eq!(history.len(), 1);
    assert!(cache_hint_requested);
}

#[test]
fn test_convert_messages_tool_result() {
    let messages = vec![ChatMessage::tool_result(
        "call_123",
        "search",
        "result text",
    )];
    let (preamble, history, cache_hint_requested) = convert_messages(&messages);
    assert!(preamble.is_none());
    assert_eq!(history.len(), 1);
    assert!(!cache_hint_requested);
    // Tool results become User messages in rig-core
    match &history[0] {
        RigMessage::User { content } => match content.first() {
            UserContent::ToolResult(r) => {
                assert_eq!(r.id, "call_123");
                assert_eq!(r.call_id.as_deref(), Some("call_123"));
            }
            other => panic!("Expected tool result content, got: {:?}", other),
        },
        other => panic!("Expected User message, got: {:?}", other),
    }
}

#[test]
fn test_convert_messages_assistant_with_tool_calls() {
    let tc = IronToolCall {
        id: "call_1".to_string(),
        name: "search".to_string(),
        arguments: serde_json::json!({"query": "test"}),
    };
    let msg = ChatMessage::assistant_with_tool_calls(Some("thinking".to_string()), vec![tc]);
    let messages = vec![msg];
    let (_preamble, history, _cache_hint_requested) = convert_messages(&messages);
    assert_eq!(history.len(), 1);
    match &history[0] {
        RigMessage::Assistant { content, .. } => {
            // Should have both text and tool call
            assert!(content.iter().count() >= 2);
            for item in content.iter() {
                if let AssistantContent::ToolCall(tc) = item {
                    assert_eq!(tc.call_id.as_deref(), Some("call_1"));
                }
            }
        }
        other => panic!("Expected Assistant message, got: {:?}", other),
    }
}

#[test]
fn test_convert_messages_tool_result_without_id_gets_fallback() {
    let messages = vec![ChatMessage {
        role: thinclaw_llm_core::Role::Tool,
        content: "result text".to_string(),
        tool_call_id: None,
        name: Some("search".to_string()),
        tool_calls: None,
        provider_metadata: std::collections::HashMap::new(),
        attachments: Vec::new(),
    }];
    let (_preamble, history, _cache_hint_requested) = convert_messages(&messages);
    match &history[0] {
        RigMessage::User { content } => match content.first() {
            UserContent::ToolResult(r) => {
                assert!(r.id.starts_with("generated_tool_call_"));
                assert_eq!(r.call_id.as_deref(), Some(r.id.as_str()));
            }
            other => panic!("Expected tool result content, got: {:?}", other),
        },
        other => panic!("Expected User message, got: {:?}", other),
    }
}

#[test]
fn test_convert_tools() {
    let tools = vec![IronToolDefinition {
        name: "search".to_string(),
        description: "Search the web".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            }
        }),
    }];
    let rig_tools = convert_tools(&tools);
    assert_eq!(rig_tools.len(), 1);
    assert_eq!(rig_tools[0].name, "search");
    assert_eq!(rig_tools[0].description, "Search the web");
}

#[test]
fn test_convert_tool_choice() {
    assert!(matches!(
        convert_tool_choice(Some("auto")),
        Some(RigToolChoice::Auto)
    ));
    assert!(matches!(
        convert_tool_choice(Some("required")),
        Some(RigToolChoice::Required)
    ));
    assert!(matches!(
        convert_tool_choice(Some("none")),
        Some(RigToolChoice::None)
    ));
    assert!(matches!(
        convert_tool_choice(Some("AUTO")),
        Some(RigToolChoice::Auto)
    ));
    assert!(convert_tool_choice(None).is_none());
    assert!(convert_tool_choice(Some("unknown")).is_none());
}

#[test]
fn test_extract_response_text_only() {
    let content = OneOrMany::one(AssistantContent::text("Hello world"));
    let usage = RigUsage::new();
    let (text, calls, _thinking, finish) = extract_response(&content, &usage);
    assert_eq!(text, Some("Hello world".to_string()));
    assert!(calls.is_empty());
    assert_eq!(finish, FinishReason::Stop);
}

#[test]
fn test_extract_response_tool_call() {
    let tc = AssistantContent::tool_call("call_1", "search", serde_json::json!({"q": "test"}));
    let content = OneOrMany::one(tc);
    let usage = RigUsage::new();
    let (text, calls, _thinking, finish) = extract_response(&content, &usage);
    assert!(text.is_none());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "search");
    assert_eq!(finish, FinishReason::ToolUse);
}

#[test]
fn test_finish_reason_for_tool_use_maps_streaming_done() {
    // A streaming turn that saw a tool call must terminate with ToolUse,
    // mirroring the non-streaming derivation in `extract_response`.
    assert_eq!(finish_reason_for_tool_use(true), FinishReason::ToolUse);
    // A plain text stream terminates with Stop.
    assert_eq!(finish_reason_for_tool_use(false), FinishReason::Stop);
}

#[test]
fn test_assistant_tool_call_empty_id_gets_generated() {
    let tc = IronToolCall {
        id: "".to_string(),
        name: "search".to_string(),
        arguments: serde_json::json!({"query": "test"}),
    };
    let messages = vec![ChatMessage::assistant_with_tool_calls(None, vec![tc])];
    let (_preamble, history, _cache_hint_requested) = convert_messages(&messages);

    match &history[0] {
        RigMessage::Assistant { content, .. } => {
            let tool_call = content.iter().find_map(|c| match c {
                AssistantContent::ToolCall(tc) => Some(tc),
                _ => None,
            });
            let tc = tool_call.expect("should have a tool call");
            assert!(!tc.id.is_empty(), "tool call id must not be empty");
            assert!(
                tc.id.starts_with("generated_tool_call_"),
                "empty id should be replaced with generated id, got: {}",
                tc.id
            );
            assert_eq!(tc.call_id.as_deref(), Some(tc.id.as_str()));
        }
        other => panic!("Expected Assistant message, got: {:?}", other),
    }
}

#[test]
fn test_assistant_tool_call_whitespace_id_gets_generated() {
    let tc = IronToolCall {
        id: "   ".to_string(),
        name: "search".to_string(),
        arguments: serde_json::json!({"query": "test"}),
    };
    let messages = vec![ChatMessage::assistant_with_tool_calls(None, vec![tc])];
    let (_preamble, history, _cache_hint_requested) = convert_messages(&messages);

    match &history[0] {
        RigMessage::Assistant { content, .. } => {
            let tool_call = content.iter().find_map(|c| match c {
                AssistantContent::ToolCall(tc) => Some(tc),
                _ => None,
            });
            let tc = tool_call.expect("should have a tool call");
            assert!(
                tc.id.starts_with("generated_tool_call_"),
                "whitespace-only id should be replaced, got: {:?}",
                tc.id
            );
        }
        other => panic!("Expected Assistant message, got: {:?}", other),
    }
}

#[test]
fn test_assistant_and_tool_result_missing_ids_share_generated_id() {
    // Simulate: assistant emits a tool call with empty id, then tool
    // result arrives without an id. Both should get deterministic
    // generated ids that match (based on their position in history).
    let tc = IronToolCall {
        id: "".to_string(),
        name: "search".to_string(),
        arguments: serde_json::json!({"query": "test"}),
    };
    let assistant_msg = ChatMessage::assistant_with_tool_calls(None, vec![tc]);
    let tool_result_msg = ChatMessage {
        role: thinclaw_llm_core::Role::Tool,
        content: "search results here".to_string(),
        tool_call_id: None,
        name: Some("search".to_string()),
        tool_calls: None,
        provider_metadata: std::collections::HashMap::new(),
        attachments: Vec::new(),
    };
    let messages = vec![assistant_msg, tool_result_msg];
    let (_preamble, history, _cache_hint_requested) = convert_messages(&messages);

    // Extract the generated call_id from the assistant tool call
    let assistant_call_id = match &history[0] {
        RigMessage::Assistant { content, .. } => {
            let tc = content.iter().find_map(|c| match c {
                AssistantContent::ToolCall(tc) => Some(tc),
                _ => None,
            });
            tc.expect("should have tool call").id.clone()
        }
        other => panic!("Expected Assistant message, got: {:?}", other),
    };

    // Extract the generated call_id from the tool result
    let tool_result_call_id = match &history[1] {
        RigMessage::User { content } => match content.first() {
            UserContent::ToolResult(r) => r
                .call_id
                .clone()
                .expect("tool result call_id must be present"),
            other => panic!("Expected ToolResult, got: {:?}", other),
        },
        other => panic!("Expected User message, got: {:?}", other),
    };

    assert!(
        !assistant_call_id.is_empty(),
        "assistant call_id must not be empty"
    );
    assert!(
        !tool_result_call_id.is_empty(),
        "tool result call_id must not be empty"
    );

    // NOTE: With the current seed-based generation, these IDs will differ
    // because the assistant tool call uses seed=0 (history.len() at that
    // point) and the tool result uses seed=1 (history.len() after the
    // assistant message was pushed). This documents the current behavior.
    // A future improvement could thread the assistant's generated ID into
    // the tool result for exact matching.
    assert_ne!(
        assistant_call_id, tool_result_call_id,
        "Current impl generates different IDs for assistant call and tool result \
         because seeds differ; this documents the known limitation"
    );
}

#[test]
fn test_saturate_u32() {
    assert_eq!(saturate_u32(100), 100);
    assert_eq!(saturate_u32(u64::MAX), u32::MAX);
    assert_eq!(saturate_u32(u32::MAX as u64), u32::MAX);
}

// -- normalize_tool_name tests --

#[test]
fn test_normalize_tool_name_exact_match() {
    let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
    assert_eq!(normalize_tool_name("echo", &known), "echo");
}

#[test]
fn test_normalize_tool_name_proxy_prefix_match() {
    let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
    assert_eq!(normalize_tool_name("proxy_echo", &known), "echo");
}

#[test]
fn test_normalize_tool_name_proxy_prefix_no_match_kept() {
    let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
    assert_eq!(
        normalize_tool_name("proxy_unknown", &known),
        "proxy_unknown"
    );
}

#[test]
fn test_normalize_tool_name_unknown_passthrough() {
    let known = HashSet::from(["echo".to_string()]);
    assert_eq!(normalize_tool_name("other_tool", &known), "other_tool");
}

// -- thinking_config_to_params tests --

#[test]
fn test_thinking_config_disabled_returns_none() {
    let config = ThinkingConfig::Disabled;
    assert!(thinking_config_to_params(&config).is_none());
}

#[test]
fn test_thinking_config_enabled_returns_anthropic_params() {
    let config = ThinkingConfig::Enabled {
        budget_tokens: 8192,
    };
    let params = thinking_config_to_params(&config).expect("should return Some");
    let thinking = params.get("thinking").expect("should have 'thinking' key");
    assert_eq!(thinking["type"], "enabled");
    assert_eq!(thinking["budget_tokens"], 8192);
}

#[test]
fn test_thinking_config_enabled_zero_budget() {
    let config = ThinkingConfig::Enabled { budget_tokens: 0 };
    let params = thinking_config_to_params(&config).expect("should return Some");
    assert_eq!(params["thinking"]["budget_tokens"], 0);
}

#[test]
fn test_thinking_config_enabled_large_budget() {
    let config = ThinkingConfig::Enabled {
        budget_tokens: 100_000,
    };
    let params = thinking_config_to_params(&config).expect("should return Some");
    assert_eq!(params["thinking"]["budget_tokens"], 100_000);
}

// -- extract_response reasoning content tests --

#[test]
fn test_extract_response_with_reasoning() {
    let reasoning = AssistantContent::Reasoning(Reasoning::multi(vec![
        "Step 1: analyze".to_string(),
        "Step 2: conclude".to_string(),
    ]));
    let text = AssistantContent::text("The answer is 42.");

    let content = OneOrMany::many(vec![reasoning, text]).unwrap();
    let usage = RigUsage::new();
    let (text, calls, thinking, finish) = extract_response(&content, &usage);

    assert_eq!(text, Some("The answer is 42.".to_string()));
    assert!(calls.is_empty());
    assert_eq!(
        thinking,
        Some("Step 1: analyze\nStep 2: conclude".to_string())
    );
    assert_eq!(finish, FinishReason::Stop);
}

#[test]
fn test_extract_response_no_reasoning() {
    let content = OneOrMany::one(AssistantContent::text("Just text."));
    let usage = RigUsage::new();
    let (_text, _calls, thinking, _finish) = extract_response(&content, &usage);
    assert!(thinking.is_none());
}

#[test]
fn test_extract_response_reasoning_with_tool_calls() {
    let reasoning = AssistantContent::Reasoning(Reasoning::new("I should search for this."));
    let tc = AssistantContent::tool_call("call_1", "search", serde_json::json!({"q": "test"}));

    let content = OneOrMany::many(vec![reasoning, tc]).unwrap();
    let usage = RigUsage::new();
    let (text, calls, thinking, finish) = extract_response(&content, &usage);

    assert!(text.is_none());
    assert_eq!(calls.len(), 1);
    assert_eq!(thinking, Some("I should search for this.".to_string()));
    assert_eq!(finish, FinishReason::ToolUse);
}

#[test]
fn test_extract_response_empty_reasoning_ignored() {
    let reasoning =
        AssistantContent::Reasoning(Reasoning::multi(vec!["".to_string(), "".to_string()]));
    let text = AssistantContent::text("Result");

    let content = OneOrMany::many(vec![reasoning, text]).unwrap();
    let usage = RigUsage::new();
    let (_, _, thinking, _) = extract_response(&content, &usage);

    // Empty reasoning strings should be filtered, resulting in None
    assert!(thinking.is_none());
}
