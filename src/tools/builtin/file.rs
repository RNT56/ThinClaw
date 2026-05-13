//! Compatibility adapter for extracted filesystem tools.

use std::path::Path;

use async_trait::async_trait;

use crate::agent::checkpoint;
use crate::context::JobContext;

pub use thinclaw_tools::builtin::file::{
    ApplyPatchTool, FileToolHost, GrepTool, ListDirTool, ReadFileTool, WriteFileTool,
};

pub struct RootFileToolHost;

#[async_trait]
impl FileToolHost for RootFileToolHost {
    async fn checkpoint_before_mutation(
        &self,
        ctx: &JobContext,
        path: &Path,
        base_dir: Option<&Path>,
        reason: &str,
    ) -> Result<(), String> {
        match checkpoint::ensure_checkpoint(ctx, path, base_dir, reason).await {
            Ok(true) | Ok(false) => Ok(()),
            Err(checkpoint::CheckpointError::Disabled) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn acp_read_text_file(
        &self,
        session_id: &str,
        path: &str,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Option<String>, String> {
        #[cfg(feature = "acp")]
        {
            crate::channels::acp::client_read_text_file(session_id, path, offset, limit)
                .await
                .map_err(|error| error.to_string())
        }
        #[cfg(not(feature = "acp"))]
        {
            let _ = (session_id, path, offset, limit);
            Ok(None)
        }
    }

    async fn acp_write_text_file(
        &self,
        session_id: &str,
        path: &str,
        content: &str,
    ) -> Result<Option<()>, String> {
        #[cfg(feature = "acp")]
        {
            crate::channels::acp::client_write_text_file(session_id, path, content)
                .await
                .map_err(|error| error.to_string())
        }
        #[cfg(not(feature = "acp"))]
        {
            let _ = (session_id, path, content);
            Ok(None)
        }
    }
}
