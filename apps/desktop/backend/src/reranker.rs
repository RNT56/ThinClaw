use anyhow::{anyhow, bail, Context, Result};
use ndarray::Array2;
use ort::session::{builder::GraphOptimizationLevel, Session};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokenizers::{Tokenizer, TruncationParams};
use uuid::Uuid;

const RERANKER_REVISION: &str = "a09144355adeed5f58c8ed011d209bf8ee5a1fec";
const MODEL_FILENAME: &str = "reranker_model.onnx";
const MODEL_REMOTE_PATH: &str = "onnx/model_quantized.onnx";
const MODEL_SIZE: u64 = 23_143_499;
const MODEL_SHA256: &str = "e9d8ebf845c413e981c175bfe49a3bfa9b3dcce2a3ba54875ee5df5a58639fbe";
const TOKENIZER_FILENAME: &str = "reranker_tokenizer.json";
const TOKENIZER_REMOTE_PATH: &str = "tokenizer.json";
const TOKENIZER_SIZE: u64 = 711_396;
const TOKENIZER_SHA256: &str = "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66";
const MAX_SEQUENCE_TOKENS: usize = 512;
const MAX_RERANK_DOCUMENTS: usize = 300;
const MAX_RERANK_QUERY_BYTES: usize = 8 * 1024;
const MAX_RERANK_DOCUMENT_BYTES: usize = 64 * 1024;
const MAX_RERANK_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOWNLOAD_ERROR_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy)]
struct ArtifactSpec {
    filename: &'static str,
    remote_path: &'static str,
    size: u64,
    sha256: &'static str,
}

const MODEL_ARTIFACT: ArtifactSpec = ArtifactSpec {
    filename: MODEL_FILENAME,
    remote_path: MODEL_REMOTE_PATH,
    size: MODEL_SIZE,
    sha256: MODEL_SHA256,
};
const TOKENIZER_ARTIFACT: ArtifactSpec = ArtifactSpec {
    filename: TOKENIZER_FILENAME,
    remote_path: TOKENIZER_REMOTE_PATH,
    size: TOKENIZER_SIZE,
    sha256: TOKENIZER_SHA256,
};

/// Wrapper that initializes the optional reranker in the background. RAG can
/// immediately use its fused vector/full-text ranking while the verified model
/// cache is prepared, so a first-run network delay never blocks app startup.
#[derive(Clone)]
pub struct RerankerWrapper {
    inner: Arc<RwLock<Option<Reranker>>>,
}

impl RerankerWrapper {
    pub async fn new(app_data_dir: PathBuf) -> Self {
        let inner = Arc::new(RwLock::new(None));
        let destination = inner.clone();
        tokio::spawn(async move {
            match Reranker::new(app_data_dir).await {
                Ok(reranker) => {
                    *destination.write().unwrap_or_else(|lock| lock.into_inner()) = Some(reranker);
                    tracing::info!("[reranker] Verified reranker initialized");
                }
                Err(error) => {
                    tracing::warn!(
                        "[reranker] Initialization failed; fused RAG ranking remains active: {error:#}"
                    );
                }
            }
        });
        Self { inner }
    }

    pub fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<(usize, f32)>> {
        let guard = self.inner.read().unwrap_or_else(|lock| lock.into_inner());
        match guard.as_ref() {
            Some(reranker) => reranker.rerank(query, documents),
            None => Ok(documents
                .iter()
                .enumerate()
                .map(|(index, _)| (index, 0.0))
                .collect()),
        }
    }

    pub fn is_available(&self) -> bool {
        self.inner
            .read()
            .unwrap_or_else(|lock| lock.into_inner())
            .is_some()
    }
}

pub struct Reranker {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl Reranker {
    pub async fn new(app_data_dir: PathBuf) -> Result<Self> {
        ensure_real_directory(&app_data_dir)?;
        let model_path = app_data_dir.join(MODEL_FILENAME);
        let tokenizer_path = app_data_dir.join(TOKENIZER_FILENAME);

        let verification_model = model_path.clone();
        let verification_tokenizer = tokenizer_path.clone();
        let cache_valid = tokio::task::spawn_blocking(move || {
            Ok::<_, anyhow::Error>(
                verify_artifact(&verification_model, MODEL_ARTIFACT)?
                    && verify_artifact(&verification_tokenizer, TOKENIZER_ARTIFACT)?,
            )
        })
        .await
        .context("reranker cache verification task failed")??;
        if !cache_valid {
            tracing::info!(
                revision = RERANKER_REVISION,
                "[reranker] Downloading pinned model artifacts"
            );
            Self::download_files(&app_data_dir, &model_path, &tokenizer_path).await?;
        }

        tokio::task::spawn_blocking(move || {
            if !verify_artifact(&model_path, MODEL_ARTIFACT)?
                || !verify_artifact(&tokenizer_path, TOKENIZER_ARTIFACT)?
            {
                bail!("reranker artifacts failed verification immediately before loading");
            }
            let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
                .map_err(|error| anyhow!("failed to load reranker tokenizer: {error}"))?;
            tokenizer
                .with_truncation(Some(TruncationParams {
                    max_length: MAX_SEQUENCE_TOKENS,
                    ..TruncationParams::default()
                }))
                .map_err(|error| anyhow!("failed to configure tokenizer truncation: {error}"))?;

            let session = Session::builder()
                .context("failed to construct ONNX session builder")?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .context("failed to configure ONNX graph optimization")?
                .with_intra_threads(1)
                .context("failed to configure ONNX worker count")?
                .commit_from_file(&model_path)
                .context("failed to load verified reranker model")?;
            Ok(Self {
                session: Mutex::new(session),
                tokenizer,
            })
        })
        .await
        .context("reranker model-loading task failed")?
    }

    async fn download_files(
        cache_dir: &Path,
        model_path: &Path,
        tokenizer_path: &Path,
    ) -> Result<()> {
        let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            let host_allowed = url
                .host_str()
                .is_some_and(|host| host == "huggingface.co" || host.ends_with(".hf.co"));
            if attempt.previous().len() >= 5 {
                attempt.error("too many Hugging Face redirects")
            } else if url.scheme() == "https"
                && host_allowed
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none()
            {
                attempt.follow()
            } else {
                attempt.error("Hugging Face redirected outside its HTTPS download hosts")
            }
        });
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(redirect_policy)
            .build()
            .context("failed to build reranker download client")?;

        let model_staging = download_artifact(&client, cache_dir, MODEL_ARTIFACT).await?;
        let tokenizer_staging =
            match download_artifact(&client, cache_dir, TOKENIZER_ARTIFACT).await {
                Ok(path) => path,
                Err(error) => {
                    let _ = tokio::fs::remove_file(&model_staging).await;
                    return Err(error);
                }
            };

        let model_staging_for_publish = model_staging.clone();
        let tokenizer_staging_for_publish = tokenizer_staging.clone();
        let model_target = model_path.to_path_buf();
        let tokenizer_target = tokenizer_path.to_path_buf();
        let cache_dir = cache_dir.to_path_buf();
        let publish = tokio::task::spawn_blocking(move || {
            publish_staged(&model_staging_for_publish, &model_target)?;
            publish_staged(&tokenizer_staging_for_publish, &tokenizer_target)?;
            #[cfg(unix)]
            std::fs::File::open(&cache_dir)
                .and_then(|directory| directory.sync_all())
                .context("failed to sync reranker cache directory")?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("reranker artifact publication task failed")?;
        if let Err(error) = publish {
            let _ = tokio::fs::remove_file(&model_staging).await;
            let _ = tokio::fs::remove_file(&tokenizer_staging).await;
            return Err(error);
        }
        Ok(())
    }

    pub fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<(usize, f32)>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        validate_rerank_input(query, documents)?;
        let session = self.session.lock().unwrap_or_else(|lock| lock.into_inner());

        let encodings: Vec<_> = documents
            .iter()
            .map(|document| self.tokenizer.encode((query, document.as_str()), true))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| anyhow!("reranker batch encoding failed: {error}"))?;
        let batch_size = encodings.len();
        let max_len = encodings
            .iter()
            .map(|encoding| encoding.get_ids().len())
            .max()
            .unwrap_or(0);
        if max_len == 0 || max_len > MAX_SEQUENCE_TOKENS {
            bail!("reranker tokenizer produced an invalid sequence length");
        }
        let tensor_len = batch_size
            .checked_mul(max_len)
            .ok_or_else(|| anyhow!("reranker tensor size overflow"))?;

        let mut input_ids = vec![0_i64; tensor_len];
        let mut attention_mask = vec![0_i64; tensor_len];
        let mut token_type_ids = vec![0_i64; tensor_len];
        for (batch_index, encoding) in encodings.iter().enumerate() {
            let offset = batch_index
                .checked_mul(max_len)
                .ok_or_else(|| anyhow!("reranker tensor offset overflow"))?;
            for (token_index, &id) in encoding.get_ids().iter().enumerate() {
                input_ids[offset + token_index] = i64::from(id);
            }
            for (token_index, &mask) in encoding.get_attention_mask().iter().enumerate() {
                attention_mask[offset + token_index] = i64::from(mask);
            }
            for (token_index, &type_id) in encoding.get_type_ids().iter().enumerate() {
                token_type_ids[offset + token_index] = i64::from(type_id);
            }
        }

        let input_ids = Array2::from_shape_vec((batch_size, max_len), input_ids)
            .context("failed to shape reranker input IDs")?;
        let attention_mask = Array2::from_shape_vec((batch_size, max_len), attention_mask)
            .context("failed to shape reranker attention mask")?;
        let token_type_ids = Array2::from_shape_vec((batch_size, max_len), token_type_ids)
            .context("failed to shape reranker token-type IDs")?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => input_ids,
                "attention_mask" => attention_mask,
                "token_type_ids" => token_type_ids,
            ]?)
            .context("reranker inference failed")?;

        let logits = outputs
            .get("logits")
            .ok_or_else(|| anyhow!("reranker model returned no logits output"))?;
        let (shape, values) = logits
            .try_extract_raw_tensor::<f32>()
            .context("reranker logits are not a float tensor")?;
        if shape.len() != 2 || shape[0] != batch_size as i64 || shape[1] < 1 {
            bail!("reranker returned an unexpected logits shape");
        }
        let columns = usize::try_from(shape[1]).context("reranker logits width is invalid")?;
        let mut results = Vec::with_capacity(batch_size);
        for index in 0..batch_size {
            let score = *values
                .get(index * columns)
                .ok_or_else(|| anyhow!("reranker logits output is truncated"))?;
            if !score.is_finite() {
                bail!("reranker returned a non-finite score");
            }
            results.push((index, score));
        }
        results.sort_by(|left, right| right.1.total_cmp(&left.1));
        Ok(results)
    }
}

fn ensure_real_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect reranker cache at {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("reranker cache root is not a real directory");
    }
    Ok(())
}

fn verify_artifact(path: &Path, spec: ArtifactSpec) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("failed to inspect reranker artifact"),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != spec.size {
        return Ok(false);
    }
    let mut file = std::fs::File::open(path).context("failed to open reranker artifact")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .context("failed to hash reranker artifact")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let valid = hex::encode(hasher.finalize()) == spec.sha256;
    if valid {
        #[cfg(unix)]
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .context("failed to secure reranker artifact")?;
    }
    Ok(valid)
}

fn artifact_url(spec: ArtifactSpec) -> String {
    format!(
        "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/{RERANKER_REVISION}/{}",
        spec.remote_path
    )
}

async fn download_artifact(
    client: &reqwest::Client,
    cache_dir: &Path,
    spec: ArtifactSpec,
) -> Result<PathBuf> {
    let staging = cache_dir.join(format!(".{}.{}.staging", spec.filename, Uuid::new_v4()));
    let result =
        async {
            let response = client
                .get(artifact_url(spec))
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(|error| {
                    anyhow!("reranker artifact request failed: {}", error.without_url())
                })?;
            if !response.status().is_success() {
                let status = response.status();
                let detail =
                    thinclaw_core::http_response::bounded_text(response, MAX_DOWNLOAD_ERROR_BYTES)
                        .await
                        .ok()
                        .map(|text| {
                            text.chars()
                                .filter(|character| !character.is_control())
                                .take(512)
                                .collect::<String>()
                        })
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| "no bounded error detail".to_string());
                bail!("reranker artifact download failed with HTTP {status}: {detail}");
            }
            if response
                .content_length()
                .is_some_and(|length| length != spec.size)
            {
                bail!("reranker artifact declared an unexpected size");
            }
            let mut response = response;
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)
                .await
                .context("failed to create reranker staging file")?;
            #[cfg(unix)]
            file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))
                .await
                .context("failed to secure reranker staging file")?;
            let mut hasher = Sha256::new();
            let mut downloaded = 0_u64;
            use tokio::io::AsyncWriteExt;
            while let Some(chunk) = response.chunk().await.map_err(|error| {
                anyhow!("reranker artifact stream failed: {}", error.without_url())
            })? {
                downloaded = downloaded
                    .checked_add(chunk.len() as u64)
                    .ok_or_else(|| anyhow!("reranker artifact size overflow"))?;
                if downloaded > spec.size {
                    bail!("reranker artifact exceeded its pinned size");
                }
                hasher.update(&chunk);
                file.write_all(&chunk)
                    .await
                    .context("failed to write reranker staging file")?;
            }
            if downloaded != spec.size || hex::encode(hasher.finalize()) != spec.sha256 {
                bail!("reranker artifact failed pinned size or SHA-256 verification");
            }
            file.sync_all()
                .await
                .context("failed to sync reranker staging file")?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&staging).await;
        return Err(error);
    }
    Ok(staging)
}

fn publish_staged(staging: &Path, target: &Path) -> Result<()> {
    #[cfg(windows)]
    if target.exists() {
        std::fs::remove_file(target).context("failed to replace stale reranker artifact")?;
    }
    std::fs::rename(staging, target).context("failed to publish reranker artifact")?;
    Ok(())
}

fn validate_rerank_input(query: &str, documents: &[String]) -> Result<()> {
    if query.trim().is_empty() || query.len() > MAX_RERANK_QUERY_BYTES || query.contains('\0') {
        bail!("reranker query is empty or exceeds its input limit");
    }
    if documents.len() > MAX_RERANK_DOCUMENTS {
        bail!("reranker document batch exceeds its count limit");
    }
    let mut total = query.len();
    for document in documents {
        if document.len() > MAX_RERANK_DOCUMENT_BYTES || document.contains('\0') {
            bail!("reranker document exceeds its input limit");
        }
        total = total
            .checked_add(document.len())
            .ok_or_else(|| anyhow!("reranker input size overflow"))?;
        if total > MAX_RERANK_TOTAL_BYTES {
            bail!("reranker batch exceeds its total input limit");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_urls_are_revision_pinned() {
        let url = artifact_url(MODEL_ARTIFACT);
        assert!(url.contains(RERANKER_REVISION));
        assert!(!url.contains("/resolve/main/"));
    }

    #[test]
    fn verifies_size_hash_and_rejects_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        std::fs::write(&path, b"abc").unwrap();
        let spec = ArtifactSpec {
            filename: "artifact",
            remote_path: "artifact",
            size: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        assert!(verify_artifact(&path, spec).unwrap());
        std::fs::write(&path, b"abd").unwrap();
        assert!(!verify_artifact(&path, spec).unwrap());
        #[cfg(unix)]
        {
            let target = directory.path().join("target");
            std::fs::write(&target, b"abc").unwrap();
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(!verify_artifact(&link, spec).unwrap());
        }
    }

    #[test]
    fn rerank_inputs_are_bounded() {
        assert!(validate_rerank_input("query", &["document".to_string()]).is_ok());
        assert!(validate_rerank_input("", &["document".to_string()]).is_err());
        assert!(validate_rerank_input(
            "query",
            &vec!["document".to_string(); MAX_RERANK_DOCUMENTS + 1]
        )
        .is_err());
        assert!(
            validate_rerank_input("query", &["x".repeat(MAX_RERANK_DOCUMENT_BYTES + 1)]).is_err()
        );
    }
}
