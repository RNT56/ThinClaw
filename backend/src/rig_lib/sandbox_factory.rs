use crate::rig_lib::RigManager;
use scrappy_mcp_tools::events::StatusReporter;
use scrappy_mcp_tools::sandbox::{Sandbox, SandboxConfig};
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

/// Configuration for the MCP sandbox factory
#[derive(Clone, Debug, Default)]
pub struct McpOrchestratorConfig {
    /// Base URL of the FastAPI MCP server
    pub mcp_base_url: Option<String>,
    /// JWT bearer token for the MCP server
    pub mcp_auth_token: Option<String>,
    /// Whether sandbox mode is enabled
    pub sandbox_enabled: bool,
    /// Path to user-defined skills
    pub user_skills_path: Option<std::path::PathBuf>,
    /// Path to built-in skills
    pub builtin_skills_path: Option<std::path::PathBuf>,
}

/// Create a configured Sandbox instance with all host tools registered.
///
/// This factory function is used by:
/// 1. `Orchestrator` (for Rig agents)
/// 2. `McpRequestHandler` (for OpenClaw requests via IPC)
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
        use scrappy_mcp_tools::skills::manager::SkillManager;
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
                        rhai::serde::to_dynamic(serde_json::Value::Array(list)).unwrap_or_default()
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
                let args: serde_json::Value = serde_json::from_str(&args_json).unwrap_or(json!({}));
                let args_map = match args.as_object() {
                    Some(obj) => obj
                        .clone()
                        .into_iter()
                        .collect::<std::collections::HashMap<String, serde_json::Value>>(),
                    None => std::collections::HashMap::new(),
                };

                let script = match manager.prepare_script(&id, &args_map) {
                    Ok(s) => s,
                    Err(e) => return format!("Skill Error: {}", e).into(),
                };

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

    // -- Register host tool: web_search --
    let rig_for_ws = rig.clone();
    sandbox
        .engine_mut()
        .register_fn("web_search", move |query: String| -> rhai::Dynamic {
            let rig = rig_for_ws.clone();
            // Rhai is synchronous — bridge to async via tokio block_in_place
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(async { rig.explicit_search(&query).await })
            });
            eprintln!(
                "[DEBUG web_search] Returning {} chars to Rhai. First 200: {:?}",
                result.len(),
                &result[..std::cmp::min(200, result.len())]
            );
            rhai::Dynamic::from(result)
        });

    // -- Register host tool: rag_search --
    let rig_for_rag = rig.clone();
    sandbox
        .engine_mut()
        .register_fn("rag_search", move |query: String| -> rhai::Dynamic {
            let rig = rig_for_rag.clone();
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if let Some(app) = &rig.app_handle {
                        use tauri::Manager;
                        let sidecar = app.state::<crate::sidecar::SidecarManager>();
                        let pool = app.state::<sqlx::SqlitePool>();
                        let store = app.state::<crate::vector_store::VectorStoreManager>();
                        let reranker = app.state::<crate::reranker::RerankerWrapper>();
                        match crate::rag::retrieve_context_internal(
                            Some(app.clone()),
                            sidecar.inner(),
                            pool.inner().clone(),
                            store.inner().clone(),
                            reranker.inner(),
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

    // -- Register host tool: read_file --
    sandbox
        .engine_mut()
        .register_fn("read_file", move |path: String| -> rhai::Dynamic {
            if std::path::Path::new(&path).exists() {
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        if content.len() > 20000 {
                            rhai::Dynamic::from(format!("{}... (truncated)", &content[..20000]))
                        } else {
                            rhai::Dynamic::from(content)
                        }
                    }
                    Err(e) => rhai::Dynamic::from(format!("Read error: {}", e)),
                }
            } else {
                rhai::Dynamic::from("File not found".to_string())
            }
        });

    // -- Register host tool: calculator (pure computation, no async needed) --
    // Returns full trace + result for transparency
    sandbox
        .engine_mut()
        .register_fn("calculator", |expr: String| -> rhai::Dynamic {
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
            let vars: std::collections::HashMap<String, f64> =
                serde_json::from_str(&vars_json).unwrap_or_default();
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
        scrappy_mcp_tools::McpClient::new(scrappy_mcp_tools::McpConfig {
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

    sandbox
        .engine_mut()
        .register_fn("search_tools", move |query: String| -> rhai::Dynamic {
            let client = mcp_client_for_discovery.clone();
            let upath = user_skills_path.clone();
            let bpath = builtin_skills_path.clone();

            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    use scrappy_mcp_tools::skills::manager::SkillManager;
                    let skill_manager = upath.map(|up| SkillManager::new(up, bpath));

                    let sr = crate::rig_lib::tool_discovery::search_all_tools(
                        &query,
                        client.as_ref(),
                        skill_manager.as_ref(),
                        true, // include host tools
                    )
                    .await;

                    serde_json::to_string(&sr).unwrap_or_else(|_| "{}".to_string())
                })
            });
            rhai::Dynamic::from(result)
        });

    // -- save_skill(id, script, description) → persist skill --
    let user_skills_path_save = mcp_config.user_skills_path.clone();
    let builtin_skills_path_save = mcp_config.builtin_skills_path.clone();

    sandbox.engine_mut().register_fn(
        "save_skill",
        move |id: String, script: String, description: String| -> String {
            // Basic validation
            let id = id.trim();
            let script = script.trim();
            if id.is_empty() || script.is_empty() {
                return "Error: ID and Script cannot be empty".to_string();
            }

            let upath = user_skills_path_save.clone();
            let bpath = builtin_skills_path_save.clone();
            let id = id.to_string();
            let script = script.to_string();
            let description = description.clone();

            let result = tokio::task::block_in_place(|| {
                use scrappy_mcp_tools::skills::manager::SkillManager;
                use scrappy_mcp_tools::skills::manifest::SkillManifest;

                if let Some(up) = upath {
                    // Construct manifest
                    let manifest = SkillManifest {
                        name: id.clone(),                 // Using ID as name for simplicity
                        description: description.clone(), // User description
                        version: "1.0.0".to_string(),
                        author: Some("Agent".to_string()),
                        tools_used: vec![], // TODO: Parse logic
                        parameters: vec![], // TODO: Allow passing params
                        script_file: format!("{}.rhai", id),
                    };

                    let manager = SkillManager::new(up, bpath);
                    match manager.save_skill(&id, manifest, &script) {
                        Ok(_) => format!("Skill '{}' saved successfully", id),
                        Err(e) => format!("Error saving skill: {}", e),
                    }
                } else {
                    "Error: User skills directory not configured".to_string()
                }
            });
            result
        },
    );

    // -- Register remote MCP tools (only when server URL is configured) --
    if let Some(base_url) = &mcp_config.mcp_base_url {
        let client_mcp_config = scrappy_mcp_tools::McpConfig {
            base_url: base_url.clone(),
            auth_token: mcp_config.mcp_auth_token.clone().unwrap_or_default(),
            timeout_ms: 30_000,
        };
        if let Ok(client) = scrappy_mcp_tools::McpClient::new(client_mcp_config) {
            // -- mcp_call(tool_name, args_json) → generic remote tool caller --
            let client_for_call = client.clone();
            sandbox.engine_mut().register_fn(
                "mcp_call",
                move |tool: String, args_json: String| -> rhai::Dynamic {
                    let c = client_for_call.clone();
                    let result = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            let args: serde_json::Value =
                                serde_json::from_str(&args_json).unwrap_or(json!({}));
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
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                scrappy_mcp_tools::tools::finance::get_stock_price(&c, &symbol)
                                    .await
                                    .map(|r| rhai::serde::to_dynamic(r).unwrap())
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
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                let cat = if category.is_empty() {
                                    None
                                } else {
                                    Some(category.as_str())
                                };
                                scrappy_mcp_tools::tools::news::get_news(
                                    &c,
                                    cat,
                                    Some(limit as usize),
                                )
                                .await
                                .map(|r| rhai::serde::to_dynamic(r).unwrap())
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
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                scrappy_mcp_tools::tools::news::search_news(&c, &query, None)
                                    .await
                                    .map(|r| rhai::serde::to_dynamic(r).unwrap())
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
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                scrappy_mcp_tools::tools::news::get_headlines(
                                    &c,
                                    &country,
                                    Some(limit as usize),
                                )
                                .await
                                .map(|r| rhai::serde::to_dynamic(r).unwrap())
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
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                scrappy_mcp_tools::tools::knowledge::rag_query(&c, &query, None)
                                    .await
                                    .map(|r| rhai::serde::to_dynamic(r).unwrap())
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
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                scrappy_mcp_tools::tools::economics::get_economic_data(
                                    &c, &country, None,
                                )
                                .await
                                .map(|r| rhai::serde::to_dynamic(r).unwrap())
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
                                scrappy_mcp_tools::tools::models::get_model_catalog(&c, None, None)
                                    .await
                                    .map(|r| rhai::serde::to_dynamic(r).unwrap())
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
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                scrappy_mcp_tools::tools::health::search_medical_research(
                                    &c,
                                    &query,
                                    Some(limit as usize),
                                )
                                .await
                                .map(|r| rhai::serde::to_dynamic(r).unwrap())
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
                        let c = c.clone();
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                let len_opt = if length.is_empty() {
                                    None
                                } else {
                                    Some(length.as_str())
                                };
                                scrappy_mcp_tools::tools::ai_tools::summarize_text(
                                    &c, &text, len_opt,
                                )
                                .await
                                .map(|r| rhai::serde::to_dynamic(r).unwrap())
                                .unwrap_or_else(|e| format!("Error: {}", e).into())
                            })
                        });
                        rhai::Dynamic::from(result)
                    },
                );
            }

            info!(
                "[sandbox_factory] MCP remote tools registered (mcp_call + typed bindings) → {}",
                base_url
            );
        } else {
            warn!(
                "[sandbox_factory] Failed to create McpClient, remote tools unavailable: {}",
                base_url
            );
        }
    }

    Some(sandbox)
}
