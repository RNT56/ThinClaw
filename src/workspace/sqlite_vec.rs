//! SQLite-vec embedding backend.
//!
//! An alternative to PostgreSQL for vector storage, using sqlite-vec
//! virtual tables for approximate nearest neighbor search.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for the SQLite-vec backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteVecConfig {
    /// Path to the SQLite database file.
    pub db_path: String,
    /// Vector dimensions.
    pub dimensions: u32,
    /// Distance metric.
    pub distance_metric: DistanceMetric,
    /// Maximum number of results per query.
    pub max_results: usize,
    /// Whether to create the vec0 virtual table on startup.
    pub auto_create: bool,
}

/// Distance metrics for vector search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DistanceMetric {
    Cosine,
    L2,
    InnerProduct,
}

impl Default for SqliteVecConfig {
    fn default() -> Self {
        Self {
            db_path: "~/.ironclaw/embeddings.db".to_string(),
            dimensions: 1536,
            distance_metric: DistanceMetric::Cosine,
            max_results: 20,
            auto_create: true,
        }
    }
}

impl SqliteVecConfig {
    /// Create config from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(path) = std::env::var("SQLITE_VEC_DB") {
            config.db_path = path;
        }
        if let Ok(dims) = std::env::var("SQLITE_VEC_DIMENSIONS")
            && let Ok(d) = dims.parse()
        {
            config.dimensions = d;
        }
        if let Ok(metric) = std::env::var("SQLITE_VEC_METRIC") {
            config.distance_metric = match metric.to_lowercase().as_str() {
                "l2" | "euclidean" => DistanceMetric::L2,
                "ip" | "inner_product" => DistanceMetric::InnerProduct,
                _ => DistanceMetric::Cosine,
            };
        }
        config
    }

    /// SQL to create the vec0 virtual table.
    pub fn create_table_sql(&self) -> String {
        format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_embeddings USING vec0(\n  id TEXT PRIMARY KEY,\n  embedding float[{}],\n  +document_id TEXT,\n  +chunk_index INTEGER,\n  +content TEXT\n);",
            self.dimensions
        )
    }

    /// SQL for the nearest neighbor query.
    pub fn search_sql(&self) -> String {
        format!(
            "SELECT id, document_id, chunk_index, content, distance \
             FROM vec_embeddings \
             WHERE embedding MATCH ? \
             ORDER BY distance \
             LIMIT {};",
            self.max_results
        )
    }

    /// SQL to insert an embedding.
    pub fn insert_sql() -> &'static str {
        "INSERT INTO vec_embeddings (id, embedding, document_id, chunk_index, content) VALUES (?, ?, ?, ?, ?)"
    }

    /// SQL to delete embeddings for a document.
    pub fn delete_sql() -> &'static str {
        "DELETE FROM vec_embeddings WHERE document_id = ?"
    }

    /// Resolve the actual path (expand ~).
    pub fn resolved_path(&self) -> String {
        if self.db_path.starts_with("~/")
            && let Ok(home) = std::env::var("HOME")
        {
            return format!("{}{}", home, &self.db_path[1..]);
        }
        self.db_path.clone()
    }
}

/// A vector search result from SQLite-vec.
#[derive(Debug, Clone)]
pub struct VecSearchResult {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub content: String,
    pub distance: f32,
    pub score: f32,
}

/// Normalize a distance to a 0..1 score (cosine: 1 - dist).
pub fn distance_to_score(distance: f32, metric: &DistanceMetric) -> f32 {
    match metric {
        DistanceMetric::Cosine => (1.0 - distance).max(0.0),
        DistanceMetric::L2 => 1.0 / (1.0 + distance),
        DistanceMetric::InnerProduct => distance.max(0.0),
    }
}

/// Stats for the SQLite-vec store.
#[derive(Debug, Clone, Default)]
pub struct VecStoreStats {
    pub total_vectors: u64,
    pub dimensions: u32,
    pub db_size_bytes: u64,
    pub documents: HashMap<String, u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SqliteVecConfig::default();
        assert_eq!(config.dimensions, 1536);
        assert_eq!(config.distance_metric, DistanceMetric::Cosine);
        assert!(config.auto_create);
    }

    #[test]
    fn test_create_table_sql() {
        let config = SqliteVecConfig {
            dimensions: 768,
            ..Default::default()
        };
        let sql = config.create_table_sql();
        assert!(sql.contains("float[768]"));
        assert!(sql.contains("vec0"));
    }

    #[test]
    fn test_search_sql() {
        let config = SqliteVecConfig {
            max_results: 10,
            ..Default::default()
        };
        let sql = config.search_sql();
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("MATCH"));
    }

    #[test]
    fn test_insert_sql() {
        assert!(SqliteVecConfig::insert_sql().contains("INSERT"));
    }

    #[test]
    fn test_distance_to_score_cosine() {
        assert!((distance_to_score(0.1, &DistanceMetric::Cosine) - 0.9).abs() < 0.001);
        assert!((distance_to_score(0.0, &DistanceMetric::Cosine) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_distance_to_score_l2() {
        assert!(distance_to_score(0.0, &DistanceMetric::L2) > 0.99);
        assert!(distance_to_score(1.0, &DistanceMetric::L2) > 0.4);
    }

    #[test]
    fn test_resolved_path() {
        let config = SqliteVecConfig {
            db_path: "/tmp/test.db".to_string(),
            ..Default::default()
        };
        assert_eq!(config.resolved_path(), "/tmp/test.db");
    }
}
