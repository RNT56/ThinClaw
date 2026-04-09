//! Hybrid search combining full-text and semantic search.
//!
//! Uses Reciprocal Rank Fusion (RRF) to combine results from:
//! 1. PostgreSQL full-text search (ts_rank_cd)
//! 2. pgvector cosine similarity search
//!
//! RRF formula: score = sum(1 / (k + rank)) for each retrieval method
//! This is robust to different score scales and produces better results
//! than simple score averaging.

use std::collections::HashMap;

use uuid::Uuid;

/// Configuration for hybrid search.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Maximum number of results to return.
    pub limit: usize,
    /// RRF constant (typically 60). Higher values favor top results more.
    pub rrf_k: u32,
    /// Whether to include FTS results.
    pub use_fts: bool,
    /// Whether to include vector results.
    pub use_vector: bool,
    /// Minimum score threshold (0.0-1.0).
    pub min_score: f32,
    /// Maximum results to fetch from each method before fusion.
    pub pre_fusion_limit: usize,
    /// Temporal decay half-life in days. None = no decay.
    /// When set, scores are multiplied by 2^(-age_days / half_life_days).
    pub temporal_decay_half_life_days: Option<f64>,
    /// Enable Maximal Marginal Relevance re-ranking for result diversity.
    pub enable_mmr: bool,
    /// MMR lambda parameter (0.0 = pure diversity, 1.0 = pure relevance).
    /// Only used when `enable_mmr` is true. Default: 0.5.
    pub mmr_lambda: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            limit: 10,
            rrf_k: 60,
            use_fts: true,
            use_vector: true,
            min_score: 0.0,
            pre_fusion_limit: 50,
            temporal_decay_half_life_days: None,
            enable_mmr: false,
            mmr_lambda: 0.5,
        }
    }
}

impl SearchConfig {
    /// Set the result limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set the RRF constant.
    pub fn with_rrf_k(mut self, k: u32) -> Self {
        self.rrf_k = k;
        self
    }

    /// Disable FTS (only use vector search).
    pub fn vector_only(mut self) -> Self {
        self.use_fts = false;
        self.use_vector = true;
        self
    }

    /// Disable vector search (only use FTS).
    pub fn fts_only(mut self) -> Self {
        self.use_fts = true;
        self.use_vector = false;
        self
    }

    /// Set minimum score threshold.
    pub fn with_min_score(mut self, score: f32) -> Self {
        self.min_score = score.clamp(0.0, 1.0);
        self
    }

    /// Enable temporal decay with a half-life in days.
    ///
    /// Older documents have their scores multiplied by `2^(-age / half_life)`,
    /// so a document that is one half-life old scores 50% of a fresh document.
    pub fn with_temporal_decay(mut self, half_life_days: f64) -> Self {
        self.temporal_decay_half_life_days = Some(half_life_days.max(0.1));
        self
    }

    /// Enable Maximal Marginal Relevance re-ranking.
    ///
    /// `lambda` controls the relevance-vs-diversity tradeoff:
    /// - 1.0 = pure relevance (no diversity)
    /// - 0.0 = pure diversity (ignore relevance)
    /// - 0.5 = balanced (default)
    pub fn with_mmr(mut self, lambda: f32) -> Self {
        self.enable_mmr = true;
        self.mmr_lambda = lambda.clamp(0.0, 1.0);
        self
    }
}

/// A search result with hybrid scoring.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Document ID containing this chunk.
    pub document_id: Uuid,
    /// Source path for the matching document.
    pub path: String,
    /// Chunk ID.
    pub chunk_id: Uuid,
    /// Chunk content.
    pub content: String,
    /// Combined RRF score (0.0-1.0 normalized).
    pub score: f32,
    /// Rank in FTS results (1-based, None if not in FTS results).
    pub fts_rank: Option<u32>,
    /// Rank in vector results (1-based, None if not in vector results).
    pub vector_rank: Option<u32>,
}

impl SearchResult {
    /// Check if this result came from FTS.
    pub fn from_fts(&self) -> bool {
        self.fts_rank.is_some()
    }

    /// Check if this result came from vector search.
    pub fn from_vector(&self) -> bool {
        self.vector_rank.is_some()
    }

    /// Check if this result came from both methods (hybrid match).
    pub fn is_hybrid(&self) -> bool {
        self.fts_rank.is_some() && self.vector_rank.is_some()
    }
}

/// Raw result from a single search method.
#[derive(Debug, Clone)]
pub struct RankedResult {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub path: String,
    pub content: String,
    pub rank: u32, // 1-based rank
    /// Optional creation timestamp for temporal decay.
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional embedding vector for MMR diversity calculation.
    pub embedding: Option<Vec<f32>>,
}

/// Reciprocal Rank Fusion algorithm.
///
/// Combines ranked results from multiple retrieval methods using the formula:
/// score(d) = sum(1 / (k + rank(d))) for each method where d appears
///
/// # Arguments
///
/// * `fts_results` - Results from full-text search, ordered by relevance
/// * `vector_results` - Results from vector search, ordered by similarity
/// * `config` - Search configuration
///
/// # Returns
///
/// Combined results sorted by RRF score (descending).
pub fn reciprocal_rank_fusion(
    fts_results: Vec<RankedResult>,
    vector_results: Vec<RankedResult>,
    config: &SearchConfig,
) -> Vec<SearchResult> {
    let k = config.rrf_k as f32;

    // Track scores and metadata for each chunk
    struct ChunkInfo {
        document_id: Uuid,
        path: String,
        content: String,
        score: f32,
        fts_rank: Option<u32>,
        vector_rank: Option<u32>,
    }

    let mut chunk_scores: HashMap<Uuid, ChunkInfo> = HashMap::new();

    // Process FTS results
    for result in fts_results {
        let rrf_score = 1.0 / (k + result.rank as f32);
        chunk_scores
            .entry(result.chunk_id)
            .and_modify(|info| {
                info.score += rrf_score;
                info.fts_rank = Some(result.rank);
            })
            .or_insert(ChunkInfo {
                document_id: result.document_id,
                path: result.path,
                content: result.content,
                score: rrf_score,
                fts_rank: Some(result.rank),
                vector_rank: None,
            });
    }

    // Process vector results
    for result in vector_results {
        let rrf_score = 1.0 / (k + result.rank as f32);
        chunk_scores
            .entry(result.chunk_id)
            .and_modify(|info| {
                info.score += rrf_score;
                info.vector_rank = Some(result.rank);
            })
            .or_insert(ChunkInfo {
                document_id: result.document_id,
                path: result.path,
                content: result.content,
                score: rrf_score,
                fts_rank: None,
                vector_rank: Some(result.rank),
            });
    }

    // Convert to SearchResult and sort by score
    let mut results: Vec<SearchResult> = chunk_scores
        .into_iter()
        .map(|(chunk_id, info)| SearchResult {
            document_id: info.document_id,
            path: info.path,
            chunk_id,
            content: info.content,
            score: info.score,
            fts_rank: info.fts_rank,
            vector_rank: info.vector_rank,
        })
        .collect();

    // Normalize scores to 0-1 range
    if let Some(max_score) = results.iter().map(|r| r.score).reduce(f32::max)
        && max_score > 0.0
    {
        for result in &mut results {
            result.score /= max_score;
        }
    }

    // Filter by minimum score
    if config.min_score > 0.0 {
        results.retain(|r| r.score >= config.min_score);
    }

    // Sort by score descending
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit results
    results.truncate(config.limit);

    results
}

/// Apply temporal decay to search results.
///
/// Multiplies each result's score by `2^(-age_days / half_life_days)`,
/// so documents that are one half-life old score 50%.
pub fn apply_temporal_decay(
    results: &mut [SearchResult],
    half_life_days: f64,
    document_timestamps: &HashMap<Uuid, chrono::DateTime<chrono::Utc>>,
) {
    let now = chrono::Utc::now();

    for result in results.iter_mut() {
        if let Some(created) = document_timestamps.get(&result.document_id) {
            let age_days = (now - *created).num_hours() as f64 / 24.0;
            let decay = (-(age_days / half_life_days) * std::f64::consts::LN_2).exp();
            result.score *= decay as f32;
        }
    }

    // Re-sort by decayed scores.
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Maximal Marginal Relevance re-ranking for result diversity.
///
/// Greedily selects results that balance relevance with novelty,
/// avoiding near-duplicate content in the result set.
///
/// `lambda` = 1.0 is pure relevance, 0.0 is pure diversity.
pub fn mmr_rerank(
    results: Vec<SearchResult>,
    embeddings: &HashMap<Uuid, Vec<f32>>,
    lambda: f32,
    limit: usize,
) -> Vec<SearchResult> {
    if results.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut selected: Vec<SearchResult> = Vec::with_capacity(limit);
    let mut remaining: Vec<SearchResult> = results;

    // First item is always the highest-scoring.
    let first = remaining.remove(0);
    selected.push(first);

    while selected.len() < limit && !remaining.is_empty() {
        let mut best_idx = 0;
        let mut best_mmr = f32::NEG_INFINITY;

        for (i, candidate) in remaining.iter().enumerate() {
            let relevance = candidate.score;

            // Max similarity to any already-selected result.
            let max_sim = selected
                .iter()
                .map(|s| {
                    cosine_similarity(
                        embeddings.get(&candidate.chunk_id),
                        embeddings.get(&s.chunk_id),
                    )
                })
                .fold(f32::NEG_INFINITY, f32::max);

            let mmr = lambda * relevance - (1.0 - lambda) * max_sim;
            if mmr > best_mmr {
                best_mmr = mmr;
                best_idx = i;
            }
        }

        selected.push(remaining.remove(best_idx));
    }

    selected
}

/// Cosine similarity between two embedding vectors.
fn cosine_similarity(a: Option<&Vec<f32>>, b: Option<&Vec<f32>>) -> f32 {
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) if a.len() == b.len() && !a.is_empty() => (a, b),
        _ => return 0.0,
    };

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Expand a search query by extracting key terms and generating alternate phrasings.
///
/// This is a lightweight, LLM-free query expansion that:
/// 1. Tokenizes the query into words
/// 2. Removes stop words
/// 3. Generates simple morphological variants (basic stemming heuristics)
/// 4. Returns expanded terms that can be ORed into the FTS query
///
/// For full LLM-based query expansion, the caller should send the query to
/// an LLM with appropriate instructions and use the expanded result.
pub fn expand_query_keywords(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "by", "do", "for", "from", "has", "have", "how",
        "i", "in", "is", "it", "its", "my", "not", "of", "on", "or", "so", "that", "the", "this",
        "to", "was", "what", "when", "where", "which", "who", "will", "with", "you",
    ];

    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|w| !w.is_empty() && !STOP_WORDS.contains(&w.as_str()))
        .collect();

    let mut expanded: Vec<String> = words.clone();

    for word in &words {
        // Simple suffix stripping for common English suffixes.
        if let Some(stem) = word.strip_suffix("ing") {
            if stem.len() >= 3 {
                expanded.push(stem.to_string());
                // "running" -> "run" + "runner"
                expanded.push(format!("{}er", stem));
            }
        } else if let Some(stem) = word.strip_suffix("tion") {
            if stem.len() >= 2 {
                expanded.push(format!("{}te", stem));
            }
        } else if let Some(stem) = word.strip_suffix("ed") {
            if stem.len() >= 3 {
                expanded.push(stem.to_string());
            }
        } else if let Some(stem) = word.strip_suffix("ly") {
            if stem.len() >= 3 {
                expanded.push(stem.to_string());
            }
        } else if let Some(stem) = word.strip_suffix("ies") {
            if stem.len() >= 2 {
                expanded.push(format!("{}y", stem));
            }
        } else if let Some(stem) = word.strip_suffix('s')
            && stem.len() >= 3
        {
            expanded.push(stem.to_string());
        }
    }

    // Deduplicate.
    expanded.sort();
    expanded.dedup();
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(chunk_id: Uuid, doc_id: Uuid, rank: u32) -> RankedResult {
        RankedResult {
            chunk_id,
            document_id: doc_id,
            path: format!("docs/{}.md", doc_id),
            content: format!("content for chunk {}", chunk_id),
            rank,
            created_at: None,
            embedding: None,
        }
    }

    #[test]
    fn test_rrf_single_method() {
        let config = SearchConfig::default().with_limit(10);

        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts_results = vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 2)];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        assert_eq!(results.len(), 2);
        // First result should have higher score
        assert!(results[0].score > results[1].score);
        // All should have FTS rank
        assert!(results.iter().all(|r| r.fts_rank.is_some()));
        assert!(results.iter().all(|r| r.vector_rank.is_none()));
    }

    #[test]
    fn test_rrf_hybrid_match_boosted() {
        let config = SearchConfig::default().with_limit(10);

        let chunk1 = Uuid::new_v4(); // In both
        let chunk2 = Uuid::new_v4(); // FTS only
        let chunk3 = Uuid::new_v4(); // Vector only
        let doc = Uuid::new_v4();

        let fts_results = vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 2)];

        let vector_results = vec![make_result(chunk1, doc, 1), make_result(chunk3, doc, 2)];

        let results = reciprocal_rank_fusion(fts_results, vector_results, &config);

        assert_eq!(results.len(), 3);

        // chunk1 should be first (hybrid match)
        assert_eq!(results[0].chunk_id, chunk1);
        assert!(results[0].is_hybrid());
        assert!(results[0].score > results[1].score);

        // Other chunks should not be hybrid
        assert!(!results[1].is_hybrid());
        assert!(!results[2].is_hybrid());
    }

    #[test]
    fn test_rrf_score_normalization() {
        let config = SearchConfig::default();

        let chunk1 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts_results = vec![make_result(chunk1, doc, 1)];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        // Single result should have normalized score of 1.0
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_rrf_min_score_filter() {
        let config = SearchConfig::default().with_limit(10).with_min_score(0.5);

        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let chunk3 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        // chunk1 has rank 1, chunk3 has rank 100 (low score)
        let fts_results = vec![
            make_result(chunk1, doc, 1),
            make_result(chunk2, doc, 50),
            make_result(chunk3, doc, 100),
        ];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        // Low-scoring results should be filtered out
        // All results should have score >= 0.5
        for result in &results {
            assert!(result.score >= 0.5);
        }
    }

    #[test]
    fn test_rrf_limit() {
        let config = SearchConfig::default().with_limit(2);

        let doc = Uuid::new_v4();
        let fts_results: Vec<_> = (1..=5)
            .map(|i| make_result(Uuid::new_v4(), doc, i))
            .collect();

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_rrf_k_parameter() {
        // Higher k values make ranking differences less pronounced
        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts_results = vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 2)];

        // Low k: rank 1 score = 1/(10+1) = 0.091, rank 2 = 1/(10+2) = 0.083
        let config_low_k = SearchConfig::default().with_rrf_k(10);
        let results_low = reciprocal_rank_fusion(fts_results.clone(), Vec::new(), &config_low_k);

        // High k: rank 1 score = 1/(100+1) = 0.0099, rank 2 = 1/(100+2) = 0.0098
        let config_high_k = SearchConfig::default().with_rrf_k(100);
        let results_high = reciprocal_rank_fusion(fts_results, Vec::new(), &config_high_k);

        // With low k, the score difference is larger (relatively)
        let diff_low = results_low[0].score - results_low[1].score;
        let diff_high = results_high[0].score - results_high[1].score;

        // Low k should have larger relative difference
        assert!(diff_low > diff_high);
    }

    #[test]
    fn test_search_config_builders() {
        let config = SearchConfig::default()
            .with_limit(20)
            .with_rrf_k(30)
            .with_min_score(0.1);

        assert_eq!(config.limit, 20);
        assert_eq!(config.rrf_k, 30);
        assert!((config.min_score - 0.1).abs() < 0.001);
        assert!(config.use_fts);
        assert!(config.use_vector);

        let fts_only = SearchConfig::default().fts_only();
        assert!(fts_only.use_fts);
        assert!(!fts_only.use_vector);

        let vector_only = SearchConfig::default().vector_only();
        assert!(!vector_only.use_fts);
        assert!(vector_only.use_vector);
    }

    #[test]
    fn test_temporal_decay() {
        let doc_id = Uuid::new_v4();
        let chunk_old = Uuid::new_v4();
        let chunk_new = Uuid::new_v4();

        let mut results = vec![
            SearchResult {
                document_id: doc_id,
                path: "docs/old.md".to_string(),
                chunk_id: chunk_old,
                content: "old".to_string(),
                score: 1.0,
                fts_rank: Some(1),
                vector_rank: None,
            },
            SearchResult {
                document_id: doc_id,
                path: "docs/new.md".to_string(),
                chunk_id: chunk_new,
                content: "new".to_string(),
                score: 0.8,
                fts_rank: Some(2),
                vector_rank: None,
            },
        ];

        let now = chrono::Utc::now();
        let mut timestamps = HashMap::new();
        timestamps.insert(doc_id, now); // Same doc, but let's test with just timestamps

        // With no age, decay should be ~1.0
        apply_temporal_decay(&mut results, 30.0, &timestamps);
        assert!(results[0].score > 0.9);
    }

    #[test]
    fn test_mmr_rerank_diversity() {
        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let chunk3 = Uuid::new_v4();

        let results = vec![
            SearchResult {
                document_id: Uuid::new_v4(),
                path: "docs/a.md".to_string(),
                chunk_id: chunk1,
                content: "a".to_string(),
                score: 1.0,
                fts_rank: Some(1),
                vector_rank: None,
            },
            SearchResult {
                document_id: Uuid::new_v4(),
                path: "docs/b.md".to_string(),
                chunk_id: chunk2,
                content: "b".to_string(),
                score: 0.9,
                fts_rank: Some(2),
                vector_rank: None,
            },
            SearchResult {
                document_id: Uuid::new_v4(),
                path: "docs/c.md".to_string(),
                chunk_id: chunk3,
                content: "c".to_string(),
                score: 0.8,
                fts_rank: Some(3),
                vector_rank: None,
            },
        ];

        // Identical embeddings for chunk1 and chunk2, different for chunk3.
        let mut embeddings = HashMap::new();
        embeddings.insert(chunk1, vec![1.0, 0.0, 0.0]);
        embeddings.insert(chunk2, vec![1.0, 0.0, 0.0]); // same as chunk1
        embeddings.insert(chunk3, vec![0.0, 1.0, 0.0]); // orthogonal

        // With pure diversity (lambda=0), chunk3 should be selected over chunk2
        let reranked = mmr_rerank(results, &embeddings, 0.0, 2);
        assert_eq!(reranked.len(), 2);
        assert_eq!(reranked[0].chunk_id, chunk1); // always first
        assert_eq!(reranked[1].chunk_id, chunk3); // diverse pick
    }

    #[test]
    fn test_expand_query_keywords() {
        let expanded = expand_query_keywords("running configuration files");
        assert!(expanded.contains(&"running".to_string()));
        assert!(expanded.contains(&"configuration".to_string()));
        assert!(expanded.contains(&"file".to_string())); // "files" -> "file"
        assert!(expanded.contains(&"runn".to_string())); // "running" -> "runn" (strip "ing")
        assert!(!expanded.contains(&"the".to_string())); // stop word
    }

    #[test]
    fn test_expand_query_dedup() {
        let expanded = expand_query_keywords("test test test");
        // Should only have "test" once.
        assert_eq!(expanded.iter().filter(|w| *w == "test").count(), 1);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(Some(&a), Some(&b));
        assert!((sim - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(Some(&a), Some(&a));
        assert!((sim - 1.0).abs() < 0.001);
    }
}
