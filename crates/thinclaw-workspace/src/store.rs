//! Workspace persistence trait.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_types::error::WorkspaceError;
use uuid::Uuid;

use crate::{MemoryChunk, MemoryDocument, SearchConfig, SearchResult, WorkspaceEntry};

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError>;
    /// Atomically replace a document only when its content still matches the
    /// caller's snapshot. Production stores override this with a single
    /// compare-and-swap statement; the default preserves compatibility for
    /// test/dummy stores that cannot provide stronger semantics.
    async fn update_document_if_current(
        &self,
        id: Uuid,
        expected_content: &str,
        content: &str,
    ) -> Result<bool, WorkspaceError> {
        let current = self.get_document_by_id(id).await?;
        if current.content != expected_content {
            return Ok(false);
        }
        self.update_document(id, content).await?;
        Ok(true)
    }
    /// Atomically append to a document, creating it when absent.
    ///
    /// Implementations must serialize concurrent writers at the database
    /// layer; a read/modify/write default would lose memory entries.
    async fn append_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
        separator: &str,
        content: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError>;
    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError>;
    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError>;
    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError>;
    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError>;
    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError>;

    /// Atomically replace all chunks for a document.
    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<(), WorkspaceError> {
        self.delete_chunks(document_id).await?;
        for (index, content, embedding) in chunks {
            self.insert_chunk(document_id, *index, content, embedding.as_deref())
                .await?;
        }
        Ok(())
    }

    /// Replace an index only when the document still has the content that was
    /// chunked. Returns `false` when a concurrent writer changed it.
    async fn replace_chunks_if_current(
        &self,
        document_id: Uuid,
        expected_content: &str,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<bool, WorkspaceError>;

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;
    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError>;
    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError>;
}

#[async_trait]
impl<T> WorkspaceStore for Arc<T>
where
    T: WorkspaceStore + ?Sized,
{
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        (**self).get_document_by_path(user_id, agent_id, path).await
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        (**self).get_document_by_id(id).await
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        (**self)
            .get_or_create_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        (**self).update_document(id, content).await
    }

    async fn update_document_if_current(
        &self,
        id: Uuid,
        expected_content: &str,
        content: &str,
    ) -> Result<bool, WorkspaceError> {
        (**self)
            .update_document_if_current(id, expected_content, content)
            .await
    }

    async fn append_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
        separator: &str,
        content: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        (**self)
            .append_document_by_path(user_id, agent_id, path, separator, content)
            .await
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        (**self)
            .delete_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        (**self).list_directory(user_id, agent_id, directory).await
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        (**self).list_all_paths(user_id, agent_id).await
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        (**self).list_documents(user_id, agent_id).await
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        (**self).delete_chunks(document_id).await
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        (**self)
            .insert_chunk(document_id, chunk_index, content, embedding)
            .await
    }

    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<(), WorkspaceError> {
        (**self).replace_chunks(document_id, chunks).await
    }

    async fn replace_chunks_if_current(
        &self,
        document_id: Uuid,
        expected_content: &str,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<bool, WorkspaceError> {
        (**self)
            .replace_chunks_if_current(document_id, expected_content, chunks)
            .await
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        (**self).update_chunk_embedding(chunk_id, embedding).await
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        (**self)
            .get_chunks_without_embeddings(user_id, agent_id, limit)
            .await
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        (**self)
            .hybrid_search(user_id, agent_id, query, embedding, config)
            .await
    }
}
