//! LanceDB embedding backend.
//!
//! Columnar vector store built on Apache Arrow, supporting
//! efficient nearest-neighbor search with auto-scaling.

use serde::{Deserialize, Serialize};

/// Configuration for the LanceDB backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanceDbConfig {
    /// URI for the LanceDB instance (local path or S3).
    pub uri: String,
    /// Table name for embeddings.
    pub table_name: String,
    /// Vector dimensions.
    pub dimensions: u32,
    /// Number of sub-vectors for IVF index.
    pub num_sub_vectors: Option<u32>,
    /// Number of partitions for IVF index.
    pub num_partitions: Option<u32>,
    /// Maximum content length to auto-capture.
    pub max_capture_length: usize,
    /// Metric type.
    pub metric: LanceMetric,
}

/// LanceDB distance metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LanceMetric {
    L2,
    Cosine,
    Dot,
}

impl Default for LanceDbConfig {
    fn default() -> Self {
        Self {
            uri: "~/.ironclaw/lancedb".to_string(),
            table_name: "embeddings".to_string(),
            dimensions: 1536,
            num_sub_vectors: None,
            num_partitions: None,
            max_capture_length: 8192,
            metric: LanceMetric::Cosine,
        }
    }
}

impl LanceDbConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(uri) = std::env::var("LANCEDB_URI") {
            config.uri = uri;
        }
        if let Ok(table) = std::env::var("LANCEDB_TABLE") {
            config.table_name = table;
        }
        if let Ok(dims) = std::env::var("LANCEDB_DIMENSIONS") {
            if let Ok(d) = dims.parse() {
                config.dimensions = d;
            }
        }
        if let Ok(max) = std::env::var("LANCEDB_MAX_CAPTURE") {
            if let Ok(m) = max.parse() {
                config.max_capture_length = m;
            }
        }
        config
    }

    /// Whether this is a local (file-based) store.
    pub fn is_local(&self) -> bool {
        !self.uri.starts_with("s3://") && !self.uri.starts_with("gs://")
    }

    /// Resolve the URI (expand ~).
    pub fn resolved_uri(&self) -> String {
        if self.uri.starts_with("~/") {
            if let Ok(home) = std::env::var("HOME") {
                return format!("{}{}", home, &self.uri[1..]);
            }
        }
        self.uri.clone()
    }

    /// Schema for the embeddings table (Arrow-compatible).
    pub fn schema_description(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: "id".into(),
                dtype: "utf8".into(),
                nullable: false,
            },
            ColumnDef {
                name: "document_id".into(),
                dtype: "utf8".into(),
                nullable: false,
            },
            ColumnDef {
                name: "chunk_index".into(),
                dtype: "int32".into(),
                nullable: false,
            },
            ColumnDef {
                name: "content".into(),
                dtype: "utf8".into(),
                nullable: true,
            },
            ColumnDef {
                name: "vector".into(),
                dtype: format!("fixed_size_list[float32, {}]", self.dimensions),
                nullable: false,
            },
            ColumnDef {
                name: "created_at".into(),
                dtype: "timestamp[ms]".into(),
                nullable: false,
            },
        ]
    }
}

/// Arrow column definition.
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub dtype: String,
    pub nullable: bool,
}

/// Lance search query options.
#[derive(Debug, Clone)]
pub struct LanceSearchOptions {
    pub vector: Vec<f32>,
    pub limit: usize,
    pub filter: Option<String>,
    pub nprobes: Option<u32>,
    pub refine_factor: Option<u32>,
}

impl Default for LanceSearchOptions {
    fn default() -> Self {
        Self {
            vector: Vec::new(),
            limit: 20,
            filter: None,
            nprobes: Some(20),
            refine_factor: Some(10),
        }
    }
}

/// Lance search result.
#[derive(Debug, Clone)]
pub struct LanceSearchResult {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub content: String,
    pub distance: f32,
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LanceDbConfig::default();
        assert_eq!(config.table_name, "embeddings");
        assert_eq!(config.dimensions, 1536);
        assert_eq!(config.max_capture_length, 8192);
    }

    #[test]
    fn test_is_local() {
        let config = LanceDbConfig::default();
        assert!(config.is_local());

        let s3 = LanceDbConfig {
            uri: "s3://bucket/path".into(),
            ..Default::default()
        };
        assert!(!s3.is_local());
    }

    #[test]
    fn test_resolved_uri() {
        let config = LanceDbConfig {
            uri: "/tmp/lance".into(),
            ..Default::default()
        };
        assert_eq!(config.resolved_uri(), "/tmp/lance");
    }

    #[test]
    fn test_schema() {
        let config = LanceDbConfig {
            dimensions: 768,
            ..Default::default()
        };
        let schema = config.schema_description();
        assert_eq!(schema.len(), 6);
        assert!(schema[4].dtype.contains("768"));
    }

    #[test]
    fn test_search_defaults() {
        let opts = LanceSearchOptions::default();
        assert_eq!(opts.limit, 20);
        assert_eq!(opts.nprobes, Some(20));
    }

    #[test]
    fn test_metric_variants() {
        assert_eq!(LanceMetric::Cosine, LanceMetric::Cosine);
        assert_ne!(LanceMetric::L2, LanceMetric::Dot);
    }
}
