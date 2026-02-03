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
        let mut results = Vec::new();
        let session = self.session.lock().unwrap();

        // Batch processing (or single loop for simplicity first)
        // MS-MARCO MiniLM expects: [CLS] query [SEP] document [SEP]
        for (index, doc) in documents.iter().enumerate() {
            // Encode: sequence_pair(query, doc)
            let encoding = self
                .tokenizer
                .encode((query, doc.as_str()), true)
                .map_err(|e| anyhow!("Encoding failed: {}", e))?;

            let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
            let attention_mask: Vec<i64> = encoding
                .get_attention_mask()
                .iter()
                .map(|&x| x as i64)
                .collect();
            let token_type_ids: Vec<i64> =
                encoding.get_type_ids().iter().map(|&x| x as i64).collect();

            let seq_len = input_ids.len();

            // Create tensors
            // Shape: [Batch(1), SeqLen]
            let input_ids_array = Array2::from_shape_vec((1, seq_len), input_ids)?;
            let attention_mask_array = Array2::from_shape_vec((1, seq_len), attention_mask)?;
            let token_type_ids_array = Array2::from_shape_vec((1, seq_len), token_type_ids)?;

            let outputs = session.run(ort::inputs![
                "input_ids" => input_ids_array,
                "attention_mask" => attention_mask_array,
                "token_type_ids" => token_type_ids_array,
            ]?)?;

            // Output is usually [Batch, 1] logits
            let logits = outputs["logits"].try_extract_tensor::<f32>()?;
            let score = logits[[0, 0]]; // First element

            // Sigmoid to get 0..1 probability if needed, or just use raw logit for sorting
            // Model was trained with BCE, so sigmoid is appropriate for probability,
            // but for ranking, raw logit is fine.
            results.push((index, score));
        }

        // Sort descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(results)
    }
}
