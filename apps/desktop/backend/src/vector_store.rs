use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const MAX_VECTOR_DIMENSIONS: usize = 65_536;
const MAX_SCOPE_ID_BYTES: usize = 256;
const MAX_INDEX_FILE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

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
    pub fn filename(&self, dims: usize) -> Result<String, String> {
        if dims == 0 || dims > MAX_VECTOR_DIMENSIONS {
            return Err("Vector dimensions are outside the supported range".to_string());
        }
        match self {
            VectorScope::Global => Ok(format!("global_{dims}.usearch")),
            VectorScope::Project(id) => {
                Ok(format!("project_{}_{dims}.usearch", scope_id_digest(id)?))
            }
            VectorScope::Chat(id) => Ok(format!("chat_{}_{dims}.usearch", scope_id_digest(id)?)),
        }
    }
}

fn scope_id_digest(id: &str) -> Result<String, String> {
    if id.is_empty() || id.len() > MAX_SCOPE_ID_BYTES || id.chars().any(char::is_control) {
        return Err("Vector scope identifier is invalid".to_string());
    }
    let digest = Sha256::digest(id.as_bytes());
    Ok(hex::encode(&digest[..16]))
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
        if dimensions == 0 || dimensions > MAX_VECTOR_DIMENSIONS {
            return Err("Vector dimensions are outside the supported range".to_string());
        }
        let options = IndexOptions {
            dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 16,
            expansion_add: 128,
            expansion_search: 64,
            multi: false,
        };

        let index = Index::new(&options).map_err(|e| format!("Failed to create index: {}", e))?;

        if path.exists() {
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() > MAX_INDEX_FILE_BYTES
            {
                return Err("Vector index path is not a bounded regular file".to_string());
            }
            if metadata.len() > 0 {
                index
                    .load(&path.to_string_lossy())
                    .map_err(|error| format!("Failed to load vector index: {error}"))?;
            } else {
                std::fs::remove_file(&path)
                    .map_err(|error| format!("Failed to remove empty vector index: {error}"))?;
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
        if vector.iter().any(|value| !value.is_finite()) {
            return Err("Vector contains a non-finite value".to_string());
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

    /// Search for nearest neighbors. Returns (key, distance) pairs sorted by distance ascending.
    pub fn search(&self, vector: &[f32], limit: usize) -> Result<Vec<(u64, f32)>, String> {
        if vector.len() != self.dimensions {
            return Err(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimensions,
                vector.len()
            ));
        }
        if limit == 0 || limit > 10_000 {
            return Err("Vector search limit is outside the supported range".to_string());
        }
        if vector.iter().any(|value| !value.is_finite()) {
            return Err("Search vector contains a non-finite value".to_string());
        }

        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());

        if index.size() == 0 {
            return Ok(Vec::new());
        }

        let result = index
            .search(vector, limit)
            .map_err(|e| format!("Search failed: {}", e))?;

        Ok(result
            .keys
            .iter()
            .copied()
            .zip(result.distances.iter().copied())
            .collect())
    }

    pub fn save(&self) -> Result<(), String> {
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if index.size() > 0 {
            index
                .save(&self.path.to_string_lossy())
                .map_err(|e| format!("Failed to save index: {}", e))?;
            let metadata = std::fs::symlink_metadata(&self.path)
                .map_err(|error| format!("Failed to inspect saved vector index: {error}"))?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() > MAX_INDEX_FILE_BYTES
            {
                return Err("Saved vector index is not a bounded regular file".to_string());
            }
            let file = std::fs::File::open(&self.path)
                .map_err(|error| format!("Failed to open saved vector index: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("Failed to sync vector index: {error}"))
        } else {
            // Don't save empty indices — just remove any stale file
            match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Failed to remove empty vector index: {error}")),
            }
            Ok(())
        }
    }

    pub fn count(&self) -> Result<usize, String> {
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        Ok(index.size())
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
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
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Failed to remove vector index: {error}")),
        }
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
    /// Current embedding dimension — may change at runtime when a new
    /// embedding model is loaded.  Protected by a Mutex so Tauri's shared
    /// State<VectorStoreManager> (which requires Sync) stays sound.
    dimensions: Arc<Mutex<usize>>,
    /// Cached stores keyed by scope.
    stores: Arc<Mutex<HashMap<VectorScope, Arc<VectorStore>>>>,
    /// Serializes profile changes, ingestion publishes, and index rebuilds.
    update_lock: Arc<tokio::sync::Mutex<()>>,
}

impl VectorStoreManager {
    pub fn new(base_dir: PathBuf, dimensions: usize) -> Result<Self, String> {
        if dimensions == 0 || dimensions > MAX_VECTOR_DIMENSIONS {
            return Err("Vector dimensions are outside the supported range".to_string());
        }
        std::fs::create_dir_all(&base_dir)
            .map_err(|e| format!("Failed to create vectors dir: {}", e))?;
        let metadata = std::fs::symlink_metadata(&base_dir)
            .map_err(|error| format!("Failed to inspect vectors dir: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Vectors path must be a real directory".to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&base_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("Failed to secure vectors dir: {error}"))?;
        }
        let base_dir = base_dir
            .canonicalize()
            .map_err(|error| format!("Failed to resolve vectors dir: {error}"))?;

        Ok(Self {
            base_dir,
            dimensions: Arc::new(Mutex::new(dimensions)),
            stores: Arc::new(Mutex::new(HashMap::new())),
            update_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub async fn lock_updates(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.update_lock.clone().lock_owned().await
    }

    /// Get (or lazily create) the `VectorStore` for a given scope.
    pub fn get(&self, scope: &VectorScope) -> Result<Arc<VectorStore>, String> {
        let dims = *self.dimensions.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.stores.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(store) = map.get(scope) {
            return Ok(store.clone());
        }

        let path = self.base_dir.join(scope.filename(dims)?);
        let store = Arc::new(VectorStore::new(path, dims)?);
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

    /// Delete all .usearch files in the base directory whose filename
    /// contains `old_dim` (e.g. `chat_abc_384.usearch`).  Called when
    /// the embedding model changes and produces a different hidden_size.
    pub fn purge_by_dimension(&self, old_dim: usize) -> Result<(), String> {
        let suffix = format!("_{old_dim}.usearch");
        let entries = std::fs::read_dir(&self.base_dir)
            .map_err(|error| format!("Failed to list vector indices: {error}"))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Failed to inspect vector index: {error}"))?;
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("Failed to inspect vector index: {error}"))?;
            if metadata.is_file() && !metadata.file_type().is_symlink() && name.ends_with(&suffix) {
                std::fs::remove_file(&path)
                    .map_err(|error| format!("Failed to remove stale vector index: {error}"))?;
            }
        }
        self.stores
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        Ok(())
    }

    /// Reinitialize the manager with a new embedding dimension.
    /// Drops all cached in-memory stores; they will be lazily recreated
    /// from the new (empty) index files on the next `get()` call.
    pub fn reinit(&self, new_dim: usize) -> Result<(), String> {
        if new_dim == 0 || new_dim > MAX_VECTOR_DIMENSIONS {
            return Err("Vector dimensions are outside the supported range".to_string());
        }
        // Keep lock ordering consistent with `get`/`delete_scope`.
        let mut dimensions = self.dimensions.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        *dimensions = new_dim;
        map.clear();

        Ok(())
    }

    /// Save all dirty indices to disk.
    pub fn save_all(&self) -> Result<(), String> {
        let map = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        for (scope, store) in map.iter() {
            store
                .save()
                .map_err(|error| format!("Failed to save vector scope {scope:?}: {error}"))?;
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
        let entries = std::fs::read_dir(&self.base_dir)
            .map_err(|error| format!("Failed to list vector indices: {error}"))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Failed to inspect vector index: {error}"))?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("Failed to inspect vector index: {error}"))?;
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && path.extension().and_then(|e| e.to_str()) == Some("usearch")
            {
                std::fs::remove_file(&path)
                    .map_err(|error| format!("Failed to remove vector index: {error}"))?;
            }
        }

        Ok(())
    }

    /// Delete the index for a specific scope (e.g. when a project is deleted).
    pub fn delete_scope(&self, scope: &VectorScope) -> Result<(), String> {
        let dims = *self.dimensions.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.stores.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(store) = map.remove(scope) {
            let _ = store.reset();
        }

        let path = self.base_dir.join(scope.filename(dims)?);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Failed to remove vector scope: {error}")),
        }
        Ok(())
    }

    /// Build a complete replacement index off to the side, publish it, then
    /// atomically swap the cached store. The database remains the source of
    /// truth, so partial in-memory additions are never externally published.
    pub fn replace_scope(
        &self,
        scope: &VectorScope,
        entries: &[(u64, Vec<f32>)],
    ) -> Result<(), String> {
        let dimensions = self.dimensions();
        let filename = scope.filename(dimensions)?;
        let final_path = self.base_dir.join(&filename);

        if entries.is_empty() {
            match std::fs::remove_file(&final_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Failed to clear vector scope: {error}")),
            }
            let store = Arc::new(VectorStore::new(final_path, dimensions)?);
            self.stores
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(scope.clone(), store);
            return Ok(());
        }

        let staging_name = format!(".{filename}.{}.staging", uuid::Uuid::new_v4());
        let staging_path = self.base_dir.join(staging_name);
        let staging_store = VectorStore::new(staging_path.clone(), dimensions)?;
        for (id, vector) in entries {
            staging_store.add(*id, vector)?;
        }
        if let Err(error) = staging_store.save() {
            let _ = std::fs::remove_file(&staging_path);
            return Err(error);
        }
        drop(staging_store);

        #[cfg(windows)]
        if final_path.exists() {
            std::fs::remove_file(&final_path)
                .map_err(|error| format!("Failed to replace vector scope: {error}"))?;
        }
        std::fs::rename(&staging_path, &final_path)
            .map_err(|error| format!("Failed to publish vector scope: {error}"))?;
        if let Ok(directory) = std::fs::File::open(&self.base_dir) {
            let _ = directory.sync_all();
        }

        let store = Arc::new(VectorStore::new(final_path, dimensions)?);
        self.stores
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(scope.clone(), store);
        Ok(())
    }

    /// Search across multiple scopes and merge results.
    /// Returns rowids sorted by cosine similarity (most similar first).
    pub fn search_scoped(
        &self,
        vector: &[f32],
        scopes: &[VectorScope],
        limit: usize,
    ) -> Result<Vec<u64>, String> {
        let mut all_results: Vec<(u64, f32)> = Vec::new();

        for scope in scopes {
            let store = self.get(scope)?;
            all_results.extend(store.search(vector, limit)?);
        }

        // Sort by distance ascending (lower = more similar for cosine metric)
        all_results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // Deduplicate by key, keeping the entry with the lowest distance
        let mut seen = std::collections::HashSet::new();
        all_results.retain(|(key, _)| seen.insert(*key));

        Ok(all_results
            .into_iter()
            .take(limit)
            .map(|(key, _)| key)
            .collect())
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
        *self.dimensions.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_filenames_never_embed_untrusted_ids() {
        let scope = VectorScope::Project("../../outside/secrets".to_string());
        let filename = scope.filename(4).unwrap();
        assert!(filename.starts_with("project_"));
        assert!(filename.ends_with("_4.usearch"));
        assert!(!filename.contains('/') && !filename.contains(".."));
        assert!(VectorScope::Chat(String::new()).filename(4).is_err());
    }

    #[test]
    fn rejects_invalid_vectors_and_dimensions() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(VectorStoreManager::new(temporary.path().join("zero"), 0).is_err());
        let manager = VectorStoreManager::new(temporary.path().join("vectors"), 2).unwrap();
        let store = manager.get(&VectorScope::Global).unwrap();
        assert!(store.add(1, &[1.0]).is_err());
        assert!(store.add(1, &[f32::NAN, 1.0]).is_err());
        assert!(store.search(&[1.0, 2.0], 0).is_err());
    }

    #[test]
    fn replacement_publishes_only_complete_scope() {
        let temporary = tempfile::tempdir().unwrap();
        let manager = VectorStoreManager::new(temporary.path().join("vectors"), 2).unwrap();
        manager
            .replace_scope(
                &VectorScope::Global,
                &[(10, vec![1.0, 0.0]), (20, vec![0.0, 1.0])],
            )
            .unwrap();
        let results = manager
            .search_scoped(&[1.0, 0.0], &[VectorScope::Global], 2)
            .unwrap();
        assert_eq!(results.first(), Some(&10));

        manager
            .replace_scope(&VectorScope::Global, &[(30, vec![1.0, 0.0])])
            .unwrap();
        let results = manager
            .search_scoped(&[1.0, 0.0], &[VectorScope::Global], 2)
            .unwrap();
        assert_eq!(results, vec![30]);
    }
}
