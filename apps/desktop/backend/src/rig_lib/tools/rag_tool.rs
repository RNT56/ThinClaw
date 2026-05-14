use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use serde_json::json;
use tauri::Manager;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RAGError {
    #[error("RAG Retrieval failed: {0}")]
    Retrieval(String),
}

#[derive(Deserialize)]
pub struct RAGArgs {
    pub query: String,
}

pub struct RAGTool {
    pub app: tauri::AppHandle,
}

impl Tool for RAGTool {
    const NAME: &'static str = "knowledge_search";

    type Error = RAGError;
    type Args = RAGArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "knowledge_search".to_string(),
            description: "Search the user's uploaded documents and knowledge base. Use this when the user asks questions about their own files, specific documents, or 'context'.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query to find relevant information in the documents"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let sidecar = self
            .app
            .state::<crate::sidecar::SidecarManager>()
            .inner()
            .clone();
        let pool = self.app.state::<sqlx::SqlitePool>().inner().clone();
        let vector_manager = self
            .app
            .state::<crate::vector_store::VectorStoreManager>()
            .inner()
            .clone();
        let reranker = self
            .app
            .state::<crate::reranker::RerankerWrapper>()
            .inner()
            .clone();
        let app_handle = self.app.clone();
        let query = args.query.clone();

        let handle = tokio::spawn(async move {
            // Notify UI
            use tauri::Emitter;
            #[derive(serde::Serialize, Clone, specta::Type)]
            struct WebSearchStatus {
                id: Option<String>,
                step: String,
                message: String,
            }
            let _ = app_handle.emit(
                "web_search_status",
                WebSearchStatus {
                    id: None, // RAGTool doesn't have conv_id easily available here yet, will fix if needed
                    step: "rag_searching".into(),
                    message: format!("Searching knowledge base for: {}", query),
                },
            );

            let emb_backend = {
                let router = app_handle.state::<crate::inference::router::InferenceRouter>();
                router.embedding_backend().await
            };

            crate::rag::retrieve_context_internal(
                Some(app_handle.clone()), // Pass app_handle to internal
                &sidecar,
                pool,
                vector_manager,
                &reranker,
                emb_backend,
                query,
                None, // chat_id
                None, // doc_ids
                None, // project_id
            )
            .await
        });

        match handle
            .await
            .map_err(|e| RAGError::Retrieval(e.to_string()))?
        {
            Ok(results) => {
                if results.is_empty() {
                    return Ok("No relevant information found in the knowledge base.".to_string());
                }
                let context = results.join("\n\n---\n\n");
                Ok(format!("**Found in Knowledge Base:**\n{}", context))
            }
            Err(e) => Err(RAGError::Retrieval(e)),
        }
    }
}
