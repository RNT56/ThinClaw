use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

// ---------------------------------------------------------------------------
// Scope: determines which index file a document's embeddings live in.
// ---------------------------------------------------------------------------

/// Identifies which vector index a document belongs to.
#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub enum VectorScope {
    /// Documents that aren't tied to any project or chat.
    Global,
    /// Documents uploaded to a specific project.
    Project(String),
    /// Documents uploaded to a standalone (non-project) chat.
    Chat(String),
}

impl VectorScope {
    /// Filename for this scope's index (relative to the vectors directory).
    pub fn filename(&self, dims: usize) -> String {
        match self {
            VectorScope::Global => format!("global_{}.usearch", dims),
            VectorScope::Project(id) => format!("project_{}_{}.usearch", id, dims),
            VectorScope::Chat(id) => format!("chat_{}_{}.usearch", id, dims),
        }
    }
}

// ---------------------------------------------------------------------------
// Single-index wrapper (unchanged API from before, but no longer `Clone`-able
// via Arc externally — the Manager handles that).
// ---------------------------------------------------------------------------

pub struct VectorStore {
    index: Mutex<Index>,
    path: PathBuf,
    dimensions: usize,
}

impl VectorStore {
    pub fn new(path: PathBuf, dimensions: usize) -> Result<Self, String> {
        let options = IndexOptions {
            dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 16,
            expansion_add: 128,
            expansion_search: 64,
            multi: false,
        };

        let mut index =
            Index::new(&options).map_err(|e| format!("Failed to create index: {}", e))?;

        if path.exists() {
            let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
            if metadata.len() > 0 {
                if let Err(e) = index.load(&path.to_string_lossy()) {
                    eprintln!(
                        "[vector_store] Load failed (likely incompatible format): {}. Backing up and Resetting.",
                        e
                    );
                    let backup_path = path.with_extension("usearch.bak");
                    let _ = std::fs::rename(&path, &backup_path);

                    index = Index::new(&options)
                        .map_err(|e| format!("Failed to reset index: {}", e))?;
                } else {
                    println!("[vector_store] Successfully loaded index from {:?}", path);
                }
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }

        Ok(Self {
            index: Mutex::new(index),
            path,
            dimensions,
        })
    }

    pub fn add(&self, id: u64, vector: &[f32]) -> Result<(), String> {
        if vector.len() != self.dimensions {
            return Err(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimensions,
                vector.len()
            ));
        }

        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());

        let current_capacity = index.capacity();
        let current_size = index.size();
        if current_size >= current_capacity {
            let new_capacity =
                std::cmp::max(current_capacity + 1000, current_capacity * 2).max(1000);
            index
                .reserve(new_capacity)
                .map_err(|e| format!("Failed to reserve capacity: {}", e))?;
        }

        index
            .add(id, vector)
            .map_err(|e| format!("Failed to add vector: {}", e))?;
        Ok(())
    }

    pub fn search(&self, vector: &[f32], limit: usize) -> Result<Vec<u64>, String> {
        if vector.len() != self.dimensions {
            return Err(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimensions,
                vector.len()
            ));
        }

        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());

        if index.size() == 0 {
            return Ok(Vec::new());
        }

        let result = index
            .search(vector, limit)
            .map_err(|e| format!("Search failed: {}", e))?;

        Ok(result.keys.to_vec())
    }

    pub fn save(&self) -> Result<(), String> {
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if index.size() > 0 {
            index
                .save(&self.path.to_string_lossy())
                .map_err(|e| format!("Failed to save index: {}", e))
        } else {
            // Don't save empty indices — just remove any stale file
            let _ = std::fs::remove_file(&self.path);
            Ok(())
        }
    }

    pub fn count(&self) -> Result<usize, String> {
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        Ok(index.size())
    }

    pub fn reset(&self) -> Result<(), String> {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());

        let options = IndexOptions {
            dimensions: self.dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 16,
            expansion_add: 128,
            expansion_search: 64,
            multi: false,
        };

        *index = Index::new(&options).map_err(|e| format!("Failed to create new index: {}", e))?;

        // Remove the file on disk
        let _ = std::fs::remove_file(&self.path);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// VectorStoreManager — manages per-scope index files.
// ---------------------------------------------------------------------------

/// Thread-safe manager that lazily creates / caches per-scope `VectorStore`
/// instances.  Registered as Tauri managed state.
#[derive(Clone)]
pub struct VectorStoreManager {
    /// Base directory for all index files (e.g. `app_data/vectors/`).
    base_dir: PathBuf,
    dimensions: usize,
    /// Cached stores keyed by scope.
    stores: Arc<Mutex<HashMap<VectorScope, Arc<VectorStore>>>>,
}

impl VectorStoreManager {
    pub fn new(base_dir: PathBuf, dimensions: usize) -> Result<Self, String> {
        std::fs::create_dir_all(&base_dir)
            .map_err(|e| format!("Failed to create vectors dir: {}", e))?;

        Ok(Self {
            base_dir,
            dimensions,
            stores: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Get (or lazily create) the `VectorStore` for a given scope.
    pub fn get(&self, scope: &VectorScope) -> Result<Arc<VectorStore>, String> {
        let mut map = self.stores.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(store) = map.get(scope) {
            return Ok(store.clone());
        }

        let path = self.base_dir.join(scope.filename(self.dimensions));
        let store = Arc::new(VectorStore::new(path, self.dimensions)?);
        map.insert(scope.clone(), store.clone());
        Ok(store)
    }

    /// Convenience: determine the scope from project_id / chat_id.
    pub fn scope_for(project_id: &Option<String>, chat_id: &Option<String>) -> VectorScope {
        if let Some(pid) = project_id {
            if !pid.is_empty() {
                return VectorScope::Project(pid.clone());
            }
        }
        if let Some(cid) = chat_id {
            if !cid.is_empty() {
                return VectorScope::Chat(cid.clone());
            }
        }
        VectorScope::Global
    }

    /// Save all dirty indices to disk.
    pub fn save_all(&self) -> Result<(), String> {
        let map = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        for (scope, store) in map.iter() {
            if let Err(e) = store.save() {
                eprintln!("[vector_store] Failed to save {:?}: {}", scope, e);
            }
        }
        Ok(())
    }

    /// Reset all loaded indices AND delete every *.usearch file in the
    /// base directory.  Used by "delete all history".
    pub fn reset_all(&self) -> Result<(), String> {
        let mut map = self.stores.lock().unwrap_or_else(|e| e.into_inner());

        // Reset in-memory indices
        for (_, store) in map.drain() {
            let _ = store.reset();
        }

        // Delete all .usearch files on disk
        if let Ok(entries) = std::fs::read_dir(&self.base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("usearch") {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }

        Ok(())
    }

    /// Delete the index for a specific scope (e.g. when a project is deleted).
    pub fn delete_scope(&self, scope: &VectorScope) -> Result<(), String> {
        let mut map = self.stores.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(store) = map.remove(scope) {
            let _ = store.reset();
        }

        let path = self.base_dir.join(scope.filename(self.dimensions));
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    /// Search across multiple scopes and merge results.
    /// Returns (rowid, rank_position) pairs sorted by the index's native
    /// ordering (cosine similarity).
    pub fn search_scoped(
        &self,
        vector: &[f32],
        scopes: &[VectorScope],
        limit: usize,
    ) -> Result<Vec<u64>, String> {
        let mut all_results = Vec::new();

        for scope in scopes {
            if let Ok(store) = self.get(scope) {
                if let Ok(keys) = store.search(vector, limit) {
                    all_results.extend(keys);
                }
            }
        }

        // Deduplicate (same rowid could theoretically appear if global + project overlap — shouldn't
        // happen with proper scoping, but defensive)
        all_results.sort_unstable();
        all_results.dedup();
        Ok(all_results)
    }

    /// Perform integrity check: compare total DB chunks vs total vectors
    /// across all loaded scopes.
    pub fn total_count(&self) -> Result<usize, String> {
        let map = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        let mut total = 0;
        for (_, store) in map.iter() {
            total += store.count()?;
        }
        Ok(total)
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }
}
