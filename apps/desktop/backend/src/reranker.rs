use anyhow::{anyhow, Result};
use ndarray::Array2;
use ort::session::{builder::GraphOptimizationLevel, Session};
use std::path::PathBuf;
use std::sync::Mutex;
use tokenizers::Tokenizer;

use std::sync::Arc;

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
            Some(r) => r.rerank(query, documents),
            None => {
                // Fallback: return documents in original order with zero scores
                Ok(documents
                    .iter()
                    .enumerate()
                    .map(|(i, _)| (i, 0.0))
                    .collect())
            }
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
        let model_path = app_data_dir.join("reranker_model.onnx");
        let tokenizer_path = app_data_dir.join("reranker_tokenizer.json");

        if !model_path.exists() || !tokenizer_path.exists() {
            println!("[reranker] Downloading model files...");
            Self::download_files(&model_path, &tokenizer_path).await?;
        }

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;

        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(1)?
            .commit_from_file(model_path)?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
        })
    }

    async fn download_files(model_path: &PathBuf, tokenizer_path: &PathBuf) -> Result<()> {
        let client = reqwest::Client::new();

        // Using Xenova's quantized ONNX models (usually reliable) or standard optimum.
        // Let's use a known working URL for ms-marco-MiniLM-L-6-v2 quantized.
        // HuggingFace -> Xenova/ms-marco-MiniLM-L-6-v2 -> onnx/model_quantized.onnx
        let model_url = "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/onnx/model_quantized.onnx";
        let tokenizer_url =
            "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/tokenizer.json";

        println!("[reranker] Downloading model from {}", model_url);
        let model_bytes = client.get(model_url).send().await?.bytes().await?;
        std::fs::write(model_path, model_bytes)
            .map_err(|e| anyhow!("Failed to write model: {}", e))?;

        println!("[reranker] Downloading tokenizer from {}", tokenizer_url);
        let tokenizer_bytes = client.get(tokenizer_url).send().await?.bytes().await?;
        std::fs::write(tokenizer_path, tokenizer_bytes)
            .map_err(|e| anyhow!("Failed to write tokenizer: {}", e))?;

        println!("[reranker] Download complete.");
        Ok(())
    }

    pub fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<(usize, f32)>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }

        let session = self.session.lock().unwrap_or_else(|e| e.into_inner());

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

        // Shape: [BatchSize, MaxSeqLen]
        let input_ids_array = Array2::from_shape_vec((batch_size, max_len), input_ids)?;
        let attention_mask_array = Array2::from_shape_vec((batch_size, max_len), attention_mask)?;
        let token_type_ids_array = Array2::from_shape_vec((batch_size, max_len), token_type_ids)?;

        // Single forward pass for the entire batch
        let outputs = session.run(ort::inputs![
            "input_ids" => input_ids_array,
            "attention_mask" => attention_mask_array,
            "token_type_ids" => token_type_ids_array,
        ]?)?;

        // Output shape: [BatchSize, 1] — one logit per candidate
        let logits = outputs["logits"].try_extract_tensor::<f32>()?;
        let mut results: Vec<(usize, f32)> = (0..batch_size).map(|i| (i, logits[[i, 0]])).collect();

        // Sort descending (highest relevance first)
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(results)
    }
}
