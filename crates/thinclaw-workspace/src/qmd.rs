//! QMD (Quantized Memory Database) embedding backend.
//!
//! A lightweight, in-process vector backend using product quantization
//! for memory-efficient approximate nearest neighbor search.

use serde::{Deserialize, Serialize};

/// Configuration for the QMD backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QmdConfig {
    /// Path to the QMD data file.
    pub data_path: String,
    /// Vector dimensions.
    pub dimensions: u32,
    /// Number of sub-quantizers.
    pub num_subquantizers: u32,
    /// Bits per sub-quantizer code.
    pub bits_per_code: u8,
    /// Number of centroids for training.
    pub num_centroids: u32,
    /// Maximum entries before compaction.
    pub max_entries: usize,
    /// Whether to memory-map the data file.
    pub mmap: bool,
}

impl Default for QmdConfig {
    fn default() -> Self {
        Self {
            data_path: "~/.thinclaw/qmd.bin".to_string(),
            dimensions: 1536,
            num_subquantizers: 8,
            bits_per_code: 8,
            num_centroids: 256,
            max_entries: 100_000,
            mmap: true,
        }
    }
}

impl QmdConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(path) = std::env::var("QMD_DATA_PATH") {
            config.data_path = path;
        }
        if let Ok(dims) = std::env::var("QMD_DIMENSIONS")
            && let Ok(d) = dims.parse()
        {
            config.dimensions = d;
        }
        if let Ok(sq) = std::env::var("QMD_SUBQUANTIZERS")
            && let Ok(n) = sq.parse()
        {
            config.num_subquantizers = n;
        }
        config
    }

    /// Bytes per quantized vector.
    pub fn bytes_per_vector(&self) -> usize {
        (self.num_subquantizers as usize) * (self.bits_per_code as usize / 8).max(1)
    }

    /// Estimated memory usage for max_entries vectors.
    pub fn estimated_memory_bytes(&self) -> usize {
        self.max_entries * self.bytes_per_vector() + self.codebook_size() + 1024 // header overhead
    }

    /// Codebook size in bytes.
    pub fn codebook_size(&self) -> usize {
        let sub_dim = self.dimensions as usize / self.num_subquantizers as usize;
        self.num_subquantizers as usize * self.num_centroids as usize * sub_dim * 4
    }

    /// Resolve the data path (expand ~).
    pub fn resolved_path(&self) -> String {
        if self.data_path.starts_with("~/")
            && let Ok(home) = std::env::var("HOME")
        {
            return format!("{}{}", home, &self.data_path[1..]);
        }
        self.data_path.clone()
    }

    /// Validate configuration.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.dimensions == 0 {
            errors.push("Dimensions must be > 0".to_string());
        }
        if !self.dimensions.is_multiple_of(self.num_subquantizers) {
            errors.push(format!(
                "Dimensions ({}) must be divisible by num_subquantizers ({})",
                self.dimensions, self.num_subquantizers
            ));
        }
        if self.num_centroids == 0 || (self.num_centroids & (self.num_centroids - 1)) != 0 {
            errors.push("num_centroids must be a power of 2".to_string());
        }
        errors
    }
}

/// A quantized vector entry.
#[derive(Debug, Clone)]
pub struct QmdEntry {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    /// Quantized codes (length = num_subquantizers).
    pub codes: Vec<u8>,
    pub content: String,
}

/// QMD search result.
#[derive(Debug, Clone)]
pub struct QmdSearchResult {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub content: String,
    /// Approximate distance.
    pub distance: f32,
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = QmdConfig::default();
        assert_eq!(config.dimensions, 1536);
        assert_eq!(config.num_subquantizers, 8);
        assert!(config.mmap);
    }

    #[test]
    fn test_bytes_per_vector() {
        let config = QmdConfig::default();
        assert_eq!(config.bytes_per_vector(), 8);
    }

    #[test]
    fn test_codebook_size() {
        let config = QmdConfig {
            dimensions: 768,
            num_subquantizers: 8,
            num_centroids: 256,
            ..Default::default()
        };
        // 8 subquantizers * 256 centroids * (768/8) sub_dim * 4 bytes
        assert_eq!(config.codebook_size(), 8 * 256 * 96 * 4);
    }

    #[test]
    fn test_validate_valid() {
        let config = QmdConfig::default();
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_validate_bad_divisibility() {
        let config = QmdConfig {
            dimensions: 100,
            num_subquantizers: 8,
            ..Default::default()
        };
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("divisible")));
    }

    #[test]
    fn test_validate_bad_centroids() {
        let config = QmdConfig {
            num_centroids: 100, // not power of 2
            ..Default::default()
        };
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("power of 2")));
    }

    #[test]
    fn test_estimated_memory() {
        let config = QmdConfig::default();
        let mem = config.estimated_memory_bytes();
        assert!(mem > 100_000); // At least 100KB
    }
}
