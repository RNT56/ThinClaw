wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../wit/tool.wit",
});

struct ToolInvokeSmoke;

impl exports::near::agent::tool::Guest for ToolInvokeSmoke {
    fn execute(_req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match near::agent::host::tool_invoke(
            "echo_alias",
            r#"{"message":"hello from wasm tool_invoke"}"#,
        ) {
            Ok(output) => exports::near::agent::tool::Response {
                output: Some(format!(r#"{{"invoked":true,"output":{}}}"#, output)),
                error: None,
            },
            Err(error) => exports::near::agent::tool::Response {
                output: None,
                error: Some(error),
            },
        }
    }

    fn schema() -> String {
        r#"{"type":"object","properties":{},"additionalProperties":false}"#.to_string()
    }

    fn description() -> String {
        "Smoke fixture for host-mediated WASM tool invocation".to_string()
    }
}

export!(ToolInvokeSmoke);
