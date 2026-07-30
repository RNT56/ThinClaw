use super::*;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::handler::viewport::Viewport;
use futures::StreamExt;

/// Extract document content, using vision OCR when PDF text extraction is not usable.
///
/// Returns the bounded content and whether OCR contributed to the result.
pub async fn extract_document_content(
    app: &AppHandle,
    _sidecar: &SidecarManager,
    file_path: &str,
    buffer: &[u8],
    hash: &str,
    force_ocr_arg: bool,
) -> Result<(String, bool), String> {
    if !valid_sha256_hex(hash) {
        return Err("Document hash is invalid".to_string());
    }
    let mut force_ocr = force_ocr_arg;
    let path_lc = file_path.to_lowercase();
    let is_pdf = path_lc.ends_with(".pdf");
    if is_pdf && !buffer.starts_with(b"%PDF-") {
        return Err("Document does not contain a valid PDF signature".to_string());
    }

    // Every parser and renderer consumes the same immutable byte snapshot.
    // This avoids reopening a mutable source path after it was validated.
    let pdf_snapshot = if is_pdf {
        Some(snapshot_pdf_bytes(buffer).await?)
    } else {
        None
    };

    let raw_content = if is_pdf {
        let extraction_bytes = buffer.to_vec();
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::task::spawn_blocking(move || {
                pdf_extract::extract_text_from_mem(&extraction_bytes)
            }),
        )
        .await
        {
            Ok(Ok(Ok(text))) => text,
            _ => {
                force_ocr = true;
                String::new()
            }
        }
    } else {
        std::str::from_utf8(buffer)
            .map_err(|_| "Document is not valid UTF-8 text".to_string())?
            .to_string()
    };

    // Sanitize
    let content: String = truncate_utf8_owned(
        raw_content.chars().filter(|&c| c != '\0').collect(),
        MAX_EXTRACTED_TEXT_BYTES,
    );

    // Garbage detection
    let is_garbage = if is_pdf && !force_ocr {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            true
        } else {
            let total = trimmed.chars().count();
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
        let _handle = tokio::spawn(async move { while (handler.next().await).is_some() {} });

        let snapshot_path = pdf_snapshot
            .as_ref()
            .ok_or_else(|| "PDF snapshot is unavailable".to_string())?
            .path();
        let file_url = reqwest::Url::from_file_path(snapshot_path)
            .map_err(|_| "Failed to construct a safe PDF URL".to_string())?;
        let page = browser
            .new_page(file_url.as_str())
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
                let supported_kind = matches!(
                    provider_cfg.kind,
                    crate::rig_lib::unified_provider::ProviderKind::OpenAI
                        | crate::rig_lib::unified_provider::ProviderKind::Groq
                        | crate::rig_lib::unified_provider::ProviderKind::OpenRouter
                        | crate::rig_lib::unified_provider::ProviderKind::Local
                );
                let credential_valid = matches!(
                    provider_cfg.kind,
                    crate::rig_lib::unified_provider::ProviderKind::Local
                ) || (!provider_cfg.token.is_empty()
                    && provider_cfg.token.len() <= 4096
                    && !provider_cfg.token.chars().any(char::is_control));
                if supported_kind
                    && credential_valid
                    && !provider_cfg.model_name.is_empty()
                    && provider_cfg.model_name.len() <= 256
                    && !provider_cfg.model_name.chars().any(char::is_control)
                {
                    let url = format!(
                        "{}/chat/completions",
                        provider_cfg.base_url.trim_end_matches('/')
                    );
                    Some((url, provider_cfg.token, provider_cfg.model_name))
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some((url, token, model_name)) = ocr_endpoint {
            let parsed_url = reqwest::Url::parse(&url)
                .map_err(|_| "OCR endpoint URL is invalid".to_string())?;
            let host = parsed_url
                .host_str()
                .ok_or_else(|| "OCR endpoint has no host".to_string())?
                .to_string();
            let is_local = host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback());
            let mut client_builder = reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none());
            if is_local {
                if !matches!(parsed_url.scheme(), "http" | "https") {
                    return Err("Local OCR endpoint URL is invalid".to_string());
                }
            } else {
                let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
                    parsed_url.as_str(),
                    &thinclaw_tools_core::OutboundUrlGuardOptions {
                        require_https: true,
                        upgrade_http_to_https: false,
                        allowlist: vec![host.clone()],
                    },
                )
                .await
                .map_err(|_| "OCR endpoint is not a public HTTPS destination".to_string())?;
                if !guarded.pinned_addrs.is_empty() {
                    client_builder = client_builder.resolve_to_addrs(&host, &guarded.pinned_addrs);
                }
            }
            let client = client_builder
                .build()
                .map_err(|error| format!("Failed to build OCR client: {error}"))?;

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
                        if screenshot.len() > 5 * 1024 * 1024 {
                            break;
                        }
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
                            ],
                            "max_tokens": 4096,
                            "stream": false
                        });

                        let mut request = client.post(&url).json(&body);
                        if !token.is_empty() {
                            request = request.bearer_auth(&token);
                        }
                        if let Ok(resp) = request.send().await {
                            if resp.status().is_success() {
                                if let Ok(json) =
                                    thinclaw_core::http_response::bounded_json::<
                                        serde_json::Value,
                                    >(resp, 1024 * 1024)
                                    .await
                                {
                                    if let Some(transcription) =
                                        json["choices"][0]["message"]["content"].as_str()
                                    {
                                        if transcription.len() > 512 * 1024 {
                                            break;
                                        } else if transcription != "[empty]"
                                            && !transcription.trim().is_empty()
                                        {
                                            if ocr_text.len().saturating_add(transcription.len())
                                                > MAX_EXTRACTED_TEXT_BYTES
                                            {
                                                break;
                                            }
                                            ocr_text
                                                .push_str(&format!("--- Page {} ---\n", i));
                                            ocr_text.push_str(transcription);
                                            ocr_text.push_str("\n\n");
                                        } else if i > 1 && transcription.contains("[empty]") {
                                            break;
                                        }
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
        let preview_rel = format!("previews/{hash}.jpg");
        let file_store = app.state::<crate::file_store::FileStore>();
        if !file_store.exists(&preview_rel).await.unwrap_or(false) && !ocr_used {
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
            let preview_result: Result<(), String> = async {
                    let _handle =
                        tokio::spawn(async move { while (handler.next().await).is_some() {} });
                    let snapshot_path = pdf_snapshot
                        .as_ref()
                        .ok_or_else(|| "PDF snapshot is unavailable".to_string())?
                        .path();
                    let file_url = reqwest::Url::from_file_path(snapshot_path)
                        .map_err(|_| "Failed to construct a safe PDF URL".to_string())?;
                    let page = browser
                        .new_page(file_url.as_str())
                        .await
                        .map_err(|e| e.to_string())?;
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if let Ok(screenshot) = page.screenshot(chromiumoxide::page::ScreenshotParams::builder().format(chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Jpeg).quality(80).build()).await {
                        if screenshot.len() > 5 * 1024 * 1024 {
                            return Ok(());
                        }
                        let _ = file_store.write(&preview_rel, &screenshot).await;
                    }
                    Ok(())
                }
                .await;
            let _ = browser.close().await;
            preview_result?;
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

    Ok((
        truncate_utf8_owned(final_content, MAX_EXTRACTED_TEXT_BYTES),
        ocr_used,
    ))
}
