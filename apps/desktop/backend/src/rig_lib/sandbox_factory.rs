use crate::rig_lib::RigManager;
use std::sync::Arc;
use thinclaw_desktop_tools::events::StatusReporter;
use thinclaw_desktop_tools::sandbox::{Sandbox, SandboxConfig};
use tracing::{info, warn};

const MAX_TOOL_TEXT_BYTES: usize = 64 * 1024;
const MAX_TOOL_JSON_BYTES: usize = 1024 * 1024;
const MAX_DOCUMENT_READ_BYTES: u64 = 1024 * 1024;
const MAX_DOCUMENT_RESULT_BYTES: usize = 20_000;

fn bounded_tool_text<'a>(value: &'a str, label: &str, max_bytes: usize) -> Result<&'a str, String> {
    if value.trim() != value || value.is_empty() || value.len() > max_bytes || value.contains('\0')
    {
        return Err(format!("{label} is missing, malformed, or oversized"));
    }
    Ok(value)
}

fn bounded_limit(limit: i64, maximum: usize) -> Result<usize, String> {
    usize::try_from(limit)
        .ok()
        .filter(|value| (1..=maximum).contains(value))
        .ok_or_else(|| format!("limit must be between 1 and {maximum}"))
}

fn dynamic_from_serializable<T: serde::Serialize>(value: T) -> rhai::Dynamic {
    rhai::serde::to_dynamic(value)
        .unwrap_or_else(|_| rhai::Dynamic::from("Error: tool response could not be encoded"))
}

fn read_managed_document(root: Option<&std::path::Path>, requested: &str) -> String {
    use std::path::{Component, Path, PathBuf};

    let Some(root) = root else {
        return "Read error: managed document storage is unavailable".to_string();
    };
    if requested.trim() != requested
        || requested.is_empty()
        || requested.len() > 4_096
        || requested.contains('\0')
    {
        return "Read error: document path is invalid".to_string();
    }
    let requested = Path::new(requested);
    if requested.is_absolute() {
        return "Read error: use a path relative to managed document storage".to_string();
    }
    let mut components = requested.components().peekable();
    if components.peek().is_some_and(
        |component| matches!(component, Component::Normal(name) if *name == "documents"),
    ) {
        components.next();
    }
    let mut relative = PathBuf::new();
    for component in components {
        match component {
            Component::Normal(part) if !part.is_empty() => relative.push(part),
            _ => return "Read error: document path is not normalized".to_string(),
        }
    }
    if relative.as_os_str().is_empty() {
        return "Read error: document path is empty".to_string();
    }

    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        _ => return "Read error: managed document storage is unavailable".to_string(),
    }
    let mut candidate = root.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        let Component::Normal(part) = component else {
            return "Read error: document path is not normalized".to_string();
        };
        candidate.push(part);
        if index + 1 < component_count {
            match std::fs::symlink_metadata(&candidate) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                _ => return "Read error: document parent is not a real directory".to_string(),
            }
        }
    }
    let bytes = match thinclaw_platform::read_regular_file_bounded_single_link(
        &candidate,
        MAX_DOCUMENT_READ_BYTES,
    ) {
        Ok(bytes) => bytes,
        Err(_) => return "Read error: document is unavailable or unsafe".to_string(),
    };
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(_) => return "Read error: document is not UTF-8 text".to_string(),
    };
    if content.len() <= MAX_DOCUMENT_RESULT_BYTES {
        return content;
    }
    let mut end = MAX_DOCUMENT_RESULT_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... (truncated)", &content[..end])
}

/// Configuration for the MCP sandbox factory
#[derive(Clone, Debug, Default)]
pub struct McpOrchestratorConfig {
    /// Base URL of the FastAPI MCP server
    pub mcp_base_url: Option<String>,
    /// JWT bearer token for the MCP server
    pub mcp_auth_token: Option<String>,
    /// Whether sandbox mode is enabled
    pub sandbox_enabled: bool,
    /// Register outbound web-search functions in the sandbox.
    pub allow_web_search: bool,
    /// Register managed-document search/read functions in the sandbox.
    pub allow_file_search: bool,
    /// Register image generation in the sandbox.
    pub allow_image_gen: bool,
    /// Path to user-defined skills
    pub user_skills_path: Option<std::path::PathBuf>,
    /// Path to built-in skills
    pub builtin_skills_path: Option<std::path::PathBuf>,
}

/// Names of the host builtin functions this factory exposes to sandboxed Rhai
/// skills. Kept in sync with the `register_fn` calls in [`create_sandbox`];
/// used for best-effort `tools_used` detection when persisting a skill.
const SANDBOX_BUILTIN_TOOLS: &[&str] = &[
    "list_skills",
    "run_skill",
    "web_search",
    "rag_search",
    "read_file",
    "calculator",
    "calculator_with_vars",
    "search_tools",
    "save_skill",
    "mcp_call",
    "finance::get_stock_price",
    "news::get_news",
    "news::search_news",
    "news::get_headlines",
    "knowledge::rag_query",
    "economics::get_economic_data",
    "models::get_model_catalog",
    "health::search_medical_research",
    "ai_tools::summarize_text",
];

/// Best-effort detection of which host builtins a Rhai skill script calls.
///
/// This is a lightweight ident scan, not a Rhai parse: it looks for each known
/// builtin name immediately followed by a `(`, ignoring surrounding whitespace.
/// It can over-report if a builtin name appears inside a string/comment, but it
/// never under-reports a genuine call, which is the useful direction for the
/// persisted `SkillManifest.tools_used` metadata. Results are de-duplicated and
/// returned in the stable order of [`SANDBOX_BUILTIN_TOOLS`].
fn detect_tools_used(script: &str) -> Vec<String> {
    SANDBOX_BUILTIN_TOOLS
        .iter()
        .filter(|name| script_calls_builtin(script, name))
        .map(|name| name.to_string())
        .collect()
}

/// Returns `true` if `script` appears to invoke `name` (the builtin name is
/// followed, after optional whitespace, by an opening parenthesis).
fn script_calls_builtin(script: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = script[from..].find(name) {
        let start = from + rel;
        let after = start + name.len();
        // Reject a match that is part of a longer identifier (e.g. `web_search`
        // matching inside `web_search_v2`). Module-qualified names contain `:`,
        // which is not an identifier char, so the preceding-char guard still
        // holds for the leading segment.
        let prev_ok = script[..start]
            .chars()
            .next_back()
            .map(|c| !c.is_alphanumeric() && c != '_')
            .unwrap_or(true);
        let next_is_call = script[after..]
            .chars()
            .find(|c| !c.is_whitespace())
            .map(|c| c == '(')
            .unwrap_or(false);
        if prev_ok && next_is_call {
            return true;
        }
        from = after;
    }
    false
}

/// Persist a user skill manifest + script. Shared by both `save_skill` builtin
/// overloads — the 3-arg form (no typed parameters) and the 4-arg form that
/// accepts a JSON parameter list (F-15). Pure sync file I/O; callers wrap it in
/// `tokio::task::block_in_place` because the builtins run inside the async runtime.
fn persist_user_skill(
    id: &str,
    script: &str,
    description: &str,
    parameters: Vec<thinclaw_desktop_tools::skills::manifest::SkillParameter>,
    user_skills_path: Option<std::path::PathBuf>,
    builtin_skills_path: Option<std::path::PathBuf>,
) -> String {
    use thinclaw_desktop_tools::skills::manager::SkillManager;
    use thinclaw_desktop_tools::skills::manifest::SkillManifest;

    let id = id.trim();
    let script = script.trim();
    if id.is_empty()
        || id.len() > 64
        || script.is_empty()
        || script.len() > 256 * 1024
        || description.len() > 16 * 1024
        || parameters.len() > 64
    {
        return "Error: skill input is missing, malformed, or oversized".to_string();
    }
    let Some(user_path) = user_skills_path else {
        return "Error: User skills directory not configured".to_string();
    };
    let manifest = SkillManifest {
        name: id.to_string(), // Using ID as name for simplicity
        description: description.to_string(),
        version: "1.0.0".to_string(),
        author: Some("Agent".to_string()),
        // Best-effort scan of the script for host builtins it calls.
        tools_used: detect_tools_used(script),
        parameters,
        script_file: format!("{}.rhai", id),
    };
    let manager = SkillManager::new(user_path, builtin_skills_path);
    match manager.save_skill(id, manifest, script) {
        Ok(_) => format!("Skill '{}' saved successfully", id),
        Err(e) => format!("Error saving skill: {}", e),
    }
}

/// Parse the optional 4th `save_skill` argument — a JSON array of skill
/// parameters — into typed [`SkillParameter`]s. A blank string yields an empty
/// list so callers may pass `""` to mean "no declared parameters".
fn parse_skill_parameters(
    params_json: &str,
) -> Result<Vec<thinclaw_desktop_tools::skills::manifest::SkillParameter>, String> {
    let trimmed = params_json.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.len() > 256 * 1024 {
        return Err("skill parameters JSON exceeds the size limit".to_string());
    }
    serde_json::from_str(trimmed).map_err(|e| e.to_string())
}

/// Create a configured Sandbox instance with all host tools registered.
///
/// This factory function is used by:
/// 1. `Orchestrator` (for Rig agents)
/// 2. `McpRequestHandler` (for ThinClaw requests via IPC)
pub fn create_sandbox<R>(
    rig: Arc<RigManager>,
    mcp_config: &McpOrchestratorConfig,
    reporter: Arc<R>,
) -> Option<Sandbox>
where
    R: StatusReporter + 'static,
{
    if !mcp_config.sandbox_enabled {
        return None;
    }

    let config = SandboxConfig::default();
    let mut sandbox = Sandbox::new(config, reporter);

    // -- Register Skills System (list_skills, run_skill) --
    if let Some(user_path) = &mcp_config.user_skills_path {
        use thinclaw_desktop_tools::skills::manager::SkillManager;
        let manager = SkillManager::new(user_path.clone(), mcp_config.builtin_skills_path.clone());
        let manager_arc = Arc::new(manager);

        // list_skills() -> JSON array
        let m1 = manager_arc.clone();
        sandbox
            .engine_mut()
            .register_fn("list_skills", move || -> rhai::Dynamic {
                match m1.list_skills() {
                    Ok(skills) => {
                        let list: Vec<serde_json::Value> = skills
                            .into_iter()
                            .map(|s| {
                                serde_json::json!({
                                    "id": s.id,
                                    "name": s.manifest.name,
                                    "description": s.manifest.description,
                                    "parameters": s.manifest.parameters
                                })
                            })
                            .collect();
                        dynamic_from_serializable(serde_json::Value::Array(list))
                    }
                    Err(e) => format!("Error listing skills: {}", e).into(),
                }
            });

        // run_skill(id, args_json) -> result
        let m2 = manager_arc.clone();
        sandbox.engine_mut().register_fn(
            "run_skill",
            move |ctx: rhai::NativeCallContext, id: String, args_json: String| -> rhai::Dynamic {
                let manager = m2.clone();

                // 1. Prepare
                if args_json.len() > MAX_TOOL_JSON_BYTES {
                    return "Skill Error: arguments exceed the size limit".into();
                }
                let args: serde_json::Value = match serde_json::from_str(&args_json) {
                    Ok(args) => args,
                    Err(_) => return "Skill Error: arguments are not valid JSON".into(),
                };
                let args_map = match args.as_object() {
                    Some(obj) => obj
                        .clone()
                        .into_iter()
                        .collect::<std::collections::HashMap<String, serde_json::Value>>(),
                    None => return "Skill Error: arguments must be a JSON object".into(),
                };

                let script = match manager.prepare_script(&id, &args_map) {
                    Ok(s) => s,
                    Err(e) => return format!("Skill Error: {}", e).into(),
                };
                if script.len() > 256 * 1024 {
                    return "Skill Error: prepared script exceeds the execution limit".into();
                }

                // 2. Execute in a new scope
                // Note: Skills are self-contained with injected parameters
                let mut scope = rhai::Scope::new();
                match ctx
                    .engine()
                    .eval_with_scope::<rhai::Dynamic>(&mut scope, &script)
                {
                    Ok(result) => result,
                    Err(e) => format!("Skill Execution Error ({}): {}", id, e).into(),
                }
            },
        );
    }

    if mcp_config.allow_web_search {
        // -- Register host tool: web_search --
        let rig_for_ws = rig.clone();
        sandbox
            .engine_mut()
            .register_fn("web_search", move |query: String| -> rhai::Dynamic {
                let query = match bounded_tool_text(&query, "search query", 16 * 1024) {
                    Ok(query) => query.to_string(),
                    Err(error) => return error.into(),
                };
                let rig = rig_for_ws.clone();
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(async { rig.explicit_search(&query).await })
                });
                rhai::Dynamic::from(result)
            });
    }

    if mcp_config.allow_file_search {
        // -- Register host tool: rag_search --
        let rig_for_rag = rig.clone();
        sandbox
            .engine_mut()
            .register_fn("rag_search", move |query: String| -> rhai::Dynamic {
                let query = match bounded_tool_text(&query, "RAG query", 16 * 1024) {
                    Ok(query) => query.to_string(),
                    Err(error) => return error.into(),
                };
                let rig = rig_for_rag.clone();
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        if let Some(app) = &rig.app_handle {
                            use tauri::Manager;
                            let sidecar = app.state::<crate::sidecar::SidecarManager>();
                            let pool = app.state::<sqlx::SqlitePool>();
                            let store = app.state::<crate::vector_store::VectorStoreManager>();
                            let reranker = app.state::<crate::reranker::RerankerWrapper>();
                            let emb_backend = {
                                let router =
                                    app.state::<crate::inference::router::InferenceRouter>();
                                router.embedding_backend().await
                            };
                            match crate::rag::retrieve_context_internal(
                                Some(app.clone()),
                                sidecar.inner(),
                                pool.inner().clone(),
                                store.inner().clone(),
                                reranker.inner(),
                                emb_backend,
                                query.clone(),
                                rig.conversation_id.clone(),
                                None::<Vec<String>>,
                                None,
                            )
                            .await
                            {
                                Ok(results) => results.join("\n\n"),
                                Err(e) => format!("RAG Error: {}", e),
                            }
                        } else {
                            "App state not available".to_string()
                        }
                    })
                });
                rhai::Dynamic::from(result)
            });

        // -- Register host tool: read_file (managed documents only) --
        let managed_documents_root = rig.app_handle.as_ref().and_then(|app| {
            use tauri::Manager;
            app.path()
                .app_data_dir()
                .ok()
                .map(|root| root.join("documents"))
        });
        sandbox
            .engine_mut()
            .register_fn("read_file", move |path: String| -> rhai::Dynamic {
                rhai::Dynamic::from(read_managed_document(
                    managed_documents_root.as_deref(),
                    &path,
                ))
            });
    }

    if mcp_config.allow_image_gen {
        let image_app = rig.app_handle.clone();
        sandbox.engine_mut().register_fn(
            "generate_image",
            move |prompt: String| -> rhai::Dynamic {
                let prompt = match bounded_tool_text(&prompt, "image prompt", 32 * 1024) {
                    Ok(prompt) => prompt.to_string(),
                    Err(error) => return error.into(),
                };
                let Some(app) = image_app.clone() else {
                    return "Image generation is unavailable".into();
                };
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async move {
                        use rig::tool::Tool as _;
                        crate::rig_lib::tools::image_gen_tool::ImageGenTool { app }
                            .call(crate::rig_lib::tools::image_gen_tool::ImageGenArgs {
                                prompt,
                                negative_prompt: None,
                            })
                            .await
                            .unwrap_or_else(|error| format!("Image generation error: {error}"))
                    })
                });
                result.into()
            },
        );
    }

    // -- Register host tool: calculator (pure computation, no async needed) --
    // Returns full trace + result for transparency
    sandbox
        .engine_mut()
        .register_fn("calculator", |expr: String| -> rhai::Dynamic {
            if let Err(error) =
                bounded_tool_text(&expr, "calculator expression", MAX_TOOL_TEXT_BYTES)
            {
                return error.into();
            }
            match crate::rig_lib::tools::calculator_tool::evaluate_with_vars(
                &expr,
                std::collections::HashMap::new(),
            ) {
                Ok(output) => rhai::Dynamic::from(
                    crate::rig_lib::tools::calculator_tool::format_eval_output(&output),
                ),
                Err(e) => rhai::Dynamic::from(format!("Calculator Error: {}", e)),
            }
        });

    // -- calculator_with_vars(expression, vars_json) for variable support --
    sandbox.engine_mut().register_fn(
        "calculator_with_vars",
        |expr: String, vars_json: String| -> rhai::Dynamic {
            if let Err(error) = bounded_tool_text(&expr, "calculator expression", 64 * 1024) {
                return error.into();
            }
            if vars_json.len() > 256 * 1024 {
                return "Calculator Error: variables JSON exceeds the size limit".into();
            }
            let vars: std::collections::HashMap<String, f64> =
                match serde_json::from_str::<std::collections::HashMap<String, f64>>(&vars_json) {
                    Ok(vars) if vars.len() <= 128 => vars,
                    Ok(_) => return "Calculator Error: too many variables".into(),
                    Err(_) => return "Calculator Error: variables are not valid JSON".into(),
                };
            match crate::rig_lib::tools::calculator_tool::evaluate_with_vars(&expr, vars) {
                Ok(output) => rhai::Dynamic::from(
                    crate::rig_lib::tools::calculator_tool::format_eval_output(&output),
                ),
                Err(e) => rhai::Dynamic::from(format!("Calculator Error: {}", e)),
            }
        },
    );

    // -- search_tools(query) → progressive discovery --
    let mcp_client_for_discovery = if let Some(base_url) = &mcp_config.mcp_base_url {
        thinclaw_desktop_tools::McpClient::new(thinclaw_desktop_tools::McpConfig {
            base_url: base_url.clone(),
            auth_token: mcp_config.mcp_auth_token.clone().unwrap_or_default(),
            timeout_ms: 30_000,
        })
        .ok()
    } else {
        None
    };

    let user_skills_path = mcp_config.user_skills_path.clone();
    let builtin_skills_path = mcp_config.builtin_skills_path.clone();
    let mut allowed_host_tools = vec!["calculator".to_string()];
    if mcp_config.allow_web_search {
        allowed_host_tools.push("web_search".to_string());
    }
    if mcp_config.allow_file_search {
        allowed_host_tools.extend(["rag_search".to_string(), "read_file".to_string()]);
    }
    if mcp_config.allow_image_gen {
        allowed_host_tools.push("generate_image".to_string());
    }

    sandbox
        .engine_mut()
        .register_fn("search_tools", move |query: String| -> rhai::Dynamic {
            if query.len() > 4_096 || query.contains('\0') {
                return rhai::Dynamic::from("Error: tool search query is malformed or oversized");
            }
            let client = mcp_client_for_discovery.clone();
            let upath = user_skills_path.clone();
            let bpath = builtin_skills_path.clone();
            let allowed_host_tools = allowed_host_tools.clone();

            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    use thinclaw_desktop_tools::skills::manager::SkillManager;
                    let skill_manager = upath.map(|up| SkillManager::new(up, bpath));

                    let sr = crate::rig_lib::tool_discovery::search_all_tools(
                        &query,
                        client.as_ref(),
                        skill_manager.as_ref(),
                        &allowed_host_tools,
                    )
                    .await;

                    serde_json::to_string(&sr).unwrap_or_else(|_| "{}".to_string())
                })
            });
            rhai::Dynamic::from(result)
        });

    // -- save_skill(id, script, description) → persist skill (no typed params) --
    let user_skills_path_save = mcp_config.user_skills_path.clone();
    let builtin_skills_path_save = mcp_config.builtin_skills_path.clone();

    sandbox.engine_mut().register_fn(
        "save_skill",
        move |id: String, script: String, description: String| -> String {
            let upath = user_skills_path_save.clone();
            let bpath = builtin_skills_path_save.clone();
            tokio::task::block_in_place(|| {
                persist_user_skill(&id, &script, &description, Vec::new(), upath, bpath)
            })
        },
    );

    // -- save_skill(id, script, description, params_json) → persist skill with
    //    typed parameters (F-15). `params_json` is a JSON array of
    //    {name, description, param_type, required, default} objects; pass "" for
    //    none. Backward-compatible: the 3-arg form above keeps working unchanged
    //    (Rhai dispatches overloads by arity).
    let user_skills_path_save_p = mcp_config.user_skills_path.clone();
    let builtin_skills_path_save_p = mcp_config.builtin_skills_path.clone();

    sandbox.engine_mut().register_fn(
        "save_skill",
        move |id: String, script: String, description: String, params_json: String| -> String {
            let parameters = match parse_skill_parameters(&params_json) {
                Ok(parameters) => parameters,
                Err(e) => return format!("Error: invalid skill parameters JSON: {}", e),
            };
            let upath = user_skills_path_save_p.clone();
            let bpath = builtin_skills_path_save_p.clone();
            tokio::task::block_in_place(|| {
                persist_user_skill(&id, &script, &description, parameters, upath, bpath)
            })
        },
    );

    // -- Register remote MCP tools (only when server URL is configured) --
    if let Some(base_url) = &mcp_config.mcp_base_url {
        let client_mcp_config = thinclaw_desktop_tools::McpConfig {
            base_url: base_url.clone(),
            auth_token: mcp_config.mcp_auth_token.clone().unwrap_or_default(),
            timeout_ms: 30_000,
        };
        if let Ok(client) = thinclaw_desktop_tools::McpClient::new(client_mcp_config) {
            // -- mcp_call(tool_name, args_json) → generic remote tool caller --
            let client_for_call = client.clone();
            sandbox.engine_mut().register_fn(
                "mcp_call",
                move |tool: String, args_json: String| -> rhai::Dynamic {
                    if bounded_tool_text(&tool, "MCP tool name", 256).is_err()
                        || args_json.len() > MAX_TOOL_JSON_BYTES
                    {
                        return "MCP Error: tool name or arguments are malformed or oversized"
                            .into();
                    }
                    let args: serde_json::Value = match serde_json::from_str(&args_json) {
                        Ok(serde_json::Value::Object(arguments)) => {
                            serde_json::Value::Object(arguments)
                        }
                        Ok(_) => return "MCP Error: arguments must be a JSON object".into(),
                        Err(_) => return "MCP Error: arguments are not valid JSON".into(),
                    };
                    let c = client_for_call.clone();
                    let result = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            match c.call_tool_raw(&tool, args).await {
                                Ok(val) => val.to_string(),
                                Err(e) => format!("MCP Error: {}", e),
                            }
                        })
                    });
                    rhai::Dynamic::from(result)
                },
            );

            // =======================================================
            // Type-Safe Tool Bindings (Modules)
            // =======================================================

            // FINANCE
            {
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "finance::get_stock_price",
                    move |symbol: String| -> rhai::Dynamic {
                        if let Err(error) = bounded_tool_text(&symbol, "stock symbol", 256) {
                            return error.into();
                        }
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                thinclaw_desktop_tools::tools::finance::get_stock_price(&c, &symbol)
                                    .await
                                    .map(dynamic_from_serializable)
                                    .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
            }

            // NEWS
            {
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "news::get_news",
                    move |category: String, limit: i64| -> rhai::Dynamic {
                        let limit = match bounded_limit(limit, 100) {
                            Ok(limit) => limit,
                            Err(error) => return error.into(),
                        };
                        if !category.is_empty()
                            && bounded_tool_text(&category, "news category", 256).is_err()
                        {
                            return "news category is malformed or oversized".into();
                        }
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                let cat = if category.is_empty() {
                                    None
                                } else {
                                    Some(category.as_str())
                                };
                                thinclaw_desktop_tools::tools::news::get_news(&c, cat, Some(limit))
                                    .await
                                    .map(dynamic_from_serializable)
                                    .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "news::search_news",
                    move |query: String| -> rhai::Dynamic {
                        if let Err(error) = bounded_tool_text(&query, "news query", 16 * 1024) {
                            return error.into();
                        }
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                thinclaw_desktop_tools::tools::news::search_news(&c, &query, None)
                                    .await
                                    .map(dynamic_from_serializable)
                                    .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "news::get_headlines",
                    move |country: String, limit: i64| -> rhai::Dynamic {
                        if let Err(error) = bounded_tool_text(&country, "country", 64) {
                            return error.into();
                        }
                        let limit = match bounded_limit(limit, 100) {
                            Ok(limit) => limit,
                            Err(error) => return error.into(),
                        };
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                thinclaw_desktop_tools::tools::news::get_headlines(
                                    &c,
                                    &country,
                                    Some(limit),
                                )
                                .await
                                .map(dynamic_from_serializable)
                                .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
            }

            // KNOWLEDGE
            {
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "knowledge::rag_query",
                    move |query: String| -> rhai::Dynamic {
                        if let Err(error) = bounded_tool_text(&query, "knowledge query", 16 * 1024)
                        {
                            return error.into();
                        }
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                thinclaw_desktop_tools::tools::knowledge::rag_query(
                                    &c, &query, None,
                                )
                                .await
                                .map(dynamic_from_serializable)
                                .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
            }

            // ECONOMICS
            {
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "economics::get_economic_data",
                    move |country: String| -> rhai::Dynamic {
                        if let Err(error) = bounded_tool_text(&country, "country", 256) {
                            return error.into();
                        }
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                thinclaw_desktop_tools::tools::economics::get_economic_data(
                                    &c, &country, None,
                                )
                                .await
                                .map(dynamic_from_serializable)
                                .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
            }

            // MODELS
            {
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "models::get_model_catalog",
                    move || -> rhai::Dynamic {
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                thinclaw_desktop_tools::tools::models::get_model_catalog(
                                    &c, None, None,
                                )
                                .await
                                .map(dynamic_from_serializable)
                                .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
            }

            // HEALTH
            {
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "health::search_medical_research",
                    move |query: String, limit: i64| -> rhai::Dynamic {
                        if let Err(error) = bounded_tool_text(&query, "medical query", 16 * 1024) {
                            return error.into();
                        }
                        let limit = match bounded_limit(limit, 100) {
                            Ok(limit) => limit,
                            Err(error) => return error.into(),
                        };
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                thinclaw_desktop_tools::tools::health::search_medical_research(
                                    &c,
                                    &query,
                                    Some(limit),
                                )
                                .await
                                .map(dynamic_from_serializable)
                                .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
            }

            // AI TOOLS
            {
                let c = client.clone();
                sandbox.engine_mut().register_fn(
                    "ai_tools::summarize_text",
                    move |text: String, length: String| -> rhai::Dynamic {
                        if text.is_empty()
                            || text.len() > MAX_TOOL_JSON_BYTES
                            || text.contains('\0')
                        {
                            return "text to summarize is missing, malformed, or oversized".into();
                        }
                        if !length.is_empty()
                            && bounded_tool_text(&length, "summary length", 64).is_err()
                        {
                            return "summary length is malformed or oversized".into();
                        }
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                let len_opt = if length.is_empty() {
                                    None
                                } else {
                                    Some(length.as_str())
                                };
                                thinclaw_desktop_tools::tools::ai_tools::summarize_text(
                                    &c, &text, len_opt,
                                )
                                .await
                                .map(dynamic_from_serializable)
                                .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
            }

            info!("[sandbox_factory] bounded MCP remote tools registered");
        } else {
            warn!("[sandbox_factory] invalid MCP configuration; remote tools unavailable");
        }
    }

    Some(sandbox)
}

#[cfg(test)]
mod tests {
    use super::{detect_tools_used, read_managed_document, MAX_DOCUMENT_RESULT_BYTES};

    #[test]
    fn detects_called_builtins() {
        let script = r#"
            let r = web_search("rust async");
            let docs = rag_search(query);
            let p = finance::get_stock_price("AAPL");
        "#;
        let tools = detect_tools_used(script);
        assert!(tools.contains(&"web_search".to_string()));
        assert!(tools.contains(&"rag_search".to_string()));
        assert!(tools.contains(&"finance::get_stock_price".to_string()));
        assert!(!tools.contains(&"read_file".to_string()));
    }

    #[test]
    fn ignores_unmatched_and_partial_idents() {
        // A reference without a following call, and a longer identifier that
        // merely contains a builtin name, must not be reported.
        let script = r#"
            let calculator_total = 5;
            print(calculator_total);
            let web_search_disabled = true;
        "#;
        let tools = detect_tools_used(script);
        assert!(!tools.contains(&"calculator".to_string()));
        assert!(!tools.contains(&"web_search".to_string()));
    }

    #[test]
    fn empty_script_has_no_tools() {
        assert!(detect_tools_used("").is_empty());
    }

    #[test]
    fn managed_document_reads_are_confined_bounded_and_utf8_safe() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("ok.txt"), "hello").unwrap();
        assert_eq!(
            read_managed_document(Some(directory.path()), "documents/ok.txt"),
            "hello"
        );
        assert!(
            read_managed_document(Some(directory.path()), "../ok.txt").starts_with("Read error")
        );
        assert!(
            read_managed_document(Some(directory.path()), "/tmp/ok.txt").starts_with("Read error")
        );

        std::fs::write(
            directory.path().join("unicode.txt"),
            "🦀".repeat(MAX_DOCUMENT_RESULT_BYTES),
        )
        .unwrap();
        let truncated = read_managed_document(Some(directory.path()), "unicode.txt");
        assert!(truncated.is_char_boundary(truncated.len()));
        assert!(truncated.ends_with("... (truncated)"));
    }

    #[cfg(unix)]
    #[test]
    fn managed_document_reads_reject_links() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            directory.path().join("symlink.txt"),
        )
        .unwrap();
        std::fs::hard_link(
            outside.path().join("secret.txt"),
            directory.path().join("hardlink.txt"),
        )
        .unwrap();

        assert!(
            read_managed_document(Some(directory.path()), "symlink.txt").starts_with("Read error")
        );
        assert!(
            read_managed_document(Some(directory.path()), "hardlink.txt").starts_with("Read error")
        );
    }
}
