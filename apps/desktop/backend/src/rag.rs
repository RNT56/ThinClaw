use crate::sidecar::SidecarManager;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::handler::viewport::Viewport;
use futures::StreamExt;
use rand::{distributions::Alphanumeric, Rng};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use tauri::{AppHandle, Emitter, Manager, State};
use thinclaw_runtime_contracts::{
    AssetKind, AssetOrigin, DirectDocumentIngestResponse, DirectDocumentUploadResponse,
};

#[derive(Deserialize, Debug)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize, Debug)]
struct EmbeddingData {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

#[tauri::command]
#[specta::specta]
pub async fn direct_rag_upload_document(
    _app: tauri::AppHandle,
    file_store: tauri::State<'_, crate::file_store::FileStore>,
    pool: State<'_, SqlitePool>,
    file_bytes: Vec<u8>,
    filename: String,
) -> Result<DirectDocumentUploadResponse, String> {
    file_store
        .create_dir_all("documents")
        .await
        .map_err(|e| e.to_string())?;

    let safe_filename = std::path::Path::new(&filename)
        .file_name()
        .ok_or("Invalid filename")?
        .to_string_lossy();

    let id = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect::<String>();

    let final_filename = format!("{}_{}", id, safe_filename);
    let relative_path = format!("documents/{}", final_filename);
    file_store
        .write(&relative_path, &file_bytes)
        .await
        .map_err(|e| format!("Failed to save document: {}", e))?;
    let path = file_store.resolve_path(&relative_path).await;

    let mut metadata = HashMap::new();
    metadata.insert("original_filename".to_string(), filename);
    let asset = crate::direct_assets::DirectAssetStore::upsert(
        pool.inner(),
        crate::direct_assets::NewDirectAsset {
            id,
            kind: AssetKind::Document,
            origin: AssetOrigin::RagDocument,
            path: path.to_string_lossy().to_string(),
            mime_type: None,
            size_bytes: Some(file_bytes.len() as u64),
            sha256: None,
            prompt: None,
            provider: None,
            style_id: None,
            aspect_ratio: None,
            resolution: None,
            width: None,
            height: None,
            seed: None,
            thumbnail_path: None,
            is_favorite: false,
            tags: None,
            metadata,
        },
    )
    .await?;

    Ok(DirectDocumentUploadResponse {
        path: path.to_string_lossy().to_string(),
        asset,
    })
}

/// Helper function to extract content from a document, potentially using OCR for PDFs.
/// Returns (final_content, ocr_used)
pub async fn extract_document_content(
    app: &AppHandle,
    _sidecar: &SidecarManager,
    file_path: &str,
    buffer: &[u8],
    hash: &str,
    force_ocr_arg: bool,
) -> Result<(String, bool), String> {
    let mut force_ocr = force_ocr_arg;
    let path_lc = file_path.to_lowercase();
    let is_pdf = path_lc.ends_with(".pdf");

    let raw_content = if is_pdf {
        match pdf_extract::extract_text(file_path) {
            Ok(t) => t,
            Err(_) => {
                force_ocr = true;
                String::new()
            }
        }
    } else {
        String::from_utf8_lossy(buffer).to_string()
    };

    // Sanitize
    let content: String = raw_content.chars().filter(|&c| c != '\0').collect();

    // Garbage detection
    let is_garbage = if is_pdf && !force_ocr {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            true
        } else {
            let total = trimmed.len();
            let alphanumeric_chars = trimmed.chars().filter(|c| c.is_alphanumeric()).count();
            // If less than 25% alphanumeric, or extremely low text density for a file of this size
            let looks_like_scan = buffer.len() > 50000 && total < 1000;
            (alphanumeric_chars as f32 / total as f32) < 0.25 || looks_like_scan
        }
    } else {
        false
    };

    let mut ocr_text = String::new();
    let mut ocr_used = false;

    if is_pdf && (force_ocr || is_garbage) {
        println!("[rag] PDF needs robust extraction. (Empty/Garbage detected or Forced)");
        ocr_used = true;

        let (mut browser, mut handler) = Browser::launch(
            BrowserConfig::builder()
                .viewport(Viewport {
                    width: 1280,
                    height: 1800,
                    ..Default::default()
                })
                .build()
                .map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| e.to_string())?;

        // Ensure browser is closed on all paths (including errors)
        // by using a scope guard pattern.
        let browser_close_result: Result<(), String> = async {
        let _handle = tokio::spawn(async move { while let Some(_) = handler.next().await {} });

        let page = browser
            .new_page(&format!("file://{}", file_path))
            .await
            .map_err(|e| format!("Failed to open PDF in browser: {}", e))?;

        // Small wait for initial render
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        // Resolve the vision-capable chat endpoint through the shared runtime
        // snapshot/provider path. Do not probe sidecar state directly here.
        let ocr_endpoint: Option<(String, String, String)> = {
            use tauri::Manager;
            let config_mgr = app.state::<crate::config::ConfigManager>();
            let secret_store = app.state::<crate::secret_store::SecretStore>();
            let engine_manager = app.state::<crate::engine::EngineManager>();
            let sidecar_state = app.state::<SidecarManager>();
            let user_config = config_mgr.get_config();

            if let Ok(provider_cfg) = crate::chat::resolve_provider(
                &user_config,
                &secret_store,
                &sidecar_state,
                &engine_manager,
            )
            .await
            {
                let url = format!("{}/chat/completions", provider_cfg.base_url);
                Some((url, provider_cfg.token, provider_cfg.model_name))
            } else {
                None
            }
        };

        if let Some((url, token, model_name)) = ocr_endpoint {
            let client = reqwest::Client::new();

            // Extract up to 15 pages via Vision-OCR, with a 2-minute overall timeout
            // to prevent a slow/stuck LLM from blocking the ingestion pipeline.
            let ocr_future = async {
                for i in 1..=15 {
                    if i > 1 {
                        let _ = page.evaluate("window.scrollBy(0, 1800)").await;
                        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                    }

                    if let Ok(screenshot) = page
                        .screenshot(
                            chromiumoxide::page::ScreenshotParams::builder()
                                .format(
                                    chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Jpeg,
                                )
                                .quality(85)
                                .build(),
                        )
                        .await
                    {
                        // Save first page as preview if needed
                        if i == 1 {
                            {
                                let preview_rel = format!("previews/{}.jpg", hash);
                                let file_store = app.state::<crate::file_store::FileStore>();
                                let _ = file_store.write(&preview_rel, &screenshot).await;
                            }
                        }

                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(screenshot);

                        let body = serde_json::json!({
                            "model": model_name,
                            "messages": [
                                {
                                    "role": "user",
                                    "content": [
                                        { "type": "text", "text": "Transcribe all visible text in this image. Maintain the original structure. Output ONLY the text. If the page is blank or has no meaningful text, output [empty]." },
                                        { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", b64) } }
                                    ]
                                }
                            ]
                        });

                        if let Ok(resp) = client
                            .post(&url)
                            .header("Authorization", format!("Bearer {}", token))
                            .json(&body)
                            .send()
                            .await
                        {
                            if let Ok(json) = resp.json::<serde_json::Value>().await {
                                if let Some(transcription) =
                                    json["choices"][0]["message"]["content"].as_str()
                                {
                                    if transcription != "[empty]" && !transcription.trim().is_empty() {
                                        ocr_text.push_str(&format!("--- Page {} ---\n", i));
                                        ocr_text.push_str(transcription);
                                        ocr_text.push_str("\n\n");
                                    } else if i > 1 && transcription.contains("[empty]") {
                                        break;
                                    }
                                }
                            }
                        }
                    } else {
                        break;
                    }
                }
            };

            match tokio::time::timeout(std::time::Duration::from_secs(120), ocr_future).await {
                Ok(()) => {}
                Err(_) => {
                    println!(
                        "[rag] OCR timed out after 120s, using {} chars of partial results",
                        ocr_text.len()
                    );
                }
            }
        } else {
            println!("[rag] WARNING: No vision-capable chat backend available for OCR. PDF will be ingested with text-only extraction. Configure a chat provider in Settings to enable Vision-OCR.");
        }
        Ok(())
        }.await;

        // Always close browser, even if OCR errored
        let _ = browser.close().await;

        // Propagate any error from the OCR block
        browser_close_result?;
    }

    // Always generate preview for PDFs (even if not using OCR) if not already existing
    if is_pdf {
        if let Ok(app_data_dir) = app.path().app_data_dir() {
            let preview_path = app_data_dir.join("previews").join(format!("{}.jpg", hash));
            if !preview_path.exists() {
                if !ocr_used {
                    let (mut browser, mut handler) = Browser::launch(
                        BrowserConfig::builder()
                            .viewport(Viewport {
                                width: 1200,
                                height: 1600,
                                ..Default::default()
                            })
                            .build()
                            .map_err(|e| e.to_string())?,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    let _handle =
                        tokio::spawn(async move { while let Some(_) = handler.next().await {} });
                    let page = browser
                        .new_page(&format!("file://{}", file_path))
                        .await
                        .map_err(|e| e.to_string())?;
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if let Ok(screenshot) = page.screenshot(chromiumoxide::page::ScreenshotParams::builder().format(chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Jpeg).quality(80).build()).await {
                        let preview_rel = format!("previews/{}.jpg", hash);
                        let file_store = app.state::<crate::file_store::FileStore>();
                        let _ = file_store.write(&preview_rel, &screenshot).await;
                    }
                    let _ = browser.close().await;
                }
            }
        }
    }

    let final_content = if !ocr_text.is_empty() {
        if content.len() < 100 {
            ocr_text
        } else {
            format!("{}\n\n[OCR Supplemental Content]:\n{}", content, ocr_text)
        }
    } else {
        content
    };

    Ok((final_content, ocr_used))
}

#[tauri::command]
#[specta::specta]
pub async fn direct_rag_ingest_document(
    app: AppHandle,
    sidecar: State<'_, SidecarManager>,
    pool: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    inference_router: State<'_, crate::inference::router::InferenceRouter>,
    file_path: String,
    chat_id: Option<String>,
    project_id: Option<String>,
    embedding_model_path: Option<String>,
) -> Result<DirectDocumentIngestResponse, String> {
    println!(
        "[rag] direct_rag_ingest_document: start for {}, chat_id={:?}, project_id={:?}",
        &file_path, chat_id, project_id
    );

    let mut file = fs::File::open(&file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let mut hasher = Sha256::new();
    hasher.update(&buffer);
    let hash = format!("{:x}", hasher.finalize());

    let existing_doc: Option<(String, String)> =
        sqlx::query_as("SELECT id, status FROM documents WHERE hash = ?")
            .bind(&hash)
            .fetch_optional(pool.inner())
            .await
            .map_err(|e| e.to_string())?;

    if let Some((id, status)) = existing_doc {
        if status == "indexed" {
            println!(
                "[rag] Deduplication: Found existing indexed document (ID: {}). Updating scope/path...",
                id
            );
            sqlx::query("UPDATE documents SET path = ?, chat_id = ?, project_id = ?, updated_at = ? WHERE id = ?")
                .bind(&file_path)
                .bind(&chat_id)
                .bind(&project_id)
                .bind(chrono::Utc::now().timestamp())
                .bind(&id)
                .execute(pool.inner())
                .await
                .map_err(|e| e.to_string())?;

            let mut metadata = HashMap::new();
            metadata.insert("hash".to_string(), hash);
            if let Some(chat_id) = chat_id.as_ref() {
                metadata.insert("chat_id".to_string(), chat_id.clone());
            }
            if let Some(project_id) = project_id.as_ref() {
                metadata.insert("project_id".to_string(), project_id.clone());
            }
            let asset = crate::direct_assets::DirectAssetStore::upsert(
                pool.inner(),
                crate::direct_assets::NewDirectAsset {
                    id: id.clone(),
                    kind: AssetKind::Document,
                    origin: AssetOrigin::RagDocument,
                    path: file_path.clone(),
                    mime_type: None,
                    size_bytes: Some(buffer.len() as u64),
                    sha256: Some(metadata["hash"].clone()),
                    prompt: None,
                    provider: None,
                    style_id: None,
                    aspect_ratio: None,
                    resolution: None,
                    width: None,
                    height: None,
                    seed: None,
                    thumbnail_path: None,
                    is_favorite: false,
                    tags: None,
                    metadata,
                },
            )
            .await?;

            return Ok(DirectDocumentIngestResponse {
                document_id: id,
                asset,
            });
        }
    }

    let (final_content, ocr_used) =
        extract_document_content(&app, &sidecar, &file_path, &buffer, &hash, false).await?;

    if ocr_used {
        println!("[rag] Vision-OCR was used for extraction.");
    }

    if final_content.trim().is_empty() {
        return Err("Document appears empty even after OCR attempts.".to_string());
    }

    let doc_id: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    sqlx::query("INSERT INTO documents (id, path, hash, status, created_at, updated_at, chat_id, project_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&doc_id)
        .bind(&file_path)
        .bind(&hash)
        .bind("processing")
        .bind(now)
        .bind(now)
        .bind(&chat_id)
        .bind(&project_id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    let chunk_size = 1000;
    let overlap = 100;

    let chars: Vec<char> = final_content.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0;

    if chars.is_empty() {
        chunks.push("".to_string());
    } else {
        while start < chars.len() {
            let end = std::cmp::min(start + chunk_size, chars.len());
            let chunk_text: String = chars[start..end].iter().collect();
            chunks.push(chunk_text);
            if end == chars.len() {
                break;
            }
            start += chunk_size - overlap;
        }
    }

    // ── Try InferenceRouter embedding backend first ───────────────────────
    let embedding_backend = inference_router.embedding_backend().await;

    let chunks_with_index: Vec<(usize, String)> = chunks.into_iter().enumerate().collect();
    let total_chunks = chunks_with_index.len();

    // Determine the scope for vector storage
    let scope = crate::vector_store::VectorStoreManager::scope_for(&project_id, &chat_id);
    let scoped_store = vector_manager
        .get(&scope)
        .map_err(|e| format!("Failed to get vector store for scope {:?}: {}", scope, e))?;

    println!(
        "[rag] Starting ingestion of {} chunks into scope {:?}",
        total_chunks, scope
    );

    if let Some(backend) = embedding_backend {
        // ── Cloud/configured embedding backend path ──────────────────────
        println!(
            "[rag] Using InferenceRouter embedding backend: {}",
            backend.info().display_name
        );

        let stream = futures::stream::iter(chunks_with_index)
            .map(|(i, chunk_text)| {
                let pool = pool.clone();
                let scoped_store = scoped_store.clone();
                let backend = backend.clone();
                let doc_id = doc_id.clone();

                async move {
                    if chunk_text.trim().is_empty() {
                        return Ok(());
                    }

                    let embedding = backend
                        .embed(chunk_text.clone())
                        .await
                        .map_err(|e| format!("Embedding failed for chunk {}: {}", i, e))?;

                    let bytes: Vec<u8> = embedding
                        .iter()
                        .flat_map(|f| f.to_le_bytes())
                        .collect();
                    let chunk_id = format!("{}-{}", doc_id, i);

                    let rowid: i64 = sqlx::query_scalar("INSERT INTO chunks (id, document_id, content, chunk_index, embedding) VALUES (?, ?, ?, ?, ?) RETURNING rowid")
                        .bind(&chunk_id)
                        .bind(&doc_id)
                        .bind(chunk_text)
                        .bind(i as i64)
                        .bind(&bytes)
                        .fetch_one(pool.inner())
                        .await
                        .map_err(|e| format!("Database insert failed: {}", e))?;

                    if let Err(e) = scoped_store.add(rowid as u64, &embedding) {
                        return Err(format!("Vector store index failed: {}", e));
                    }
                    Ok(())
                }
            })
            .buffer_unordered(5);

        let results: Vec<Result<(), String>> = stream.collect().await;

        for res in &results {
            if let Err(e) = res {
                // Roll back both chunks and document to prevent orphans
                let _ = sqlx::query("DELETE FROM chunks WHERE document_id = ?")
                    .bind(&doc_id)
                    .execute(pool.inner())
                    .await;
                let _ = sqlx::query("DELETE FROM documents WHERE id = ?")
                    .bind(&doc_id)
                    .execute(pool.inner())
                    .await;
                return Err(format!("Ingestion failed (rolling back): {}", e));
            }
        }
    } else {
        // ── Sidecar fallback path ────────────────────────────────────────
        // Ensure the embedding server is alive; start on demand if not.
        let (port, token) = {
            let maybe_live = if let Some((p, t)) = sidecar.get_embedding_config() {
                let health_url = format!("http://127.0.0.1:{}/health", p);
                let alive = reqwest::Client::new()
                    .get(&health_url)
                    .timeout(std::time::Duration::from_secs(2))
                    .send()
                    .await
                    .is_ok();
                if alive {
                    Some((p, t))
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(cfg) = maybe_live {
                cfg
            } else {
                let model_path = embedding_model_path
                    .clone()
                    .ok_or_else(|| "Embedding server is not running and no embedding_model_path provided. Please select an embedding model in Settings.".to_string())?;

                println!(
                    "[rag] Embedding server not alive — starting on demand with model: {}",
                    model_path
                );

                crate::sidecar::start_embedding_server_core(
                    &app,
                    &sidecar,
                    &vector_manager,
                    model_path.clone(),
                )
                .await
                .map_err(|e| format!("Failed to auto-start embedding server: {}", e))?;

                sidecar.get_embedding_config().ok_or_else(|| {
                    "Embedding server failed to start (no config after launch)".to_string()
                })?
            }
        };

        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/v1/embeddings", port);

        let stream = futures::stream::iter(chunks_with_index)
            .map(|(i, chunk_text)| {
                let pool = pool.clone();
                let scoped_store = scoped_store.clone();
                let client = client.clone();
                let url = url.clone();
                let token = token.clone();
                let doc_id = doc_id.clone();

                async move {
                    if chunk_text.trim().is_empty() {
                        return Ok(());
                    }

                    let response = client
                        .post(&url)
                        .header("Authorization", format!("Bearer {}", token))
                        .json(&serde_json::json!({
                            "input": chunk_text,
                            "model": "default"
                        }))
                        .send()
                        .await
                        .map_err(|e| format!("Request failed: {}", e))?;

                    if !response.status().is_success() {
                        let status = response.status();
                        let text = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "Could not read response text".to_string());
                        return Err(format!(
                            "Embedding failed for chunk {}: {} - Body: {}",
                            i, status, text
                        ));
                    }

                    let res_json: EmbeddingResponse = response
                        .json()
                        .await
                        .map_err(|e| format!("Parse error: {}", e))?;

                    if let Some(data) = res_json.data.first() {
                        let bytes: Vec<u8> = data
                            .embedding
                            .iter()
                            .flat_map(|f| f.to_le_bytes())
                            .collect();
                        let chunk_id = format!("{}-{}", doc_id, i);

                        let rowid: i64 = sqlx::query_scalar("INSERT INTO chunks (id, document_id, content, chunk_index, embedding) VALUES (?, ?, ?, ?, ?) RETURNING rowid")
                            .bind(&chunk_id)
                            .bind(&doc_id)
                            .bind(chunk_text)
                            .bind(i as i64)
                            .bind(&bytes)
                            .fetch_one(pool.inner())
                            .await
                            .map_err(|e| format!("Database insert failed: {}", e))?;

                        if let Err(e) = scoped_store.add(rowid as u64, &data.embedding) {
                            return Err(format!("Vector store index failed: {}", e));
                        }
                    }
                    Ok(())
                }
            })
            .buffer_unordered(5);

        let results: Vec<Result<(), String>> = stream.collect().await;

        for res in &results {
            if let Err(e) = res {
                // Roll back both chunks and document to prevent orphans
                let _ = sqlx::query("DELETE FROM chunks WHERE document_id = ?")
                    .bind(&doc_id)
                    .execute(pool.inner())
                    .await;
                let _ = sqlx::query("DELETE FROM documents WHERE id = ?")
                    .bind(&doc_id)
                    .execute(pool.inner())
                    .await;
                return Err(format!("Ingestion failed (rolling back): {}", e));
            }
        }
    }

    if let Err(e) = scoped_store.save() {
        let _ = sqlx::query("DELETE FROM documents WHERE id = ?")
            .bind(&doc_id)
            .execute(pool.inner())
            .await;
        return Err(format!("Failed to save vector index: {}", e));
    }

    sqlx::query("UPDATE documents SET status = 'indexed' WHERE id = ?")
        .bind(&doc_id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    let mut metadata = HashMap::new();
    metadata.insert("hash".to_string(), hash.clone());
    metadata.insert("ocr_used".to_string(), ocr_used.to_string());
    if let Some(chat_id) = chat_id.as_ref() {
        metadata.insert("chat_id".to_string(), chat_id.clone());
    }
    if let Some(project_id) = project_id.as_ref() {
        metadata.insert("project_id".to_string(), project_id.clone());
    }
    let asset = crate::direct_assets::DirectAssetStore::upsert(
        pool.inner(),
        crate::direct_assets::NewDirectAsset {
            id: doc_id.clone(),
            kind: AssetKind::Document,
            origin: AssetOrigin::RagDocument,
            path: file_path,
            mime_type: None,
            size_bytes: Some(buffer.len() as u64),
            sha256: Some(hash),
            prompt: None,
            provider: None,
            style_id: None,
            aspect_ratio: None,
            resolution: None,
            width: None,
            height: None,
            seed: None,
            thumbnail_path: None,
            is_favorite: false,
            tags: None,
            metadata,
        },
    )
    .await?;

    Ok(DirectDocumentIngestResponse {
        document_id: doc_id,
        asset,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn direct_rag_retrieve_context(
    app: tauri::AppHandle,
    sidecar: State<'_, SidecarManager>,
    pool: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    reranker: State<'_, crate::reranker::RerankerWrapper>,
    inference_router: State<'_, crate::inference::router::InferenceRouter>,
    query: String,
    chat_id: Option<String>,
    doc_ids: Option<Vec<String>>,
    project_id: Option<String>,
) -> Result<Vec<String>, String> {
    let embedding_backend = inference_router.embedding_backend().await;
    retrieve_context_internal(
        Some(app),
        sidecar.inner(),
        pool.inner().clone(),
        vector_manager.inner().clone(),
        reranker.inner(),
        embedding_backend,
        query,
        chat_id,
        doc_ids,
        project_id,
    )
    .await
}

pub async fn retrieve_context_internal(
    app: Option<tauri::AppHandle>,
    sidecar: &SidecarManager,
    pool: SqlitePool,
    vector_manager: crate::vector_store::VectorStoreManager,
    reranker: &crate::reranker::RerankerWrapper,
    embedding_backend: Option<std::sync::Arc<dyn crate::inference::embedding::EmbeddingBackend>>,
    query: String,
    chat_id: Option<String>,
    doc_ids: Option<Vec<String>>,
    project_id_arg: Option<String>,
) -> Result<Vec<String>, String> {
    #[derive(serde::Serialize, Clone)]
    struct WebSearchStatus {
        id: Option<String>,
        step: String,
        message: String,
    }

    if let Some(h) = &app {
        use tauri::Emitter;
        let _ = h.emit(
            "web_search_status",
            WebSearchStatus {
                id: chat_id.clone(),
                step: "rag_searching".into(),
                message: format!("Searching knowledge base for: {}", query),
            },
        );
    }

    let rrf_k = 60.0;
    let query_lower = query.to_lowercase();

    let mut project_id: Option<String> = project_id_arg;
    if project_id.is_none() {
        if let Some(cid) = &chat_id {
            project_id = sqlx::query_scalar("SELECT project_id FROM conversations WHERE id = ?")
                .bind(cid)
                .fetch_optional(&pool)
                .await
                .map_err(|e| format!("Failed to fetch project_id: {}", e))?
                .flatten();
        }
    }

    let initial_top_k = 150;

    let is_overview = query_lower.contains("list file")
        || query_lower.contains("what file")
        || query_lower.contains("available document")
        || query_lower.contains("structure of project")
        || query_lower.contains("what documents");

    if let Some(pid) = &project_id {
        if is_overview {
            let paths: Vec<String> = sqlx::query_scalar(
                "SELECT path FROM documents WHERE project_id = ? ORDER BY path ASC",
            )
            .bind(pid)
            .fetch_all(&pool)
            .await
            .unwrap_or(Vec::new());

            if !paths.is_empty() {
                let list = paths.join("\n- ");
                return Ok(vec![format!("**Available Project Files:**\n- {}", list)]);
            }
        }
    }

    if let Some(pid) = &project_id {
        let docs: Vec<(String, String)> =
            sqlx::query_as("SELECT id, path FROM documents WHERE project_id = ?")
                .bind(pid)
                .fetch_all(&pool)
                .await
                .unwrap_or(Vec::new());

        let mut matched_doc_id = None;
        let mut matched_path = "";

        for (id, path) in &docs {
            let filename = std::path::Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();

            if query_lower.contains(&filename) && filename.len() > 3 {
                matched_doc_id = Some(id);
                matched_path = path;
                break;
            }
        }

        if let Some(doc_id) = matched_doc_id {
            let chunks: Vec<String> = sqlx::query_scalar(
                "SELECT content FROM chunks WHERE document_id = ? ORDER BY chunk_index ASC",
            )
            .bind(doc_id)
            .fetch_all(&pool)
            .await
            .unwrap_or(Vec::new());

            let full_text = chunks.join("");
            let safe_text = if full_text.len() > 15000 {
                let mut end = 15000;
                while !full_text.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}... (truncated)", &full_text[..end])
            } else {
                full_text
            };

            return Ok(vec![format!(
                "**Reading File: {}**\n\n{}",
                matched_path, safe_text
            )]);
        }
    }

    let is_vague = query_lower.len() < 25
        || query_lower.contains("summariz")
        || query_lower.contains("what is this")
        || query_lower.contains("tell me about")
        || query_lower.contains("explain this");

    let mut global_chunks: Vec<String> = Vec::new();
    if let Some(docs) = &doc_ids {
        for doc_id in docs {
            let intro_rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT c.content, d.path FROM chunks c JOIN documents d ON c.document_id = d.id WHERE d.id = ? ORDER BY c.chunk_index ASC LIMIT 3"
            )
            .bind(doc_id)
            .fetch_all(&pool)
            .await
            .unwrap_or_default();

            let total_chunks: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM chunks WHERE document_id = ?")
                    .bind(doc_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0);

            if total_chunks <= 10 && total_chunks > 3 {
                let all_rows: Vec<(String, String)> = sqlx::query_as(
                    "SELECT c.content, d.path FROM chunks c JOIN documents d ON c.document_id = d.id WHERE d.id = ? ORDER BY c.chunk_index ASC"
                )
                .bind(doc_id)
                .fetch_all(&pool)
                .await
                .unwrap_or_default();

                for (content, path) in all_rows {
                    global_chunks.push(format!("[Full Document: {}]\n{}", path, content));
                }
            } else {
                for (content, path) in intro_rows {
                    global_chunks.push(format!("[Intro: {}]\n{}", path, content));
                }
            }
        }
    }

    let mut vector_results: Vec<i64> = Vec::new();

    // Determine the vector search scopes
    let scope = crate::vector_store::VectorStoreManager::scope_for(&project_id, &chat_id);
    let mut search_scopes = vec![scope.clone()];
    // Always include Global scope as well (so global docs are available everywhere)
    if !matches!(scope, crate::vector_store::VectorScope::Global) {
        search_scopes.push(crate::vector_store::VectorScope::Global);
    }

    // Try embedding via InferenceRouter first (supports cloud + local backends),
    // fall back to direct sidecar HTTP call if no embedding backend is active.
    if let Some(backend) = &embedding_backend {
        match backend.embed(query.clone()).await {
            Ok(embedding) => {
                if let Ok(keys) =
                    vector_manager.search_scoped(&embedding, &search_scopes, initial_top_k)
                {
                    vector_results = keys.into_iter().map(|k| k as i64).collect();
                }
            }
            Err(e) => {
                tracing::warn!(
                    "[rag] InferenceRouter embedding failed, trying sidecar: {}",
                    e
                );
                // Fall through to sidecar below
            }
        }
    }

    // Sidecar fallback (or primary if no embedding backend configured)
    if vector_results.is_empty() {
        if let Some((port, token)) = sidecar.get_embedding_config() {
            let client = reqwest::Client::new();
            let url = format!("http://127.0.0.1:{}/v1/embeddings", port);

            if let Ok(response) = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", token))
                .json(&serde_json::json!({ "input": query, "model": "default" }))
                .send()
                .await
            {
                if let Ok(res_json) = response.json::<EmbeddingResponse>().await {
                    if let Some(data) = res_json.data.first() {
                        if let Ok(keys) = vector_manager.search_scoped(
                            &data.embedding,
                            &search_scopes,
                            initial_top_k,
                        ) {
                            vector_results = keys.into_iter().map(|k| k as i64).collect();
                        }
                    }
                }
            }
        }
    }

    let fts_query = format!("\"{}\"", query.replace("\"", ""));
    let fts_results: Vec<i64> = {
        // Scope-filter FTS results to match the vector search scopes:
        // project documents + global, or chat documents + global.
        if let Some(ref pid) = project_id {
            sqlx::query(
                r#"SELECT f.rowid FROM chunks_fts f
                   JOIN chunks c ON c.rowid = f.rowid
                   JOIN documents d ON c.document_id = d.id
                   WHERE f.content MATCH ?
                   AND (d.project_id = ? OR (d.project_id IS NULL AND d.chat_id IS NULL))
                   ORDER BY f.rank LIMIT ?"#,
            )
            .bind(&fts_query)
            .bind(pid)
            .bind(initial_top_k as i64)
            .fetch_all(&pool)
            .await
            .map(|rows| rows.into_iter().map(|r| r.get::<i64, _>("rowid")).collect())
            .unwrap_or(Vec::new())
        } else if let Some(ref cid) = chat_id {
            sqlx::query(
                r#"SELECT f.rowid FROM chunks_fts f
                   JOIN chunks c ON c.rowid = f.rowid
                   JOIN documents d ON c.document_id = d.id
                   WHERE f.content MATCH ?
                   AND (d.chat_id = ? OR (d.project_id IS NULL AND d.chat_id IS NULL))
                   ORDER BY f.rank LIMIT ?"#,
            )
            .bind(&fts_query)
            .bind(cid)
            .bind(initial_top_k as i64)
            .fetch_all(&pool)
            .await
            .map(|rows| rows.into_iter().map(|r| r.get::<i64, _>("rowid")).collect())
            .unwrap_or(Vec::new())
        } else {
            // Global scope — no filter needed
            sqlx::query("SELECT rowid FROM chunks_fts WHERE content MATCH ? ORDER BY rank LIMIT ?")
                .bind(&fts_query)
                .bind(initial_top_k as i64)
                .fetch_all(&pool)
                .await
                .map(|rows| rows.into_iter().map(|r| r.get::<i64, _>("rowid")).collect())
                .unwrap_or(Vec::new())
        }
    };

    let mut fused_scores: HashMap<i64, f32> = HashMap::new();

    for (rank, id) in vector_results.iter().enumerate() {
        let score = 1.0 / (rrf_k + (rank as f32) + 1.0);
        *fused_scores.entry(*id).or_insert(0.0) += score;
    }

    for (rank, id) in fts_results.iter().enumerate() {
        let score = 1.0 / (rrf_k + (rank as f32) + 1.0);
        *fused_scores.entry(*id).or_insert(0.0) += score;
    }

    // No project-boost needed — scoped indices handle this natively now.

    let mut final_ranking: Vec<(i64, f32)> = fused_scores.into_iter().collect();
    final_ranking.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let search_top_k = if doc_ids.is_some() {
        300
    } else {
        initial_top_k
    };

    let candidate_ids: Vec<i64> = final_ranking
        .into_iter()
        .take(search_top_k)
        .map(|(id, _)| id)
        .collect();

    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: String = candidate_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");

    let mut final_sql = format!(
        r#"
        SELECT c.content, d.path
        FROM chunks c
        JOIN documents d ON c.document_id = d.id
        WHERE c.rowid IN ({})
        "#,
        placeholders
    );

    // Vector results are already scope-filtered (scoped index + global index).
    // We only need the doc_ids filter for explicitly attached documents.
    if let Some(docs) = &doc_ids {
        if !docs.is_empty() {
            let doc_placeholders = docs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            final_sql.push_str(&format!(" AND d.id IN ({})", doc_placeholders));
        }
    }

    let mut query_obj = sqlx::query(&final_sql);

    for id in &candidate_ids {
        query_obj = query_obj.bind(id);
    }

    if let Some(docs) = &doc_ids {
        if !docs.is_empty() {
            for doc_id in docs {
                query_obj = query_obj.bind(doc_id);
            }
        }
    }

    let content_rows = query_obj
        .fetch_all(&pool)
        .await
        .map_err(|e| format!("DB Fetch Error: {}", e))?;

    if let Some(h) = app {
        use std::collections::HashSet;
        let mut unique_paths = HashSet::new();
        for row in &content_rows {
            let path: String = row.get("path");
            unique_paths.insert(path);
        }

        let results: Vec<serde_json::Value> = unique_paths
            .into_iter()
            .map(|p| {
                let filename = std::path::Path::new(&p)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                json!({
                    "title": filename,
                    "link": p,
                    "snippet": format!("Found relevant excerpts in {}", filename)
                })
            })
            .collect();

        let _ = h.emit(
            "web_search_results",
            json!({
                "id": chat_id.clone(),
                "results": results
            }),
        );

        let _ = h.emit(
            "web_search_status",
            WebSearchStatus {
                id: chat_id.clone(),
                step: "done".into(),
                message: format!("Found {} relevant documents.", content_rows.len()),
            },
        );
    }

    let candidates: Vec<(String, String)> = content_rows
        .iter()
        .map(|r| (r.get::<String, _>("content"), r.get::<String, _>("path")))
        .collect();

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let candidate_texts: Vec<String> = candidates.iter().map(|(c, _)| c.clone()).collect();
    let reranked_indices = reranker
        .rerank(&query, &candidate_texts)
        .map_err(|e| format!("Reranking failed: {}", e))?;

    let is_explicit = doc_ids.is_some() && !doc_ids.as_ref().unwrap().is_empty();
    let threshold = if is_explicit { -10.0 } else { -5.0 };

    let passed_results: Vec<(usize, f32)> = reranked_indices
        .into_iter()
        .filter(|(_, score)| *score > threshold)
        .collect();

    let mut top_results: Vec<String> = passed_results
        .into_iter()
        .take(5)
        .map(|(idx, _)| {
            let (content, path) = &candidates[idx];
            format!("[Source: {}]\n{}", path, content)
        })
        .collect();

    if is_vague && !global_chunks.is_empty() {
        for g_chunk in global_chunks.into_iter().rev().take(3) {
            let prefix_end = g_chunk
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i < 50)
                .last()
                .unwrap_or(0);
            if prefix_end == 0
                || !top_results
                    .iter()
                    .any(|r| r.contains(&g_chunk[..prefix_end]))
            {
                top_results.insert(0, g_chunk);
            }
        }
    } else if !global_chunks.is_empty() {
        let first_intro = &global_chunks[0];
        let prefix_end = first_intro
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i < 50)
            .last()
            .unwrap_or(0);
        if prefix_end == 0
            || !top_results
                .iter()
                .any(|r| r.contains(&first_intro[..prefix_end]))
        {
            top_results.insert(0, first_intro.clone());
        }
    }

    Ok(top_results)
}

pub async fn list_project_files(pool: &SqlitePool, project_id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT path FROM documents WHERE project_id = ? ORDER BY path ASC LIMIT 50")
        .bind(project_id)
        .fetch_all(pool)
        .await
        .unwrap_or(Vec::new())
}

pub async fn perform_integrity_check(
    pool: &SqlitePool,
    vector_manager: &crate::vector_store::VectorStoreManager,
) -> Result<String, String> {
    let chunk_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chunks")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("DB Count Error: {}", e))?;

    // With per-scope indices, total_count only reflects loaded indices.
    // On a fresh start with no queries yet, no indices are loaded, so we
    // skip the comparison and just report the DB count.
    let vector_count = vector_manager
        .total_count()
        .map_err(|e| format!("Vector Store Error: {}", e))? as i64;

    if vector_count > 0 && chunk_count != vector_count {
        return Ok(format!(
            "mismatch: db={}, vector={}",
            chunk_count, vector_count
        ));
    }

    Ok("ok".to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_rag_check_vector_index_integrity(
    pool: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
) -> Result<String, String> {
    perform_integrity_check(&pool, &vector_manager).await
}
