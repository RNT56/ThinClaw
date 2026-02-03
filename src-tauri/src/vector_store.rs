use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

#[derive(Clone)]
pub struct VectorStore {
    index: Arc<Mutex<Index>>,
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
                // Try loading. If it fails, the binary format might be incompatible.
                if let Err(e) = index.load(&path.to_string_lossy()) {
                    eprintln!(
                        "[vector_store] Load failed (likely incompatible format): {}. Backing up and Resetting.",
                        e
                    );
                    // Backup instead of delete
                    let backup_path = path.with_extension("usearch.bak");
                    let _ = std::fs::rename(&path, &backup_path);

                    // MUST recreate the index object because load failure can leave C++ state corrupted
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
            index: Arc::new(Mutex::new(index)),
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

        let index = self.index.lock().map_err(|_| "Failed to lock index")?;

        // Reserve capacity if needed - usearch requires this before adding
        let current_capacity = index.capacity();
        let current_size = index.size();
        if current_size >= current_capacity {
            // Reserve more capacity (grow by 1000 or double, whichever is larger)
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

        let index = self.index.lock().map_err(|_| "Failed to lock index")?;
        let result = index
            .search(vector, limit)
            .map_err(|e| format!("Search failed: {}", e))?;

        Ok(result.keys.to_vec())
    }

    pub async fn save(&self) -> Result<(), String> {
        let index_arc = self.index.clone();
        let path = self.path.clone();

        tokio::task::spawn_blocking(move || {
            let index = index_arc.lock().map_err(|_| "Failed to lock index")?;
            index
                .save(&path.to_string_lossy())
                .map_err(|e| format!("Failed to save index: {}", e))
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    pub fn count(&self) -> Result<usize, String> {
        let index = self.index.lock().map_err(|_| "Failed to lock index")?;
        Ok(index.size())
    }

    pub fn reset(&self) -> Result<(), String> {
        let mut index = self.index.lock().map_err(|_| "Failed to lock index")?;

        let options = IndexOptions {
            dimensions: self.dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 16,
            expansion_add: 128,
            expansion_search: 64,
            multi: false,
        };

        // Re-create the index object (in-memory)
        *index = Index::new(&options).map_err(|e| format!("Failed to create new index: {}", e))?;

        // Save (overwrite) to disk immediately to clear it
        index
            .save(&self.path.to_string_lossy())
            .map_err(|e| format!("Failed to save cleared index: {}", e))?;

        Ok(())
    }
}
