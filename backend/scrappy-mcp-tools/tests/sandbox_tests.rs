#[cfg(test)]
mod tests {
    use scrappy_mcp_tools::{
        events::NullReporter,
        sandbox::{Sandbox, SandboxConfig, SandboxError},
    };
    use std::sync::Arc;

    fn make_sandbox() -> Sandbox {
        Sandbox::new(SandboxConfig::default(), Arc::new(NullReporter))
    }

    // -----------------------------------------------------------------------
    // Basic execution
    // -----------------------------------------------------------------------

    #[test]
    fn test_simple_expression() {
        let sb = make_sandbox();
        let result = sb.execute("40 + 2").expect("should execute");
        assert_eq!(result.output, "42");
    }

    #[test]
    fn test_string_result() {
        let sb = make_sandbox();
        let result = sb.execute(r#""hello world""#).expect("should execute");
        assert_eq!(result.output, "hello world");
    }

    #[test]
    fn test_unit_result() {
        let sb = make_sandbox();
        let result = sb.execute("let x = 5;").expect("should execute");
        assert_eq!(result.output, "null");
    }

    #[test]
    fn test_multiline_script() {
        let sb = make_sandbox();
        let result = sb
            .execute(
                r#"
let a = 10;
let b = 20;
a + b
"#,
            )
            .expect("should execute");
        assert_eq!(result.output, "30");
    }

    // -----------------------------------------------------------------------
    // Built-in helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_timestamp_now() {
        let sb = make_sandbox();
        let result = sb.execute("timestamp_now()").expect("should execute");
        // Should be an RFC3339 timestamp string
        assert!(
            result.output.contains("T"),
            "Expected RFC3339 timestamp, got: {}",
            result.output
        );
    }

    #[test]
    fn test_json_stringify() {
        let sb = make_sandbox();
        let result = sb
            .execute(r#"let m = #{"key": "value"}; json_stringify(m)"#)
            .expect("should execute");
        assert!(
            result.output.contains("key"),
            "Expected JSON with 'key', got: {}",
            result.output
        );
    }

    // -----------------------------------------------------------------------
    // Host tool registration
    // -----------------------------------------------------------------------

    #[test]
    fn test_register_host_tool() {
        let mut sb = make_sandbox();
        sb.engine_mut()
            .register_fn("greet", |name: String| -> String {
                format!("Hello, {}!", name)
            });

        let result = sb.execute(r#"greet("Scrappy")"#).expect("should execute");
        assert_eq!(result.output, "Hello, Scrappy!");
    }

    #[test]
    fn test_host_tool_with_computation() {
        let mut sb = make_sandbox();
        sb.engine_mut()
            .register_fn("double", |n: i64| -> i64 { n * 2 });

        let result = sb.execute("let x = double(21); x").expect("should execute");
        assert_eq!(result.output, "42");
    }

    // -----------------------------------------------------------------------
    // Security: Forbidden patterns
    // -----------------------------------------------------------------------

    #[test]
    fn test_forbidden_std_fs() {
        let sb = make_sandbox();
        let err = sb
            .execute("std::fs::read(\"foo\")")
            .expect_err("should reject");
        match err {
            SandboxError::ForbiddenPattern(pat) => assert_eq!(pat, "std::fs"),
            other => panic!("Expected ForbiddenPattern, got {:?}", other),
        }
    }

    #[test]
    fn test_forbidden_unsafe() {
        let sb = make_sandbox();
        let err = sb
            .execute("unsafe { let x = 1; }")
            .expect_err("should reject");
        match err {
            SandboxError::ForbiddenPattern(pat) => assert_eq!(pat, "unsafe"),
            other => panic!("Expected ForbiddenPattern, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Error handling
    // -----------------------------------------------------------------------

    #[test]
    fn test_unknown_function() {
        let sb = make_sandbox();
        let err = sb
            .execute("nonexistent_tool(\"arg\")")
            .expect_err("should error");
        match err {
            SandboxError::Compilation(msg) => {
                assert!(
                    msg.contains("Unknown function"),
                    "Expected 'Unknown function', got: {}",
                    msg
                );
            }
            other => panic!("Expected Compilation error, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_error() {
        let sb = make_sandbox();
        let err = sb.execute("let x = ").expect_err("should error");
        match err {
            SandboxError::Compilation(msg) => {
                assert!(
                    msg.contains("Parse error"),
                    "Expected 'Parse error', got: {}",
                    msg
                );
            }
            other => panic!("Expected Compilation error, got {:?}", other),
        }
    }

    #[test]
    fn test_operations_limit() {
        let mut config = SandboxConfig::default();
        config.max_operations = 100; // Very low limit

        let sb = Sandbox::new(config, Arc::new(NullReporter));
        let err = sb
            .execute("let x = 0; while x < 100000 { x += 1; } x")
            .expect_err("should timeout");
        match err {
            SandboxError::Timeout(_) => {} // Expected
            other => panic!("Expected Timeout, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Error → LLM feedback formatting
    // -----------------------------------------------------------------------

    #[test]
    fn test_llm_feedback_runtime() {
        let err = SandboxError::Runtime("variable not found".into());
        let feedback = err.to_llm_feedback();
        assert!(feedback.contains("Tool Execution Error"));
        assert!(feedback.contains("Hint"));
    }

    #[test]
    fn test_llm_feedback_compilation() {
        let err = SandboxError::Compilation("Unknown function: foo".into());
        let feedback = err.to_llm_feedback();
        assert!(feedback.contains("Script Compilation Error"));
        assert!(feedback.contains("search_tools"));
    }

    #[test]
    fn test_llm_feedback_forbidden() {
        let err = SandboxError::ForbiddenPattern("std::fs".into());
        let feedback = err.to_llm_feedback();
        assert!(feedback.contains("Security Violation"));
    }

    // -----------------------------------------------------------------------
    // Result size limit
    // -----------------------------------------------------------------------

    #[test]
    fn test_result_size_limit() {
        let mut config = SandboxConfig::default();
        config.max_result_size = 10; // Very small

        let sb = Sandbox::new(config, Arc::new(NullReporter));
        let err = sb
            .execute(r#""This string is definitely longer than 10 bytes""#)
            .expect_err("should error");
        match err {
            SandboxError::ResultTooLarge { .. } => {} // Expected
            other => panic!("Expected ResultTooLarge, got {:?}", other),
        }
    }
}
