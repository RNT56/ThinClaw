use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokenizers::{Tokenizer, TruncationDirection, TruncationParams, TruncationStrategy};

use std::sync::Arc;

const RERANKER_REVISION: &str = "a09144355adeed5f58c8ed011d209bf8ee5a1fec";
const RERANKER_MODEL_SIZE: u64 = 23_143_499;
const RERANKER_MODEL_SHA256: &str =
    "e9d8ebf845c413e981c175bfe49a3bfa9b3dcce2a3ba54875ee5df5a58639fbe";
const RERANKER_TOKENIZER_SIZE: u64 = 711_396;
const RERANKER_TOKENIZER_SHA256: &str =
    "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66";
const MAX_RERANK_DOCUMENTS: usize = 150;
const MAX_RERANK_TOKENS: usize = 512;
const DOWNLOAD_ATTEMPTS: usize = 3;

/// Wrapper that gracefully handles reranker initialization failures.
/// If the reranker fails to load (e.g., download failure, ORT issues),
/// this wrapper will skip reranking instead of crashing.
#[derive(Clone)]
pub struct RerankerWrapper {
    inner: Arc<Option<Reranker>>,
}

impl RerankerWrapper {
    pub async fn new(app_data_dir: PathBuf) -> Self {
        match Reranker::new(app_data_dir).await {
            Ok(r) => {
                println!("[reranker] Successfully initialized.");
                Self {
                    inner: Arc::new(Some(r)),
                }
            }
            Err(e) => {
                eprintln!(
                    "[reranker] Failed to initialize: {}. RAG will skip reranking.",
                    e
                );
                Self {
                    inner: Arc::new(None),
                }
            }
        }
    }

    pub fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<(usize, f32)>> {
        match self.inner.as_ref() {
            Some(r) => match r.rerank(
                query,
                &documents[..documents.len().min(MAX_RERANK_DOCUMENTS)],
            ) {
                Ok(mut results) => {
                    // Keep the initial fused order for overflow candidates,
                    // below every scored candidate. This bounds ONNX memory
                    // without discarding explicit-document results.
                    let overflow_score =
                        results.last().map(|(_, score)| score - 1.0).unwrap_or(0.0);
                    results.extend(
                        (MAX_RERANK_DOCUMENTS..documents.len())
                            .map(|index| (index, overflow_score)),
                    );
                    Ok(results)
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "reranker inference failed; preserving fused retrieval order"
                    );
                    Ok(fallback_results(documents.len()))
                }
            },
            None => Ok(fallback_results(documents.len())),
        }
    }

    pub fn is_available(&self) -> bool {
        self.inner.is_some()
    }
}

pub struct Reranker {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl Reranker {
    pub async fn new(app_data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&app_data_dir)
            .with_context(|| format!("Failed to create {}", app_data_dir.display()))?;
        let model_path = app_data_dir.join("reranker_model.onnx");
        let tokenizer_path = app_data_dir.join("reranker_tokenizer.json");

        Self::ensure_files(&model_path, &tokenizer_path).await?;

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                direction: TruncationDirection::Right,
                max_length: MAX_RERANK_TOKENS,
                strategy: TruncationStrategy::LongestFirst,
                stride: 0,
            }))
            .map_err(|e| anyhow!("Failed to configure reranker truncation: {e}"))?;

        let builder = Session::builder().map_err(|e| anyhow!(e.to_string()))?;
        let builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow!(e.to_string()))?;
        let mut builder = builder
            .with_intra_threads(1)
            .map_err(|e| anyhow!(e.to_string()))?;
        let session = builder
            .commit_from_file(model_path)
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
        })
    }

    async fn ensure_files(model_path: &Path, tokenizer_path: &Path) -> Result<()> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .user_agent("ThinClawDesktop/0.14")
            .build()?;

        let base = format!(
            "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/{RERANKER_REVISION}"
        );
        ensure_artifact(
            &client,
            &format!("{base}/onnx/model_quantized.onnx"),
            model_path,
            RERANKER_MODEL_SIZE,
            RERANKER_MODEL_SHA256,
        )
        .await?;
        ensure_artifact(
            &client,
            &format!("{base}/tokenizer.json"),
            tokenizer_path,
            RERANKER_TOKENIZER_SIZE,
            RERANKER_TOKENIZER_SHA256,
        )
        .await?;
        Ok(())
    }

    pub fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<(usize, f32)>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        if documents.len() > MAX_RERANK_DOCUMENTS {
            return Err(anyhow!(
                "Reranker candidate count {} exceeds the bounded maximum {}",
                documents.len(),
                MAX_RERANK_DOCUMENTS
            ));
        }

        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());

        // Tokenize all query-document pairs
        // MS-MARCO MiniLM expects: [CLS] query [SEP] document [SEP]
        let encodings: Vec<_> = documents
            .iter()
            .map(|doc| self.tokenizer.encode((query, doc.as_str()), true))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| anyhow!("Batch encoding failed: {}", e))?;

        let batch_size = encodings.len();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0);

        // Build padded tensors for the full batch
        let mut input_ids = vec![0i64; batch_size * max_len];
        let mut attention_mask = vec![0i64; batch_size * max_len];
        let mut token_type_ids = vec![0i64; batch_size * max_len];

        for (i, enc) in encodings.iter().enumerate() {
            let offset = i * max_len;
            for (j, &id) in enc.get_ids().iter().enumerate() {
                input_ids[offset + j] = id as i64;
            }
            for (j, &mask) in enc.get_attention_mask().iter().enumerate() {
                attention_mask[offset + j] = mask as i64;
            }
            for (j, &tid) in enc.get_type_ids().iter().enumerate() {
                token_type_ids[offset + j] = tid as i64;
            }
        }

        // Shape: [BatchSize, MaxSeqLen]. Construct ORT-owned tensors directly
        // so the boundary is independent of the ndarray version used by ORT.
        let input_ids_tensor = Tensor::from_array(([batch_size, max_len], input_ids))?;
        let attention_mask_tensor = Tensor::from_array(([batch_size, max_len], attention_mask))?;
        let token_type_ids_tensor = Tensor::from_array(([batch_size, max_len], token_type_ids))?;

        // Single forward pass for the entire batch
        let outputs = session.run(ort::inputs![
            "input_ids" => input_ids_tensor,
            "attention_mask" => attention_mask_tensor,
            "token_type_ids" => token_type_ids_tensor,
        ])?;

        // Output shape: [BatchSize, 1] — one logit per candidate
        let (shape, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
        if logits.len() != batch_size {
            return Err(anyhow!(
                "Reranker returned {} logits with shape {:?} for a batch of {}",
                logits.len(),
                shape,
                batch_size
            ));
        }
        if logits.iter().any(|score| !score.is_finite()) {
            return Err(anyhow!("Reranker returned a non-finite relevance score"));
        }
        let mut results: Vec<(usize, f32)> = logits.iter().copied().enumerate().collect();

        // Sort descending (highest relevance first)
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(results)
    }
}

fn fallback_results(document_count: usize) -> Vec<(usize, f32)> {
    (0..document_count).map(|index| (index, 0.0)).collect()
}

fn verify_artifact(path: &Path, expected_size: u64, expected_sha256: &str) -> Result<bool> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() != expected_size {
        return Ok(false);
    }

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()) == expected_sha256)
}

async fn ensure_artifact(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<()> {
    if verify_artifact(destination, expected_size, expected_sha256)? {
        return Ok(());
    }

    let staging = destination.with_extension("part");
    let _ = tokio::fs::remove_file(&staging).await;
    let mut last_error = None;
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        match download_artifact(client, url, &staging, expected_size, expected_sha256).await {
            Ok(()) => {
                if destination.exists() {
                    tokio::fs::remove_file(destination).await?;
                }
                tokio::fs::rename(&staging, destination).await?;
                return Ok(());
            }
            Err(error) => {
                tracing::warn!(attempt, url, error = %error, "reranker artifact download failed");
                last_error = Some(error);
                let _ = tokio::fs::remove_file(&staging).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("Reranker artifact download failed")))
}

async fn download_artifact(
    client: &reqwest::Client,
    url: &str,
    staging: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let response = client.get(url).send().await?.error_for_status()?;
    if let Some(length) = response.content_length() {
        if length != expected_size {
            return Err(anyhow!(
                "Unexpected content length for {url}: {length}, expected {expected_size}"
            ));
        }
    }

    let mut file = tokio::fs::File::create(staging).await?;
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > expected_size {
            return Err(anyhow!("Reranker artifact exceeded its declared size"));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;

    if downloaded != expected_size {
        return Err(anyhow!(
            "Incomplete reranker artifact: received {downloaded}, expected {expected_size} bytes"
        ));
    }
    let actual_sha256 = hex::encode(hasher.finalize());
    if actual_sha256 != expected_sha256 {
        return Err(anyhow!(
            "Reranker artifact checksum mismatch: {actual_sha256}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_verification_rejects_wrong_size_and_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        std::fs::write(&path, b"verified").unwrap();
        assert!(verify_artifact(
            &path,
            8,
            "1c34f88707b55e6104c4eb20e71ffa3d33e414b71ef689a15fad0640d0ac58cb"
        )
        .unwrap());
        assert!(!verify_artifact(&path, 9, "unused").unwrap());
        assert!(!verify_artifact(&path, 8, "wrong").unwrap());
    }

    #[test]
    fn fallback_preserves_retrieval_order() {
        assert_eq!(fallback_results(3), vec![(0, 0.0), (1, 0.0), (2, 0.0)]);
    }
}
