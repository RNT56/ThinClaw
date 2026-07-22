//! File operation tools for reading, writing, and navigating the filesystem.
//!
//! These tools provide controlled access to the filesystem with:
//! - Path validation and sandboxing
//! - Size limits on read/write operations
//! - Support for common development tasks

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::fs;

use thinclaw_tools_core::ToolRateLimitConfig;
use thinclaw_tools_core::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};
use thinclaw_types::JobContext;
use thinclaw_workspace::paths as ws_paths;

#[async_trait]
pub trait FileToolHost: Send + Sync {
    async fn checkpoint_before_mutation(
        &self,
        ctx: &JobContext,
        path: &Path,
        base_dir: Option<&Path>,
        reason: &str,
    ) -> Result<(), String>;

    async fn acp_read_text_file(
        &self,
        _session_id: &str,
        _path: &str,
        _offset: Option<u64>,
        _limit: Option<u64>,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }

    async fn acp_write_text_file(
        &self,
        _session_id: &str,
        _path: &str,
        _content: &str,
    ) -> Result<Option<()>, String> {
        Ok(None)
    }
}

/// Well-known workspace filenames that must go through memory_write or
/// prompt_manage, not write_file.
///
/// If the LLM tries to write one of these via the filesystem tool we reject
/// immediately and point it at the correct tool.
const WORKSPACE_FILES: &[&str] = &[
    ws_paths::HEARTBEAT,
    ws_paths::MEMORY,
    ws_paths::IDENTITY,
    ws_paths::SOUL,
    ws_paths::SOUL_LOCAL,
    ws_paths::AGENTS,
    ws_paths::USER,
    ws_paths::README,
];

/// Check whether `path` resolves to a workspace file that should be written
/// through workspace memory tools instead of `write_file`.
///
/// Only blocks files that live directly in the workspace root. Files inside
/// project subdirectories (e.g. `clawi-site/README.md`) are safe to write
/// via the filesystem tool.
fn is_workspace_path(path: &Path, base_dir: Option<&Path>) -> bool {
    let root = base_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let root = resolve_with_canonical_ancestor(&root);
    let path = resolve_with_canonical_ancestor(path);
    let Ok(relative) = path.strip_prefix(&root) else {
        return false;
    };
    let components = relative.components().collect::<Vec<_>>();
    if components.len() == 1 {
        return relative
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| WORKSPACE_FILES.contains(&name));
    }

    relative.starts_with("daily") || relative.starts_with("context")
}

/// Maximum file size for reading (1MB).
const MAX_READ_SIZE: u64 = 1024 * 1024;

/// Maximum file size for writing (5MB).
const MAX_WRITE_SIZE: usize = 5 * 1024 * 1024;

/// Maximum directory listing entries.
const MAX_DIR_ENTRIES: usize = 500;

/// Normalize a path by resolving `.` and `..` components lexically (no filesystem access).
///
/// This is critical for security: `std::fs::canonicalize` only works on paths that exist,
/// so for new files we must normalize without touching the filesystem.
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Only pop if there's a normal component to pop (don't escape root/prefix)
                if components
                    .last()
                    .is_some_and(|c| matches!(c, std::path::Component::Normal(_)))
                {
                    components.pop();
                }
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
}

/// Resolve symlinks and platform aliases through the nearest existing
/// ancestor, while preserving a not-yet-created filename tail. On macOS, for
/// example, temporary paths may be spelled with `/var` while their canonical
/// parent is `/private/var`; canonicalizing only the existing root would make a
/// new child appear to be outside that root.
fn resolve_with_canonical_ancestor(path: &Path) -> PathBuf {
    let normalized = normalize_lexical(path);
    if let Ok(canonical) = normalized.canonicalize() {
        return canonical;
    }

    let mut ancestor = normalized.as_path();
    let mut tail = Vec::<std::ffi::OsString>::new();
    loop {
        if ancestor.exists() {
            let mut resolved = ancestor
                .canonicalize()
                .unwrap_or_else(|_| ancestor.to_path_buf());
            for component in tail.into_iter().rev() {
                resolved.push(component);
            }
            return normalize_lexical(&resolved);
        }
        if let Some(name) = ancestor.file_name() {
            tail.push(name.to_os_string());
        }
        match ancestor.parent() {
            Some(parent) if parent != ancestor => ancestor = parent,
            _ => return normalized,
        }
    }
}

/// Validate that a path is safe (no traversal attacks).
///
/// When `base_dir` is `Some`, containment is fail-closed: the resolved path must
/// live under that canonical base, and `..` traversal or symlink escapes are
/// rejected even through non-existent parent directories (we normalize the
/// joined path lexically before checking, so an escape cannot slip through where
/// `canonicalize()` would fall back to the raw path).
///
/// When `base_dir` is `None` the tool operates in unrestricted mode — the
/// deliberate trusted-local-operator contract used when no workspace base is
/// configured (see [`crate::registry::ToolRegistry::register_filesystem_tools`],
/// which warns at registration time when no base is set). Relative paths resolve
/// against the current working directory and absolute paths are accepted as-is;
/// untrusted contexts must register the filesystem tools WITH a base directory to
/// sandbox access.
pub fn validate_path(path_str: &str, base_dir: Option<&Path>) -> Result<PathBuf, ToolError> {
    let path = PathBuf::from(path_str);

    // Base against which relative paths are resolved. With an explicit
    // `base_dir` this is also the containment root; with no base we resolve
    // relative paths against the current working directory.
    let containment_base: PathBuf = match base_dir {
        Some(base) => base.to_path_buf(),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    // Resolve to an absolute path. Absolute inputs are taken as-is; relative
    // inputs are joined onto the base.
    let resolved = if path.is_absolute() {
        path.canonicalize()
            .unwrap_or_else(|_| normalize_lexical(&path))
    } else {
        let joined = containment_base.join(&path);
        joined
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(&joined))
    };

    // Containment is enforced only when an explicit base directory is
    // configured. With no base, the tool runs in unrestricted mode (the
    // trusted-operator contract); registration emits a warning so the
    // unsandboxed state is observable.
    if base_dir.is_some() {
        let base_canonical = resolve_with_canonical_ancestor(&containment_base);

        // For existing paths, canonicalize to resolve symlinks.
        // For non-existent paths, the lexical normalization above already
        // removed all `..` components, so starts_with is reliable.
        let check_path = resolve_with_canonical_ancestor(&resolved);

        if !check_path.starts_with(&base_canonical) {
            return Err(ToolError::NotAuthorized(format!(
                "Path escapes sandbox: {}",
                path_str
            )));
        }
    }

    Ok(resolved)
}

async fn checkpoint_before_mutation(
    host: Option<&Arc<dyn FileToolHost>>,
    ctx: &JobContext,
    path: &Path,
    base_dir: Option<&Path>,
    reason: &str,
) -> Result<(), ToolError> {
    if let Some(host) = host {
        host.checkpoint_before_mutation(ctx, path, base_dir, reason)
            .await
            .map_err(|err| {
                ToolError::ExecutionFailed(format!(
                    "Failed to create filesystem checkpoint: {}",
                    err
                ))
            })?;
    }
    Ok(())
}

fn metadata_base_dir(ctx: &JobContext) -> Option<PathBuf> {
    ctx.metadata
        .get("tool_base_dir")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
}

pub fn effective_base_dir(
    ctx: &JobContext,
    configured: Option<&Path>,
) -> Result<Option<PathBuf>, ToolError> {
    let contextual = metadata_base_dir(ctx);
    match (contextual, configured) {
        // A per-job base may narrow a configured sandbox, but must never
        // replace it with an unrelated directory. Context metadata can
        // originate at protocol boundaries (for example ACP).
        (Some(contextual), Some(configured)) => {
            let contextual = contextual.to_str().ok_or_else(|| {
                ToolError::NotAuthorized("Contextual tool base is not valid UTF-8".to_string())
            })?;
            validate_path(contextual, Some(configured)).map(Some)
        }
        (Some(contextual), None) => Ok(Some(contextual)),
        (None, Some(configured)) => Ok(Some(configured.to_path_buf())),
        (None, None) => Ok(None),
    }
}

fn read_file_result(
    path: &Path,
    content: &str,
    offset: usize,
    limit: Option<u64>,
) -> serde_json::Value {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let start_line = if offset > 0 {
        offset.saturating_sub(1).min(total_lines)
    } else {
        0
    };
    let end_line = if let Some(lim) = limit {
        (start_line + lim as usize).min(total_lines)
    } else {
        total_lines
    };

    let selected_lines: Vec<String> = lines[start_line..end_line]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>6}│ {}", start_line + i + 1, line))
        .collect();

    serde_json::json!({
        "content": selected_lines.join("\n"),
        "total_lines": total_lines,
        "lines_shown": end_line - start_line,
        "path": path.display().to_string()
    })
}

fn acp_session_id(ctx: &JobContext) -> Option<&str> {
    ctx.metadata
        .get("acp_session_id")
        .and_then(serde_json::Value::as_str)
}

/// Read file contents tool.
#[derive(Default)]
pub struct ReadFileTool {
    base_dir: Option<PathBuf>,
    host: Option<Arc<dyn FileToolHost>>,
}

impl ReadFileTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_host(mut self, host: Arc<dyn FileToolHost>) -> Self {
        self.host = Some(host);
        self
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file from the LOCAL FILESYSTEM. NOT for workspace memory paths \
         (use memory_read for those). Returns file content as text. \
         For large files, you can specify offset and limit to read a portion."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-indexed, optional)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (optional)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = require_str(&params, "path")?;

        let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = params.get("limit").and_then(|v| v.as_u64());

        let start = std::time::Instant::now();

        let base_dir = effective_base_dir(ctx, self.base_dir.as_deref())?;
        let path = validate_path(path_str, base_dir.as_deref())?;

        if is_workspace_path(&path, base_dir.as_deref()) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is a workspace memory file. Use memory_read or prompt_manage instead of read_file.",
                path_str
            )));
        }

        if let (Some(host), Some(session_id)) = (self.host.as_ref(), acp_session_id(ctx)) {
            match host
                .acp_read_text_file(
                    session_id,
                    &path.display().to_string(),
                    (offset > 0).then_some(offset as u64),
                    limit,
                )
                .await
            {
                Ok(Some(content)) => {
                    return Ok(ToolOutput::success(
                        read_file_result(&path, &content, offset, limit),
                        start.elapsed(),
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(ToolError::ExternalService(format!(
                        "ACP fs/read_text_file failed: {error}"
                    )));
                }
            }
        }

        // Check file size
        let metadata = fs::symlink_metadata(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Cannot access file: {}", e)))?;

        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ToolError::ExecutionFailed(
                "Path is not a regular file".to_string(),
            ));
        }

        if metadata.len() > MAX_READ_SIZE {
            return Err(ToolError::ExecutionFailed(format!(
                "File too large ({} bytes). Maximum is {} bytes. Use offset/limit for partial reads.",
                metadata.len(),
                MAX_READ_SIZE
            )));
        }

        // Read file
        let content = thinclaw_platform::read_regular_file_bounded_single_link_async(
            path.clone(),
            MAX_READ_SIZE,
        )
        .await
        .and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read file: {}", e)))?;

        Ok(ToolOutput::success(
            read_file_result(&path, &content, offset, limit),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        true // File content could contain anything
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }
}

/// Write file contents tool.
#[derive(Default)]
pub struct WriteFileTool {
    base_dir: Option<PathBuf>,
    host: Option<Arc<dyn FileToolHost>>,
}

impl WriteFileTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_host(mut self, host: Arc<dyn FileToolHost>) -> Self {
        self.host = Some(host);
        self
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file on the LOCAL FILESYSTEM. NOT for workspace memory \
         (use memory_write or prompt_manage for that). Creates the file if it doesn't exist, overwrites if it does. \
         Parent directories are created automatically. Use apply_patch for targeted edits."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = require_str(&params, "path")?;

        let content = require_str(&params, "content")?;

        let start = std::time::Instant::now();

        // Check content size
        if content.len() > MAX_WRITE_SIZE {
            return Err(ToolError::InvalidParameters(format!(
                "Content too large ({} bytes). Maximum is {} bytes.",
                content.len(),
                MAX_WRITE_SIZE
            )));
        }

        let base_dir = effective_base_dir(ctx, self.base_dir.as_deref())?;
        let path = validate_path(path_str, base_dir.as_deref())?;

        // Reject workspace paths: these live in the database, not on disk.
        if is_workspace_path(&path, base_dir.as_deref()) {
            let normalized = std::path::Path::new(path_str)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path_str);
            let guidance = if [
                ws_paths::SOUL,
                ws_paths::SOUL_LOCAL,
                ws_paths::AGENTS,
                ws_paths::USER,
            ]
            .iter()
            .any(|candidate| normalized.eq_ignore_ascii_case(candidate))
            {
                "Use the prompt_manage tool instead of write_file."
            } else {
                "Use the memory_write tool instead of write_file. For HEARTBEAT.md use target='heartbeat', for MEMORY.md use target='memory'."
            };
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is a workspace memory file. {}",
                path_str, guidance
            )));
        }

        if let (Some(host), Some(session_id)) = (self.host.as_ref(), acp_session_id(ctx)) {
            match host
                .acp_write_text_file(session_id, &path.display().to_string(), content)
                .await
            {
                Ok(Some(())) => {
                    let result = serde_json::json!({
                        "path": path.display().to_string(),
                        "bytes_written": content.len(),
                        "success": true,
                        "backend": "acp_client_fs"
                    });
                    return Ok(ToolOutput::success(result, start.elapsed()));
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(ToolError::ExternalService(format!(
                        "ACP fs/write_text_file failed: {error}"
                    )));
                }
            }
        }

        checkpoint_before_mutation(
            self.host.as_ref(),
            ctx,
            &path,
            base_dir.as_deref(),
            "pre: write_file",
        )
        .await?;

        // Create parent directories
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to create directories: {}", e))
            })?;
        }

        // Write file
        thinclaw_platform::write_regular_file_atomic_async(
            path.clone(),
            content.as_bytes().to_vec(),
            true,
        )
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write file: {}", e)))?;

        let result = serde_json::json!({
            "path": path.display().to_string(),
            "bytes_written": content.len(),
            "success": true
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false // We're writing, not reading external data
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// List directory contents tool.
#[derive(Debug, Default)]
pub struct ListDirTool {
    base_dir: Option<PathBuf>,
}

impl ListDirTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List contents of a directory on the LOCAL FILESYSTEM. NOT for workspace memory \
         (use memory_tree for that). Shows files and subdirectories with their sizes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list (defaults to current directory)"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "If true, list contents recursively (default false)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum depth for recursive listing (default 3)"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let recursive = params
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_depth = params
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;

        let start = std::time::Instant::now();

        let base_dir = effective_base_dir(ctx, self.base_dir.as_deref())?;
        let path = validate_path(path_str, base_dir.as_deref())?;

        let mut entries = Vec::new();
        list_dir_inner(&path, &path, recursive, max_depth, 0, &mut entries).await?;

        // Sort entries
        entries.sort_by(|a, b| {
            let a_is_dir = a.ends_with('/');
            let b_is_dir = b.ends_with('/');
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.cmp(b),
            }
        });

        let truncated = entries.len() > MAX_DIR_ENTRIES;
        if truncated {
            entries.truncate(MAX_DIR_ENTRIES);
        }

        let result = serde_json::json!({
            "path": path.display().to_string(),
            "entries": entries,
            "count": entries.len(),
            "truncated": truncated
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // Filenames are untrusted workspace content.
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }
}

/// Recursively list directory contents.
async fn list_dir_inner(
    base: &Path,
    path: &Path,
    recursive: bool,
    max_depth: usize,
    current_depth: usize,
    entries: &mut Vec<String>,
) -> Result<(), ToolError> {
    if entries.len() >= MAX_DIR_ENTRIES {
        return Ok(());
    }

    let mut dir = fs::read_dir(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read directory: {}", e)))?;

    while let Some(entry) = dir
        .next_entry()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read entry: {}", e)))?
    {
        if entries.len() >= MAX_DIR_ENTRIES {
            break;
        }

        let entry_path = entry.path();
        let relative = entry_path
            .strip_prefix(base)
            .unwrap_or(&entry_path)
            .to_string_lossy();

        let metadata = fs::symlink_metadata(&entry_path).await.ok();
        let is_symlink = metadata
            .as_ref()
            .is_some_and(|metadata| metadata.file_type().is_symlink());
        let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir()) && !is_symlink;

        let display = if is_symlink {
            format!("{} [symlink]", relative)
        } else if is_dir {
            format!("{}/", relative)
        } else {
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            format!("{} ({})", relative, format_size(size))
        };

        entries.push(display);

        if recursive && is_dir && current_depth < max_depth {
            // Skip common non-essential directories
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !matches!(
                name_str.as_ref(),
                "node_modules" | "target" | ".git" | "__pycache__" | "venv" | ".venv"
            ) {
                Box::pin(list_dir_inner(
                    base,
                    &entry_path,
                    recursive,
                    max_depth,
                    current_depth + 1,
                    entries,
                ))
                .await?;
            }
        }
    }

    Ok(())
}

/// Format file size in human-readable form.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Apply patch tool for targeted file edits.
#[derive(Default)]
pub struct ApplyPatchTool {
    base_dir: Option<PathBuf>,
    host: Option<Arc<dyn FileToolHost>>,
}

impl ApplyPatchTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_host(mut self, host: Arc<dyn FileToolHost>) -> Self {
        self.host = Some(host);
        self
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply targeted edits to a file using search/replace. Finds the exact 'old_string' \
         and replaces it with 'new_string'. Use for surgical code changes without rewriting entire files. \
         The old_string must match exactly (including whitespace and indentation)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace it with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace all occurrences (default false, replaces first only)"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = require_str(&params, "path")?;

        let old_string = require_str(&params, "old_string")?;

        if old_string.is_empty() {
            return Err(ToolError::InvalidParameters(
                "old_string must not be empty".to_string(),
            ));
        }

        let new_string = require_str(&params, "new_string")?;

        let replace_all = params
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let start = std::time::Instant::now();

        let base_dir = effective_base_dir(ctx, self.base_dir.as_deref())?;
        let path = validate_path(path_str, base_dir.as_deref())?;

        if is_workspace_path(&path, base_dir.as_deref()) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is a workspace memory file. Use prompt_manage or memory_write instead of apply_patch.",
                path_str
            )));
        }

        checkpoint_before_mutation(
            self.host.as_ref(),
            ctx,
            &path,
            base_dir.as_deref(),
            "pre: apply_patch",
        )
        .await?;

        // Read current content
        let content = thinclaw_platform::read_regular_file_bounded_single_link_async(
            path.clone(),
            MAX_READ_SIZE,
        )
        .await
        .and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read file: {}", e)))?;

        // Check if old_string exists
        if !content.contains(old_string) {
            return Err(ToolError::ExecutionFailed(format!(
                "Could not find the specified text in {}. Make sure old_string matches exactly.",
                path.display()
            )));
        }

        let replacements = if replace_all {
            content.matches(old_string).count()
        } else {
            1
        };
        let removed_bytes = old_string
            .len()
            .checked_mul(replacements)
            .ok_or_else(|| ToolError::InvalidParameters("replacement size overflow".to_string()))?;
        let added_bytes = new_string
            .len()
            .checked_mul(replacements)
            .ok_or_else(|| ToolError::InvalidParameters("replacement size overflow".to_string()))?;
        let projected_size = content
            .len()
            .checked_sub(removed_bytes)
            .and_then(|size| size.checked_add(added_bytes))
            .ok_or_else(|| ToolError::InvalidParameters("replacement size overflow".to_string()))?;
        if projected_size > MAX_WRITE_SIZE {
            return Err(ToolError::InvalidParameters(format!(
                "Patched content would be too large ({projected_size} bytes). Maximum is {MAX_WRITE_SIZE} bytes."
            )));
        }

        // Apply replacement
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        // Write back
        thinclaw_platform::write_regular_file_atomic_async(
            path.clone(),
            new_content.into_bytes(),
            true,
        )
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write file: {}", e)))?;

        let result = serde_json::json!({
            "path": path.display().to_string(),
            "replacements": replacements,
            "success": true
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false // We're writing, not reading external data
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// Search file contents using pattern matching (grep-like).
#[derive(Debug, Default)]
pub struct GrepTool {
    base_dir: Option<PathBuf>,
}

impl GrepTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }
}

/// Maximum number of matches to return.
const MAX_GREP_MATCHES: usize = 100;

/// Maximum file size to scan (5MB).
const MAX_GREP_FILE_SIZE: u64 = 5 * 1024 * 1024;

/// Maximum regex/literal query size accepted by grep.
const MAX_GREP_PATTERN_SIZE: usize = 16 * 1024;

/// Maximum number of characters returned from any one source line.
const MAX_GREP_LINE_CHARS: usize = 8 * 1024;

/// Maximum context radius per match.
const MAX_GREP_CONTEXT_LINES: usize = 20;

/// Maximum directory depth for recursive search.
const MAX_GREP_DEPTH: usize = 10;

/// Directories to skip when recursively searching.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "__pycache__",
    "venv",
    ".venv",
    ".next",
    "dist",
    "build",
    ".cargo",
    ".tox",
    "vendor",
];

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search for a pattern in file contents. Searches a file or recursively searches \
         a directory. Returns matching lines with file paths and line numbers. \
         Supports literal strings and regex patterns. Use include_pattern to filter by \
         file extension (e.g. '*.rs', '*.py')."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The search pattern (literal string or regex)"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search (defaults to current directory)"
                },
                "is_regex": {
                    "type": "boolean",
                    "description": "If true, treat pattern as a regex (default false, literal match)"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "If true, ignore case (default false)"
                },
                "include_pattern": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. '*.rs', '*.py', '*.tsx')"
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Number of context lines before and after each match (default 0)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let pattern_str = require_str(&params, "pattern")?;
        if pattern_str.len() > MAX_GREP_PATTERN_SIZE {
            return Err(ToolError::InvalidParameters(format!(
                "Search pattern is too large ({} bytes). Maximum is {} bytes.",
                pattern_str.len(),
                MAX_GREP_PATTERN_SIZE
            )));
        }

        let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let is_regex = params
            .get("is_regex")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let case_insensitive = params
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let include_pattern = params.get("include_pattern").and_then(|v| v.as_str());

        let context_lines = params
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(MAX_GREP_CONTEXT_LINES as u64) as usize;

        let start = std::time::Instant::now();

        let base_dir = effective_base_dir(ctx, self.base_dir.as_deref())?;
        let path = validate_path(path_str, base_dir.as_deref())?;

        // Build the matcher
        let pattern = if is_regex {
            pattern_str.to_string()
        } else {
            // Escape regex special characters for literal matching
            regex::escape(pattern_str)
        };

        let re = if case_insensitive {
            regex::Regex::new(&format!("(?i){}", pattern))
        } else {
            regex::Regex::new(&pattern)
        }
        .map_err(|e| ToolError::InvalidParameters(format!("Invalid pattern: {}", e)))?;

        // Collect files to search
        let mut files = Vec::new();
        if path.is_file() {
            files.push(path.clone());
        } else if path.is_dir() {
            collect_files(&path, &mut files, include_pattern, 0).await?;
        } else {
            return Err(ToolError::ExecutionFailed(format!(
                "Path does not exist: {}",
                path.display()
            )));
        }

        // Search each file
        let mut matches = Vec::new();
        let mut files_searched = 0_u32;
        let mut files_with_matches = 0_u32;

        for file_path in &files {
            if matches.len() >= MAX_GREP_MATCHES {
                break;
            }

            if is_workspace_path(file_path, base_dir.as_deref()) {
                continue;
            }

            // Skip files that are too large
            let metadata = match fs::symlink_metadata(&file_path).await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() > MAX_GREP_FILE_SIZE
            {
                continue;
            }

            // Read file content (skip binary files)
            let content = match thinclaw_platform::read_regular_file_bounded_single_link_async(
                file_path.clone(),
                MAX_GREP_FILE_SIZE,
            )
            .await
            .and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
            }) {
                Ok(content) => content,
                Err(_) => continue, // binary or unreadable
            };

            files_searched += 1;
            let lines: Vec<&str> = content.lines().collect();
            let mut file_had_match = false;

            for (line_idx, line) in lines.iter().enumerate() {
                if matches.len() >= MAX_GREP_MATCHES {
                    break;
                }

                if re.is_match(line) {
                    file_had_match = true;

                    // Gather context lines
                    let ctx_start = line_idx.saturating_sub(context_lines);
                    let ctx_end = (line_idx + context_lines + 1).min(lines.len());

                    let context: Vec<serde_json::Value> = if context_lines > 0 {
                        lines[ctx_start..ctx_end]
                            .iter()
                            .enumerate()
                            .map(|(i, l)| {
                                let actual_line = ctx_start + i + 1;
                                serde_json::json!({
                                    "line": actual_line,
                                    "content": bounded_grep_line(l),
                                    "is_match": actual_line == line_idx + 1,
                                })
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };

                    let relative = file_path
                        .strip_prefix(&path)
                        .unwrap_or(file_path)
                        .to_string_lossy()
                        .to_string();

                    let mut match_obj = serde_json::json!({
                        "file": relative,
                        "line": line_idx + 1,
                        "content": bounded_grep_line(line.trim()),
                    });

                    if !context.is_empty() {
                        match_obj["context"] = serde_json::json!(context);
                    }

                    matches.push(match_obj);
                }
            }

            if file_had_match {
                files_with_matches += 1;
            }
        }

        let truncated = matches.len() >= MAX_GREP_MATCHES;

        let result = serde_json::json!({
            "matches": matches,
            "total_matches": matches.len(),
            "files_searched": files_searched,
            "files_with_matches": files_with_matches,
            "truncated": truncated,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // File content could contain anything
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }
}

fn bounded_grep_line(value: &str) -> String {
    let mut chars = value.chars();
    let bounded = chars.by_ref().take(MAX_GREP_LINE_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{bounded}… [truncated]")
    } else {
        bounded
    }
}

/// Recursively collect files for grep, respecting include patterns and skip dirs.
async fn collect_files(
    dir: &Path,
    files: &mut Vec<PathBuf>,
    include_pattern: Option<&str>,
    depth: usize,
) -> Result<(), ToolError> {
    if depth > MAX_GREP_DEPTH || files.len() >= MAX_GREP_MATCHES * 10 {
        return Ok(());
    }

    let mut entries = fs::read_dir(dir)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read directory: {}", e)))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read entry: {}", e)))?
    {
        let entry_path = entry.path();
        let metadata = match fs::symlink_metadata(&entry_path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        if metadata.file_type().is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Skip known non-essential directories
            if SKIP_DIRS.contains(&name_str.as_ref()) || name_str.starts_with('.') {
                continue;
            }

            Box::pin(collect_files(
                &entry_path,
                files,
                include_pattern,
                depth + 1,
            ))
            .await?;
        } else if metadata.is_file() {
            // Apply include pattern filter
            if let Some(pattern) = include_pattern {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !glob_matches(pattern, &name_str) {
                    continue;
                }
            }

            files.push(entry_path);
        }
    }

    Ok(())
}

/// Simple glob matching (supports *.ext and *pattern* style).
fn glob_matches(pattern: &str, filename: &str) -> bool {
    if pattern.starts_with("*.") {
        // Extension match: *.rs, *.py, etc.
        let ext = &pattern[1..]; // ".rs"
        filename.ends_with(ext)
    } else if pattern.starts_with('*') && pattern.ends_with('*') {
        // Contains match: *test*
        let inner = &pattern[1..pattern.len() - 1];
        filename.contains(inner)
    } else if let Some(suffix) = pattern.strip_prefix('*') {
        // Suffix match: *_test.rs
        filename.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        // Prefix match: test_*
        filename.starts_with(prefix)
    } else {
        // Exact match
        filename == pattern
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_read_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line 1\nline 2\nline 3\n").unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let content = result.result.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("line 1"));
        assert!(content.contains("line 2"));
    }

    #[tokio::test]
    async fn contextual_base_cannot_replace_configured_sandbox() {
        let configured = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        let tool = ReadFileTool::new().with_base_dir(configured.path().to_path_buf());
        let ctx = JobContext {
            metadata: serde_json::json!({
                "tool_base_dir": outside.path().to_string_lossy(),
            }),
            ..JobContext::default()
        };

        let result = tool
            .execute(serde_json::json!({"path": "secret.txt"}), &ctx)
            .await;

        assert!(matches!(result, Err(ToolError::NotAuthorized(_))));
    }

    #[tokio::test]
    async fn test_write_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("new_file.txt");

        let tool = WriteFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "content": "hello world"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_apply_patch() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("code.rs");
        std::fs::write(&file_path, "fn main() {\n    println!(\"old\");\n}\n").unwrap();

        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "println!(\"old\")",
                    "new_string": "println!(\"new\")"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("println!(\"new\")"));
    }

    #[tokio::test]
    async fn test_write_file_rejects_workspace_paths() {
        let dir = TempDir::new().unwrap();
        let tool = WriteFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let workspace_files = &[
            "HEARTBEAT.md",
            "MEMORY.md",
            "IDENTITY.md",
            "SOUL.md",
            "SOUL.local.md",
            "AGENTS.md",
            "USER.md",
            "README.md",
        ];

        for filename in workspace_files {
            let err = tool
                .execute(
                    serde_json::json!({
                        "path": filename,
                        "content": "test"
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();

            let msg = err.to_string();
            let expects_prompt_manage = matches!(
                *filename,
                "SOUL.md" | "SOUL.local.md" | "AGENTS.md" | "USER.md"
            );
            assert!(
                if expects_prompt_manage {
                    msg.contains("prompt_manage")
                } else {
                    msg.contains("memory_write")
                },
                "Rejection for {} should mention the correct workspace tool, got: {}",
                filename,
                msg
            );
        }

        for disguised_path in [
            dir.path().join("SOUL.md"),
            dir.path().join("nested/../MEMORY.md"),
        ] {
            let err = tool
                .execute(
                    serde_json::json!({
                        "path": disguised_path.to_string_lossy(),
                        "content": "test"
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("prompt_manage")
                    || err.to_string().contains("memory_write")
            );
        }

        // daily/ and context/ prefixes should also be rejected
        for prefix_path in &["daily/2024-01-15.md", "context/vision.md"] {
            let err = tool
                .execute(
                    serde_json::json!({
                        "path": prefix_path,
                        "content": "test"
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();

            assert!(
                err.to_string().contains("memory_write"),
                "Rejection for {} should mention memory_write",
                prefix_path
            );
        }

        // Regular files should still work
        let regular_path = dir.path().join("normal.txt");
        let result = tool
            .execute(
                serde_json::json!({
                    "path": regular_path.to_str().unwrap(),
                    "content": "fine"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn read_and_patch_reject_absolute_workspace_memory_paths() {
        let dir = TempDir::new().unwrap();
        let soul_path = dir.path().join("SOUL.md");
        std::fs::write(&soul_path, "old").unwrap();
        let ctx = JobContext::default();

        let read_error = ReadFileTool::new()
            .with_base_dir(dir.path().to_path_buf())
            .execute(
                serde_json::json!({"path": soul_path.to_string_lossy()}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(read_error.to_string().contains("memory_read"));

        let patch_error = ApplyPatchTool::new()
            .with_base_dir(dir.path().to_path_buf())
            .execute(
                serde_json::json!({
                    "path": soul_path.to_string_lossy(),
                    "old_string": "old",
                    "new_string": "new"
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(patch_error.to_string().contains("prompt_manage"));
        assert_eq!(std::fs::read_to_string(soul_path).unwrap(), "old");
    }

    #[tokio::test]
    async fn apply_patch_rejects_empty_match() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("code.rs");
        std::fs::write(&path, "content").unwrap();

        let error = ApplyPatchTool::new()
            .with_base_dir(dir.path().to_path_buf())
            .execute(
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "old_string": "",
                    "new_string": "x",
                    "replace_all": true
                }),
                &JobContext::default(),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("must not be empty"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "content");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grep_does_not_follow_nested_symlinks_or_search_workspace_memory() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "needle-outside").unwrap();
        std::fs::write(dir.path().join("SOUL.md"), "needle-memory").unwrap();
        symlink(outside.path(), dir.path().join("linked")).unwrap();

        let result = GrepTool::new()
            .with_base_dir(dir.path().to_path_buf())
            .execute(
                serde_json::json!({"path": ".", "pattern": "needle"}),
                &JobContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(result.result["total_matches"], 0);
    }

    #[tokio::test]
    async fn test_list_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file1.txt"), "content").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let tool = ListDirTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": dir.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let entries = result.result.get("entries").unwrap().as_array().unwrap();
        assert!(entries.len() >= 2);
    }

    #[test]
    fn test_normalize_lexical() {
        // Basic .. resolution
        assert_eq!(
            normalize_lexical(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        // Multiple .. components
        assert_eq!(
            normalize_lexical(Path::new("/a/b/c/../../d")),
            PathBuf::from("/a/d")
        );
        // . components stripped
        assert_eq!(
            normalize_lexical(Path::new("/a/./b/./c")),
            PathBuf::from("/a/b/c")
        );
        // Cannot escape root
        assert_eq!(
            normalize_lexical(Path::new("/a/../../..")),
            PathBuf::from("/")
        );
    }

    #[test]
    fn test_validate_path_rejects_traversal_nonexistent_parent() {
        // The critical test: writing to ../../outside/newdir/file with base_dir
        // set should be rejected even when the parent directory does not exist
        // (i.e. canonicalize() cannot resolve it).
        let dir = TempDir::new().unwrap();
        let evil_path = format!(
            "{}/../../outside/newdir/file.txt",
            dir.path().to_str().unwrap()
        );
        let result = validate_path(&evil_path, Some(dir.path()));
        assert!(
            result.is_err(),
            "Should reject traversal via non-existent parent, got: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_rejects_relative_traversal() {
        let dir = TempDir::new().unwrap();
        let result = validate_path("../../etc/passwd", Some(dir.path()));
        assert!(
            result.is_err(),
            "Should reject relative traversal, got: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_allows_valid_nested_write() {
        let dir = TempDir::new().unwrap();
        let result = validate_path("subdir/newfile.txt", Some(dir.path()));
        assert!(
            result.is_ok(),
            "Should allow nested writes within sandbox: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_allows_dot_dot_within_sandbox() {
        // a/b/../c resolves to a/c which is still inside the sandbox
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        let result = validate_path("a/b/../c.txt", Some(dir.path()));
        assert!(
            result.is_ok(),
            "Should allow .. that stays within sandbox: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_no_base_dir_allows_relative_under_cwd() {
        // Unrestricted mode (no base configured): a relative path resolves
        // against the cwd and is allowed.
        let result = validate_path("Cargo.toml", None);
        assert!(
            result.is_ok(),
            "Relative path should resolve against cwd when unconfigured: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_no_base_dir_allows_absolute_path() {
        // Unrestricted mode is the deliberate trusted-local-operator contract:
        // with no base configured, absolute paths outside the cwd are allowed.
        // Untrusted contexts must register the tools WITH a base_dir (see
        // ToolRegistry::register_filesystem_tools, which warns when none is set).
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, "hi").unwrap();
        let result = validate_path(file.to_str().unwrap(), None);
        assert!(
            result.is_ok(),
            "Absolute path should be allowed in unrestricted mode: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_with_base_dir_still_rejects_absolute_escape() {
        // Containment remains fail-closed when a base IS configured.
        let dir = TempDir::new().unwrap();
        let result = validate_path("/etc/passwd", Some(dir.path()));
        assert!(
            result.is_err(),
            "Absolute escape must be rejected when a base_dir is configured, got: {:?}",
            result
        );
    }
}
