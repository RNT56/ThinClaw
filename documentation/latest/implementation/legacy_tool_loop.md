# Legacy Tool Loop — Deleted Code Archive

> **Removed from:** `src-tauri/src/rig_lib/orchestrator.rs`  
> **Removed on:** 2026-02-22  
> **Reason:** Unified with the sandbox/Rhai execution path (TODO ③)  
> **Replaced by:** `run_sandbox_loop` + `build_sandbox_unconditional`

---

## Overview

The legacy tool loop used `<tool_code>` XML tags containing JSON tool-call descriptors.
The LLM would emit a block like:

```xml
<tool_code>
{
  "name": "web_search",
  "arguments": { "query": "..." }
}
</tool_code>
```

The orchestrator would parse the JSON, execute the tool inline (hardcoded `match`),
and inject the result back as a `<tool_result>` user message for the next LLM turn.

This was replaced by the Rhai sandbox path, which uses `<rhai_code>` tags and executes
scripts in a sandboxed Rhai engine with registered tool functions.

---

## How `run_turn` dispatched to the legacy path

At the bottom of `run_turn`, after the summarization and context-collection phases,
the old code had a two-branch dispatch:

```rust
// --- SANDBOX MODE (new) ---
if let Some(sandbox) = sandbox {
    Self::run_sandbox_loop(
        &tx, &rig_clone, &sandbox, &perms,
        &final_history, &all_doc_ids, &current_docs,
        &project_id_clone, &conversation_id_clone,
        &persona_instructions, &query,
    ).await;
    return;
}

// --- LEGACY TOOL MODE (existing <tool_code> parsing) ---
Self::run_legacy_tool_loop(
    &tx, &rig_clone, &perms,
    &final_history, &all_doc_ids, &all_doc_names,
    &current_docs, &project_id_clone, &conversation_id_clone,
    &persona_instructions, &query,
).await;
```

The new code removes the legacy branch and ensures a sandbox is always available
via `build_sandbox_unconditional()`.

Note: The legacy path also required `all_doc_names` (a `Vec<String>` of document
display names collected alongside `all_doc_ids`). That variable is no longer
collected in `run_turn` since the sandbox path doesn't use it.

---

## Full deleted method: `run_legacy_tool_loop`

```rust
// -----------------------------------------------------------------------
// Legacy tool loop (existing <tool_code> parsing — unchanged logic)
// -----------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn run_legacy_tool_loop(
    tx: &mpsc::Sender<Result<crate::rig_lib::unified_provider::ProviderEvent, String>>,
    rig: &Arc<RigManager>,
    perms: &ToolPermissions,
    final_history: &[crate::chat::Message],
    all_doc_ids: &[String],
    _all_doc_names: &[String],
    _current_docs: &Option<Vec<crate::chat::AttachedDoc>>,
    project_id: &Option<String>,
    conversation_id: &Option<String>,
    persona_instructions: &str,
    query: &str,
) {
    use crate::rig_lib::unified_provider::ProviderEvent;

    let max_turns = 5;
    let mut current_turn = 0;
    let mut conversation: Vec<serde_json::Value> = Vec::new();

    // 1. Dynamic System Prompt
    let mut tools_desc = String::from("AVAILABLE TOOLS:\n");
    if perms.allow_web_search {
        tools_desc.push_str("- web_search(query: str): Search internet for real-time info.\n");
    }
    if perms.allow_file_search {
        tools_desc.push_str("- rag_search(query: str): Search project documents/codebase.\n");
        tools_desc.push_str(
            "- read_file(path: str, force_ocr: bool?): Read file content. Set force_ocr to true only if standard text extraction is failing or garbage.\n",
        );
    }
    if perms.allow_image_gen {
        tools_desc.push_str("- generate_image(prompt: str): Generate an image.\n");
    }

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let search_rules = if perms.force_web_search {
        "CORE RULES:\n\
         1. **ALWAYS SEARCH**: The user has explicitly enabled web search. You MUST use `web_search` for every query that could benefit from external information. Only skip search for pure greetings.\n\
         2. **FORMALIZE QUERIES**: Transform vague user prompts into precise search queries before calling `web_search`."
    } else {
        "CORE RULES:\n\
         1. **REPLY DIRECTLY** for greetings, code, creative writing, general knowledge, opinions, or follow-up chat.\n\
         2. If the user needs real-time information (today's news, live prices, current events) or explicitly asks to search, use tools.\n\
         3. When in doubt about whether to use a tool, reply directly. Only call a tool if you are confident the answer requires fresh external data."
    };

    let system_prompt = format!(
        r#"{}. 
Current Date: {}

{}

TOOL USAGE:
To use a tool, output valid JSON inside <tool_code> tags.
Example:
<tool_code>
{{
  "name": "web_search",
  "arguments": {{ "query": "..." }}
}}
</tool_code>

{}"#,
        persona_instructions, date, search_rules, tools_desc
    );

    conversation.push(json!({
        "role": "system",
        "content": system_prompt
    }));

    // 2. History Conversion
    for msg in final_history {
        conversation.push(json!({
            "role": msg.role,
            "content": msg.content
        }));
    }

    // 3. Current User Query and Context Collection (Resolving Paths for Tools)
    let mut doc_info = Vec::new();
    let mut visual_messages = Vec::new();
    if !all_doc_ids.is_empty() {
        if let Some(app) = &rig.app_handle {
            use tauri::Manager;
            let pool = app.state::<sqlx::SqlitePool>();
            // Build dynamic IN query
            let placeholders = all_doc_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let query_str = format!(
                "SELECT id, path, hash FROM documents WHERE id IN ({})",
                placeholders
            );
            let mut db_query = sqlx::query_as::<_, (String, String, String)>(&query_str);
            for id in all_doc_ids {
                db_query = db_query.bind(id);
            }

            match db_query.fetch_all(pool.inner()).await {
                Ok(docs) => {
                    for (_id, path, hash) in docs {
                        let name = std::path::Path::new(&path)
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "unknown_file".to_string());
                        doc_info.push(format!("{} (at {})", name, path));

                        // 1. Check if the file itself is an image
                        let path_lower = path.to_lowercase();
                        let is_direct_image = path_lower.ends_with(".png")
                            || path_lower.ends_with(".jpg")
                            || path_lower.ends_with(".jpeg")
                            || path_lower.ends_with(".webp");

                        let mut image_injected = false;
                        if is_direct_image {
                            if let Ok(bytes) = std::fs::read(&path) {
                                use base64::Engine;
                                let b64 =
                                    base64::engine::general_purpose::STANDARD.encode(bytes);
                                let mime = if path_lower.ends_with(".png") {
                                    "image/png"
                                } else if path_lower.ends_with(".webp") {
                                    "image/webp"
                                } else {
                                    "image/jpeg"
                                };
                                visual_messages.push(json!({
                                      "role": "user",
                                      "content": [
                                          { "type": "text", "text": format!("Attached Image ({}):", path) },
                                          { "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", mime, b64) } }
                                      ]
                                  }));
                                image_injected = true;
                            }
                        }

                        if !image_injected {
                            if let Ok(app_data_dir) = app.path().app_data_dir() {
                                let preview_path =
                                    app_data_dir.join("previews").join(format!("{}.jpg", hash));
                                if preview_path.exists() {
                                    if let Ok(bytes) = std::fs::read(preview_path) {
                                        use base64::Engine;
                                        let b64 = base64::engine::general_purpose::STANDARD
                                            .encode(bytes);
                                        visual_messages.push(json!({
                                             "role": "user",
                                             "content": [
                                                 { "type": "text", "text": format!("Visual Preview of attached document ({}):", path) },
                                                 { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", b64) } }
                                             ]
                                         }));
                                    }
                                }
                            }
                        }

                        // 3. Auto-Injection (simplified)
                        if !path_lower.ends_with(".pdf") && !is_direct_image {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if content.len() < 12000 {
                                    doc_info.push(format!(
                                        "\n[Direct Content of {}]:\n{}\n",
                                        name, content
                                    ));
                                }
                            }
                        }
                    }
                }
                Err(e) => eprintln!("[orchestrator] Error resolving doc paths: {}", e),
            }
        }
    }

    let mut effective_query = query.to_string();

    if let Some(pid) = project_id {
        let mut context_str = format!("Project Context ID: {}\n", pid);
        if perms.allow_file_search {
            if let Some(app) = &rig.app_handle {
                use tauri::Manager;
                let pool = app.state::<sqlx::SqlitePool>();
                let files = crate::rag::list_project_files(pool.inner(), pid).await;
                if !files.is_empty() {
                    let list = if files.len() > 50 {
                        let subset = files[..50].join("\n- ");
                        format!("- {}\n... ({} more files)", subset, files.len() - 50)
                    } else {
                        files.join("\n- ")
                    };
                    context_str
                        .push_str(&format!("\n[AVAILABLE PROJECT FILES]:\n- {}\n", list));
                }
            }
        }
        effective_query = format!("{}\nRequest: {}", context_str, query);
    }

    if !doc_info.is_empty() {
        effective_query = format!(
            "[CURRENT CHAT ATTACHMENTS]:\n{}\n\n{}",
            doc_info.join("\n"),
            effective_query
        );
    }

    // Inject Visual Previews
    for vmsg in visual_messages {
        conversation.push(vmsg);
    }

    // Start turn with a strong grounding injection if searching is allowed
    let final_query = if perms.force_web_search {
        format!(
            "**SEARCH MODE ACTIVE**: Research this request using `web_search`. \
             Formalize the query and search.\n\nRequest: {}",
            effective_query
        )
    } else if perms.allow_web_search {
        format!(
            "Respond to this request. Only use tools if the request genuinely requires \
             real-time or external data you don't have. Otherwise reply directly.\n\n\
             Request: {}",
            effective_query
        )
    } else {
        effective_query
    };

    conversation.push(json!({
        "role": "user",
        "content": final_query
    }));

    // 4. ReAct Loop
    let mut _final_answer_streaming = false;
    let _ = tx
        .send(Ok(ProviderEvent::Content(
            "\n<scrappy_status type=\"thinking\" />\n".into(),
        )))
        .await;

    while current_turn < max_turns {
        if rig.is_cancelled() {
            let _ = tx
                .send(Ok(ProviderEvent::Content("\n[Stopped]".into())))
                .await;
            break;
        }
        current_turn += 1;
        let mut full_response = String::new();
        let mut buffer = String::new();
        let mut tool_detected = false;

        use futures::StreamExt;
        let mut stream = match rig
            .provider
            .stream_raw_completion(conversation.clone(), Some(0.1))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(Err(format!("Provider Error: {}", e))).await;
                break;
            }
        };

        while let Some(chunk_res) = stream.next().await {
            if rig.is_cancelled() {
                let _ = tx
                    .send(Ok(ProviderEvent::Content("\n[Stopped]".into())))
                    .await;
                return;
            }
            match chunk_res {
                Ok(event) => match event {
                    ProviderEvent::Content(token) => {
                        full_response.push_str(&token);
                        buffer.push_str(&token);

                        if buffer.contains("<tool_code>") {
                            tool_detected = true;
                            if buffer.ends_with("<tool_code>") {
                                let _ = tx
                                    .send(Ok(ProviderEvent::Content(
                                        "\n<scrappy_status type=\"thinking\" />\n".into(),
                                    )))
                                    .await;
                            }
                        } else {
                            if !tool_detected {
                                let _ = tx.send(Ok(ProviderEvent::Content(token))).await;
                                _final_answer_streaming = true;
                            }
                        }
                    }
                    ProviderEvent::Usage(u) => {
                        let _ = tx.send(Ok(ProviderEvent::Usage(u))).await;
                    }
                    ProviderEvent::ContextUpdate(c) => {
                        let _ = tx.send(Ok(ProviderEvent::ContextUpdate(c))).await;
                    }
                },
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                }
            }
        }

        if !tool_detected {
            break;
        }

        // Parse Tool
        let mut tool_executed = false;
        if let Some(start) = full_response.find("<tool_code>") {
            if let Some(end) = full_response.find("</tool_code>") {
                let json_str = &full_response[start + 11..end].trim();
                let json_str = if json_str.starts_with("```json") {
                    json_str
                        .trim_start_matches("```json")
                        .trim_end_matches("```")
                        .trim()
                } else if json_str.starts_with("```") {
                    json_str
                        .trim_start_matches("```")
                        .trim_end_matches("```")
                        .trim()
                } else {
                    json_str
                };

                let tool_call = match serde_json::from_str::<serde_json::Value>(json_str) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("Failed to parse tool JSON: {} in Turn {}", e, current_turn);
                        if !_final_answer_streaming {
                            let _ = tx
                                .send(Ok(ProviderEvent::Content(
                                    "\n[Tool Parse Error - Proceeding with answer]\n".into(),
                                )))
                                .await;
                        }
                        break;
                    }
                };

                conversation.push(json!({
                    "role": "assistant",
                    "content": full_response
                }));

                // Tool execution
                let name = tool_call["name"].as_str().unwrap_or("");
                let args = tool_call["arguments"].clone();
                let allowed_web = perms.allow_web_search;
                let allowed_file = perms.allow_file_search;
                let allowed_img = perms.allow_image_gen;
                let result = match name {
                    "web_search" if allowed_web => {
                        let q = args["query"].as_str().unwrap_or("");
                        let _ = tx
                            .send(Ok(ProviderEvent::Content(
                                format!(
                                    "\n<scrappy_status type=\"web_search\" query=\"{}\" />\n",
                                    q
                                )
                                .into(),
                            )))
                            .await;
                        rig.explicit_search(q).await
                    }
                    "rag_search" if allowed_file => {
                        let q = args["query"].as_str().unwrap_or("");
                        let _ = tx
                            .send(Ok(ProviderEvent::Content(
                                format!(
                                    "\n<scrappy_status type=\"rag_search\" query=\"{}\" />\n",
                                    q
                                )
                                .into(),
                            )))
                            .await;
                        if let Some(app) = &rig.app_handle {
                            use tauri::Manager;
                            let context_res = crate::rag::retrieve_context_internal(
                                rig.app_handle.clone(),
                                app.state::<crate::sidecar::SidecarManager>().inner(),
                                app.state::<sqlx::SqlitePool>().inner().clone(),
                                app.state::<crate::vector_store::VectorStoreManager>()
                                    .inner()
                                    .clone(),
                                app.state::<crate::reranker::RerankerWrapper>().inner(),
                                q.to_string(),
                                conversation_id.clone(),
                                if all_doc_ids.is_empty() {
                                    None
                                } else {
                                    Some(all_doc_ids.to_vec())
                                },
                                project_id.clone(),
                            )
                            .await;
                            match context_res {
                                Ok(r) => r.join("\n\n"),
                                Err(e) => format!("Error: {}", e),
                            }
                        } else {
                            "App state missing".into()
                        }
                    }
                    "read_file" if allowed_file => {
                        let path = args["path"].as_str().unwrap_or("");
                        let _ = tx.send(Ok(ProviderEvent::Content(
                            format!("\n<scrappy_status type=\"tool_call\" query=\"Reading {}\" />\n", path).into(),
                        ))).await;

                        // Path sandboxing: resolve to canonical path and validate
                        let requested = std::path::Path::new(path);

                        // Block path traversal attempts
                        if path.contains("..") {
                            "Access denied: path traversal not allowed".into()
                        } else if let Ok(canonical) = requested.canonicalize() {
                            // Allow reads within: app data dir (documents), home dir
                            let is_allowed = {
                                let home_ok = std::env::var("HOME")
                                    .ok()
                                    .map(|h| canonical.starts_with(std::path::Path::new(&h)))
                                    .unwrap_or(false);
                                let app_ok = rig
                                    .app_handle
                                    .as_ref()
                                    .and_then(|app| {
                                        use tauri::Manager;
                                        app.path().app_data_dir().ok()
                                    })
                                    .map(|d| canonical.starts_with(&d))
                                    .unwrap_or(false);
                                home_ok || app_ok
                            };

                            if !is_allowed {
                                format!(
                                    "Access denied: path outside allowed directories ({})",
                                    canonical.display()
                                )
                            } else if let Ok(c) = std::fs::read_to_string(&canonical) {
                                if c.len() > 20000 {
                                    format!("{}... (truncated)", &c[..20000])
                                } else {
                                    c
                                }
                            } else {
                                "Read failed (binary file or permission denied)".into()
                            }
                        } else {
                            "File not found".into()
                        }
                    }
                    "generate_image" if allowed_img => {
                        let _ = tx
                            .send(Ok(ProviderEvent::Content(
                                "\n<scrappy_status type=\"image_gen\" />\n".into(),
                            )))
                            .await;
                        "Image Generation Triggered".to_string()
                    }
                    _ => "Unknown tool or permission denied".to_string(),
                };

                conversation.push(json!({
                    "role": "user",
                    "content": format!("<tool_result>\n{}\n</tool_result>", result)
                }));
                tool_executed = true;
            }
        }

        if !tool_executed {
            break;
        }
    } // End Loop
}
```

---

## Key differences from the sandbox path

| Aspect | Legacy `<tool_code>` | Sandbox `<rhai_code>` |
|--------|---------------------|-----------------------|
| Tag format | `<tool_code>{ JSON }</tool_code>` | `<rhai_code>script</rhai_code>` |
| Execution | Inline `match name { … }` in orchestrator | Rhai VM with registered functions |
| Multi-step | One tool per turn (loop re-prompts) | Multiple calls in one script |
| Tool routing | Hardcoded match arms | `sandbox_factory.rs` registrations |
| Status events | Inline `tx.send(ProviderEvent::Content(xml))` | `OrchestratorStatusReporter` bridge |
| Path sandboxing | Inline canonicalize + HOME/app_data check | Rhai sandbox config restrictions |
| Document resolution | Inline DB queries + base64 encoding | Handled by RAG tools in sandbox |
| Project file listing | Inline `crate::rag::list_project_files()` | Not needed (tools self-discover) |
| Result injection | Raw string in `<tool_result>` | Summarized via `summarize_arbitrary_json` |

## Dependencies for reimplementation

If you ever need to bring this back, you would need:

1. **`all_doc_names: Vec<String>`** — re-add collection in `run_turn` alongside `all_doc_ids`
2. **`crate::rag::list_project_files`** — still exists, just not called from orchestrator
3. **`crate::rag::retrieve_context_internal`** — still exists
4. **`rig.explicit_search()`** — still exists on `RigManager`
5. The `<tool_code>` / `</tool_code>` tag detection in the streaming loop (vs `<rhai_code>`)
6. JSON parsing of the tool call (`serde_json::from_str`)
7. Re-add the legacy branch in `run_turn`:
   ```rust
   if let Some(sandbox) = sandbox {
       Self::run_sandbox_loop(/* ... */).await;
       return;
   }
   Self::run_legacy_tool_loop(/* ... */).await;
   ```
