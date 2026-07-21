use serde::Serialize;
use specta::Type;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

const MAX_GGUF_KV_COUNT: u64 = 1_000_000;
const MAX_GGUF_TENSOR_COUNT: u64 = 100_000_000;
const MAX_GGUF_METADATA_BYTES: u64 = 512 * 1024 * 1024;
const MAX_GGUF_KEY_BYTES: u64 = 64 * 1024;
const MAX_GGUF_VALUE_STRING_BYTES: u64 = 4 * 1024 * 1024;
const MAX_GGUF_CHAT_TEMPLATE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_GGUF_SKIPPED_STRING_BYTES: u64 = 16 * 1024 * 1024;
const MAX_GGUF_ARRAY_ITEMS: u64 = 20_000_000;
const MAX_GGUF_NESTING: usize = 8;

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
    let path = std::path::Path::new(path);
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("GGUF model must be a regular, non-symlink file".to_string());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = BoundedGgufReader::new(file, metadata.len());

    // Read Magic
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err("Not a GGUF file".to_string());
    }

    // Version
    let version = reader.read_u32()?;
    if version != 2 && version != 3 {
        return Err(format!("Unsupported GGUF version: {}", version));
    }

    // Tensor Count
    let tensor_count = reader.read_u64()?;
    if tensor_count > MAX_GGUF_TENSOR_COUNT {
        return Err("GGUF tensor count exceeds the supported limit".to_string());
    }

    // Metadata KV Count
    let kv_count = reader.read_u64()?;
    if kv_count > MAX_GGUF_KV_COUNT {
        return Err("GGUF metadata entry count exceeds the supported limit".to_string());
    }

    let mut metadata = GGUFMetadata::default();

    for _ in 0..kv_count {
        let key = reader.read_string(MAX_GGUF_KEY_BYTES, "metadata key")?;
        let val_type = reader.read_u32()?;

        match key.as_str() {
            "general.architecture" => {
                metadata.architecture = read_value_string(&mut reader, val_type)?;
            }
            "tokenizer.chat_template" => {
                if val_type == 8 {
                    metadata.chat_template =
                        Some(reader.read_string(MAX_GGUF_CHAT_TEMPLATE_BYTES, "chat template")?);
                } else {
                    skip_value(&mut reader, val_type, 0)?;
                }
            }
            _ if key.ends_with(".context_length") => {
                metadata.context_length = read_value_u64(&mut reader, val_type)?;
            }
            _ if key.ends_with(".embedding_length") => {
                metadata.embedding_length = read_value_u64(&mut reader, val_type)?;
            }
            _ if key.ends_with(".block_count") => {
                metadata.block_count = read_value_u64(&mut reader, val_type)?;
            }
            _ if key.ends_with(".attention.head_count") => {
                metadata.head_count = read_value_u64(&mut reader, val_type)?;
            }
            _ if key.ends_with(".attention.head_count_kv") => {
                metadata.head_count_kv = read_value_u64(&mut reader, val_type)?;
            }
            "general.file_type" => {
                metadata.file_type = read_value_u32(&mut reader, val_type)?;
            }
            _ => {
                skip_value(&mut reader, val_type, 0)?;
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

struct BoundedGgufReader {
    file: File,
    file_len: u64,
    position: u64,
}

impl BoundedGgufReader {
    fn new(file: File, file_len: u64) -> Self {
        Self {
            file,
            file_len,
            position: 0,
        }
    }

    fn reserve(&self, length: u64) -> Result<u64, String> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| "GGUF metadata offset overflowed".to_string())?;
        if end > self.file_len {
            return Err("GGUF metadata is truncated".to_string());
        }
        if end > MAX_GGUF_METADATA_BYTES {
            return Err("GGUF metadata exceeds the supported scan limit".to_string());
        }
        Ok(end)
    }

    fn read_exact(&mut self, buffer: &mut [u8]) -> Result<(), String> {
        let length = u64::try_from(buffer.len())
            .map_err(|_| "GGUF read length exceeds the supported range".to_string())?;
        let end = self.reserve(length)?;
        self.file
            .read_exact(buffer)
            .map_err(|error| error.to_string())?;
        self.position = end;
        Ok(())
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let mut bytes = [0_u8; 4];
        self.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, String> {
        let mut bytes = [0_u8; 8];
        self.read_exact(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn skip(&mut self, length: u64) -> Result<(), String> {
        let end = self.reserve(length)?;
        self.file
            .seek(SeekFrom::Start(end))
            .map_err(|error| error.to_string())?;
        self.position = end;
        Ok(())
    }

    fn read_string(&mut self, limit: u64, label: &str) -> Result<String, String> {
        let length = self.read_u64()?;
        if length > limit {
            return Err(format!("GGUF {label} exceeds the {limit}-byte limit"));
        }
        let length = usize::try_from(length)
            .map_err(|_| format!("GGUF {label} length exceeds this platform's range"))?;
        let mut bytes = vec![0_u8; length];
        self.read_exact(&mut bytes)?;
        String::from_utf8(bytes).map_err(|_| format!("GGUF {label} is not valid UTF-8"))
    }

    fn skip_string(&mut self) -> Result<(), String> {
        let length = self.read_u64()?;
        if length > MAX_GGUF_SKIPPED_STRING_BYTES {
            return Err("GGUF string value exceeds the supported limit".to_string());
        }
        self.skip(length)
    }
}

fn read_value_string(reader: &mut BoundedGgufReader, val_type: u32) -> Result<String, String> {
    if val_type != 8 {
        return Err("Expected string".to_string());
    }
    reader.read_string(MAX_GGUF_VALUE_STRING_BYTES, "string value")
}

fn read_value_u64(reader: &mut BoundedGgufReader, val_type: u32) -> Result<u64, String> {
    match val_type {
        4 => Ok(u64::from(reader.read_u32()?)),
        10 => reader.read_u64(),
        _ => Err(format!("Expected UINT32/UINT64, got type {}", val_type)),
    }
}

fn read_value_u32(reader: &mut BoundedGgufReader, val_type: u32) -> Result<u32, String> {
    if val_type != 4 {
        return Err("Expected UINT32".to_string());
    }
    reader.read_u32()
}

fn scalar_size(value_type: u32) -> Option<u64> {
    match value_type {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    }
}

fn skip_array(
    reader: &mut BoundedGgufReader,
    element_type: u32,
    length: u64,
    depth: usize,
) -> Result<(), String> {
    if length > MAX_GGUF_ARRAY_ITEMS {
        return Err("GGUF array length exceeds the supported limit".to_string());
    }
    if let Some(size) = scalar_size(element_type) {
        let bytes = length
            .checked_mul(size)
            .ok_or_else(|| "GGUF array byte length overflowed".to_string())?;
        return reader.skip(bytes);
    }
    match element_type {
        8 => {
            for _ in 0..length {
                reader.skip_string()?;
            }
            Ok(())
        }
        9 => {
            if depth >= MAX_GGUF_NESTING {
                return Err("GGUF array nesting exceeds the supported limit".to_string());
            }
            for _ in 0..length {
                let nested_type = reader.read_u32()?;
                let nested_length = reader.read_u64()?;
                skip_array(reader, nested_type, nested_length, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(format!("Unknown GGUF array element type: {element_type}")),
    }
}

fn skip_value(reader: &mut BoundedGgufReader, val_type: u32, depth: usize) -> Result<(), String> {
    if let Some(size) = scalar_size(val_type) {
        return reader.skip(size);
    }
    match val_type {
        8 => reader.skip_string(),
        9 => {
            if depth >= MAX_GGUF_NESTING {
                return Err("GGUF value nesting exceeds the supported limit".to_string());
            }
            let element_type = reader.read_u32()?;
            let length = reader.read_u64()?;
            skip_array(reader, element_type, length, depth + 1)
        }
        _ => Err(format!("Unknown GGUF type: {val_type}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_header(file: &mut File, tensor_count: u64, kv_count: u64) {
        file.write_all(b"GGUF").unwrap();
        file.write_all(&3_u32.to_le_bytes()).unwrap();
        file.write_all(&tensor_count.to_le_bytes()).unwrap();
        file.write_all(&kv_count.to_le_bytes()).unwrap();
    }

    fn write_string(file: &mut File, value: &[u8]) {
        file.write_all(&(value.len() as u64).to_le_bytes()).unwrap();
        file.write_all(value).unwrap();
    }

    #[test]
    fn parses_a_bounded_minimal_header() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp.reopen().unwrap();
        write_header(&mut file, 0, 1);
        write_string(&mut file, b"general.architecture");
        file.write_all(&8_u32.to_le_bytes()).unwrap();
        write_string(&mut file, b"llama");
        drop(file);

        let metadata = read_gguf_metadata(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(metadata.architecture, "llama");
        assert_eq!(metadata.model_family.as_deref(), Some("llama3"));
    }

    #[test]
    fn rejects_hostile_counts_and_string_lengths_before_allocation() {
        let excessive_count = tempfile::NamedTempFile::new().unwrap();
        let mut file = excessive_count.reopen().unwrap();
        write_header(&mut file, 0, MAX_GGUF_KV_COUNT + 1);
        drop(file);
        assert!(read_gguf_metadata(excessive_count.path().to_str().unwrap()).is_err());

        let excessive_key = tempfile::NamedTempFile::new().unwrap();
        let mut file = excessive_key.reopen().unwrap();
        write_header(&mut file, 0, 1);
        file.write_all(&(MAX_GGUF_KEY_BYTES + 1).to_le_bytes())
            .unwrap();
        drop(file);
        assert!(read_gguf_metadata(excessive_key.path().to_str().unwrap()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_model_files() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let model = temp.path().join("model.gguf");
        let link = temp.path().join("link.gguf");
        let mut file = File::create(&model).unwrap();
        write_header(&mut file, 0, 0);
        drop(file);
        symlink(&model, &link).unwrap();
        assert!(read_gguf_metadata(link.to_str().unwrap()).is_err());
    }

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
