use serde::Serialize;
use specta::Type;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

#[derive(Serialize, Clone, Type, Debug, Default)]
pub struct GGUFMetadata {
    pub architecture: String,
    #[specta(type = f64)]
    pub context_length: u64,
    #[specta(type = f64)]
    pub embedding_length: u64,
    #[specta(type = f64)]
    pub block_count: u64,
    #[specta(type = f64)]
    pub head_count: u64,
    #[specta(type = f64)]
    pub head_count_kv: u64,
    pub file_type: u32,
    /// Raw chat template string from tokenizer.chat_template (Jinja2)
    pub chat_template: Option<String>,
    /// Detected model family based on architecture + template heuristics
    pub model_family: Option<String>,
}

pub fn read_gguf_metadata(path: &str) -> Result<GGUFMetadata, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;

    // Read Magic
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != b"GGUF" {
        return Err("Not a GGUF file".to_string());
    }

    // Version
    let mut version_bytes = [0u8; 4];
    file.read_exact(&mut version_bytes)
        .map_err(|e| e.to_string())?;
    let version = u32::from_le_bytes(version_bytes);
    if version != 2 && version != 3 {
        return Err(format!("Unsupported GGUF version: {}", version));
    }

    // Tensor Count
    let mut tensor_count_bytes = [0u8; 8];
    file.read_exact(&mut tensor_count_bytes)
        .map_err(|e| e.to_string())?;

    // Metadata KV Count
    let mut kv_count_bytes = [0u8; 8];
    file.read_exact(&mut kv_count_bytes)
        .map_err(|e| e.to_string())?;
    let kv_count = u64::from_le_bytes(kv_count_bytes);

    let mut metadata = GGUFMetadata::default();

    for _ in 0..kv_count {
        let key = read_gguf_string(&mut file)?;

        let mut type_bytes = [0u8; 4];
        file.read_exact(&mut type_bytes)
            .map_err(|e| e.to_string())?;
        let val_type = u32::from_le_bytes(type_bytes);

        match key.as_str() {
            "general.architecture" => {
                metadata.architecture = read_value_string(&mut file, val_type)?;
            }
            "tokenizer.chat_template" => {
                if val_type == 8 {
                    metadata.chat_template = Some(read_gguf_string(&mut file)?);
                } else {
                    skip_value(&mut file, val_type)?;
                }
            }
            _ if key.ends_with(".context_length") => {
                metadata.context_length = read_value_u64(&mut file, val_type)?;
            }
            _ if key.ends_with(".embedding_length") => {
                metadata.embedding_length = read_value_u64(&mut file, val_type)?;
            }
            _ if key.ends_with(".block_count") => {
                metadata.block_count = read_value_u64(&mut file, val_type)?;
            }
            _ if key.ends_with(".attention.head_count") => {
                metadata.head_count = read_value_u64(&mut file, val_type)?;
            }
            _ if key.ends_with(".attention.head_count_kv") => {
                metadata.head_count_kv = read_value_u64(&mut file, val_type)?;
            }
            "general.file_type" => {
                metadata.file_type = read_value_u32(&mut file, val_type)?;
            }
            _ => {
                skip_value(&mut file, val_type)?;
            }
        }
    }

    if metadata.head_count_kv == 0 {
        metadata.head_count_kv = metadata.head_count;
    }

    metadata.model_family = Some(detect_model_family(
        &metadata.architecture,
        metadata.chat_template.as_deref(),
    ));

    Ok(metadata)
}

/// Detect the model family from GGUF architecture string and/or chat template content.
/// Returns a normalized family string used for stop token selection.
pub fn detect_model_family(architecture: &str, chat_template: Option<&str>) -> String {
    let arch_lower = architecture.to_lowercase();

    // 1. Architecture-based detection (most reliable)
    if arch_lower.contains("llama") {
        return "llama3".into();
    }
    if arch_lower.contains("mistral") || arch_lower.contains("mixtral") {
        return "mistral".into();
    }
    if arch_lower.contains("deepseek") {
        return "deepseek".into();
    }
    if arch_lower.contains("chatglm") || arch_lower.contains("glm") {
        return "glm".into();
    }
    if arch_lower.contains("gemma") {
        return "gemma".into();
    }
    if arch_lower.contains("qwen") {
        return "qwen".into();
    }
    if arch_lower.contains("phi") {
        return "chatml".into();
    }
    if arch_lower.contains("starcoder") || arch_lower.contains("codellama") {
        return "llama3".into();
    }

    // 2. Template-based fallback detection
    if let Some(tpl) = chat_template {
        if tpl.contains("<|eot_id|>") || tpl.contains("<|start_header_id|>") {
            return "llama3".into();
        }
        if tpl.contains("[INST]") || tpl.contains("[/INST]") {
            return "mistral".into();
        }
        if tpl.contains("<start_of_turn>") || tpl.contains("<end_of_turn>") {
            return "gemma".into();
        }
        if tpl.contains("<|im_start|>") || tpl.contains("<|im_end|>") {
            return "qwen".into();
        }
        if tpl.contains("[gMASK]") || tpl.contains("sop") {
            return "glm".into();
        }
    }

    // 3. Unknown — default to ChatML which is the most common format
    "chatml".into()
}

/// Return the appropriate stop tokens for a given model family.
/// These are used both in llama-server --stop args and thinclaw model config.
pub fn stop_tokens_for_family(family: &str) -> Vec<String> {
    match family {
        "llama3" => vec![
            "<|eot_id|>".into(),
            "<|end_of_text|>".into(),
            "<|start_header_id|>user".into(),
            "<|start_header_id|>system".into(),
        ],
        "mistral" => vec!["[/INST]".into(), "</s>".into(), "[INST]".into()],
        "deepseek" => vec![
            "<|end_of_sentence|>".into(),
            "<|User|>".into(),
            "<|begin_of_sentence|>".into(),
        ],
        "glm" => vec!["[gMASK]".into(), "<sop>".into(), "<eop>".into()],
        "gemma" => vec![
            "<end_of_turn>".into(),
            "<start_of_turn>user".into(),
            "<start_of_turn>system".into(),
        ],
        "qwen" | "chatml" => vec![
            "<|im_end|>".into(),
            "<|im_start|>user".into(),
            "<|im_start|>system".into(),
            "<|endoftext|>".into(),
        ],
        _ => vec![
            "Human:".into(),
            "User:".into(),
            "### Human".into(),
            "### User".into(),
        ],
    }
}

fn read_gguf_string(file: &mut File) -> Result<String, String> {
    let mut len_bytes = [0u8; 8];
    file.read_exact(&mut len_bytes).map_err(|e| e.to_string())?;
    let len = u64::from_le_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).map_err(|e| e.to_string())?;
    String::from_utf8(buf).map_err(|e| e.to_string())
}

fn read_value_string(file: &mut File, val_type: u32) -> Result<String, String> {
    if val_type != 8 {
        return Err("Expected string".to_string());
    }
    read_gguf_string(file)
}

fn read_value_u64(file: &mut File, val_type: u32) -> Result<u64, String> {
    match val_type {
        4 => {
            let mut b = [0u8; 4];
            file.read_exact(&mut b).map_err(|e| e.to_string())?;
            Ok(u32::from_le_bytes(b) as u64)
        }
        10 => {
            let mut b = [0u8; 8];
            file.read_exact(&mut b).map_err(|e| e.to_string())?;
            Ok(u64::from_le_bytes(b))
        }
        _ => Err(format!("Expected UINT32/UINT64, got type {}", val_type)),
    }
}

fn read_value_u32(file: &mut File, val_type: u32) -> Result<u32, String> {
    if val_type != 4 {
        return Err("Expected UINT32".to_string());
    }
    let mut b = [0u8; 4];
    file.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(u32::from_le_bytes(b))
}

fn skip_value(file: &mut File, val_type: u32) -> Result<(), String> {
    match val_type {
        0..=7 => {
            // 0=UINT8(1) 1=INT8(1) 2=UINT16(2) 3=INT16(2) 4=UINT32(4) 5=INT32(4) 6=FLOAT32(4) 7=BOOL(1)
            let sizes: [i64; 8] = [1, 1, 2, 2, 4, 4, 4, 1];
            file.seek(SeekFrom::Current(sizes[val_type as usize]))
                .map_err(|e| e.to_string())?;
        }
        8 => {
            let mut len_bytes = [0u8; 8];
            file.read_exact(&mut len_bytes).map_err(|e| e.to_string())?;
            let len = u64::from_le_bytes(len_bytes);
            file.seek(SeekFrom::Current(len as i64))
                .map_err(|e| e.to_string())?;
        }
        9 => {
            let mut arr_type_bytes = [0u8; 4];
            file.read_exact(&mut arr_type_bytes)
                .map_err(|e| e.to_string())?;
            let arr_type = u32::from_le_bytes(arr_type_bytes);
            let mut len_bytes = [0u8; 8];
            file.read_exact(&mut len_bytes).map_err(|e| e.to_string())?;
            let len = u64::from_le_bytes(len_bytes);
            for _ in 0..len {
                skip_value(file, arr_type)?;
            }
        }
        // 10=UINT64 11=FLOAT64 12=INT64 13=INT64 — all 8 bytes
        10 | 11 | 12 | 13 => {
            file.seek(SeekFrom::Current(8)).map_err(|e| e.to_string())?;
        }
        _ => return Err(format!("Unknown GGUF type: {}", val_type)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // detect_model_family — architecture-based
    // -----------------------------------------------------------------------

    #[test]
    fn detect_family_llama() {
        assert_eq!(detect_model_family("llama", None), "llama3");
        assert_eq!(detect_model_family("LLaMA", None), "llama3");
    }

    #[test]
    fn detect_family_mistral() {
        assert_eq!(detect_model_family("mistral", None), "mistral");
        assert_eq!(detect_model_family("Mixtral", None), "mistral");
    }

    #[test]
    fn detect_family_deepseek() {
        assert_eq!(detect_model_family("deepseek", None), "deepseek");
    }

    #[test]
    fn detect_family_glm() {
        assert_eq!(detect_model_family("chatglm", None), "glm");
        assert_eq!(detect_model_family("GLM-4", None), "glm");
    }

    #[test]
    fn detect_family_gemma() {
        assert_eq!(detect_model_family("gemma", None), "gemma");
        assert_eq!(detect_model_family("gemma2", None), "gemma");
    }

    #[test]
    fn detect_family_qwen() {
        assert_eq!(detect_model_family("qwen2", None), "qwen");
    }

    #[test]
    fn detect_family_phi_is_chatml() {
        assert_eq!(detect_model_family("phi3", None), "chatml");
        assert_eq!(detect_model_family("Phi-4", None), "chatml");
    }

    #[test]
    fn detect_family_starcoder_maps_to_llama3() {
        assert_eq!(detect_model_family("starcoder", None), "llama3");
        assert_eq!(detect_model_family("codellama", None), "llama3");
    }

    #[test]
    fn detect_family_default_is_chatml() {
        assert_eq!(detect_model_family("unknown_arch", None), "chatml");
    }

    // -----------------------------------------------------------------------
    // detect_model_family — template-based fallback
    // -----------------------------------------------------------------------

    #[test]
    fn detect_family_from_llama3_template() {
        assert_eq!(detect_model_family("unknown", Some("<|eot_id|>")), "llama3");
        assert_eq!(
            detect_model_family("unknown", Some("<|start_header_id|>assistant")),
            "llama3"
        );
    }

    #[test]
    fn detect_family_from_mistral_template() {
        assert_eq!(
            detect_model_family("unknown", Some("[INST] Hello [/INST]")),
            "mistral"
        );
    }

    #[test]
    fn detect_family_from_gemma_template() {
        assert_eq!(
            detect_model_family("unknown", Some("<start_of_turn>user\nHey<end_of_turn>")),
            "gemma"
        );
    }

    #[test]
    fn detect_family_from_chatml_template() {
        assert_eq!(
            detect_model_family("unknown", Some("<|im_start|>user\nHello<|im_end|>")),
            "qwen"
        );
    }

    #[test]
    fn detect_family_from_glm_template() {
        assert_eq!(detect_model_family("unknown", Some("[gMASK]<sop>")), "glm");
    }

    #[test]
    fn detect_family_arch_takes_priority_over_template() {
        // If architecture is "llama" but template has Mistral tokens,
        // architecture wins
        assert_eq!(
            detect_model_family("llama", Some("[INST] Hello [/INST]")),
            "llama3"
        );
    }

    // -----------------------------------------------------------------------
    // stop_tokens_for_family
    // -----------------------------------------------------------------------

    #[test]
    fn stop_tokens_known_families_non_empty() {
        for family in &[
            "llama3", "mistral", "deepseek", "glm", "gemma", "qwen", "chatml",
        ] {
            let tokens = stop_tokens_for_family(family);
            assert!(
                !tokens.is_empty(),
                "Family '{}' should have stop tokens",
                family
            );
        }
    }

    #[test]
    fn stop_tokens_llama3_has_eot() {
        let tokens = stop_tokens_for_family("llama3");
        assert!(tokens.contains(&"<|eot_id|>".to_string()));
    }

    #[test]
    fn stop_tokens_mistral_has_inst() {
        let tokens = stop_tokens_for_family("mistral");
        assert!(tokens.contains(&"[/INST]".to_string()));
    }

    #[test]
    fn stop_tokens_qwen_and_chatml_are_same() {
        assert_eq!(
            stop_tokens_for_family("qwen"),
            stop_tokens_for_family("chatml")
        );
    }

    #[test]
    fn stop_tokens_unknown_family_has_defaults() {
        let tokens = stop_tokens_for_family("some_future_family");
        assert!(!tokens.is_empty());
        assert!(tokens.contains(&"Human:".to_string()));
    }
}
