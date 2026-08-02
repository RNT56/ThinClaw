//! Memory/workspace CLI commands.
//!
//! Exposes the workspace system for direct CLI use without starting the agent.

use std::io::{IsTerminal, Read, Write};
use std::sync::Arc;

use clap::Subcommand;

use crate::terminal_branding::TerminalBranding;
use crate::workspace::{EmbeddingProvider, SearchConfig, Workspace};
use thinclaw_tools::builtin::memory::{MemoryDeleteAction, resolve_memory_delete_action};

/// Run a memory command using the Database trait (works with any backend).
pub async fn run_memory_command_with_db(
    cmd: MemoryCommand,
    db: std::sync::Arc<dyn crate::db::Database>,
    embeddings: Option<Arc<dyn EmbeddingProvider>>,
) -> anyhow::Result<()> {
    let mut workspace = Workspace::new_with_db("default", db);
    if let Some(emb) = embeddings {
        workspace = workspace.with_embeddings(emb);
    }

    match cmd {
        MemoryCommand::Search { query, limit } => search(&workspace, &query, limit).await,
        MemoryCommand::Read { path } => read(&workspace, &path).await,
        MemoryCommand::Write {
            path,
            content,
            append,
        } => write(&workspace, &path, content, append).await,
        MemoryCommand::Tree { path, depth } => tree(&workspace, &path, depth).await,
        MemoryCommand::Delete { path, dry_run, yes } => {
            delete(&workspace, &path, dry_run, yes).await
        }
        MemoryCommand::Status => status(&workspace).await,
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum MemoryCommand {
    /// Search workspace memory (hybrid full-text + semantic)
    Search {
        /// Search query
        query: String,

        /// Maximum number of results
        #[arg(short, long, default_value = "5")]
        limit: usize,
    },

    /// Read a file from the workspace
    Read {
        /// File path (e.g., "MEMORY.md", "daily/2024-01-15.md")
        path: String,
    },

    /// Write content to a workspace file
    Write {
        /// File path (e.g., "notes/idea.md")
        path: String,

        /// Content to write (omit to read from stdin)
        content: Option<String>,

        /// Append instead of overwrite
        #[arg(short, long)]
        append: bool,
    },

    /// Show workspace directory tree
    Tree {
        /// Root path to start from
        #[arg(default_value = "")]
        path: String,

        /// Maximum depth to traverse
        #[arg(short, long, default_value = "3")]
        depth: usize,
    },

    /// Show workspace status (document count, index health)
    Status,

    /// Delete a workspace document using the shared memory safety policy
    Delete {
        /// Exact workspace path to delete
        path: String,

        /// Resolve and report the operation without changing data
        #[arg(long)]
        dry_run: bool,

        /// Confirm destructive deletion noninteractively
        #[arg(long)]
        yes: bool,
    },
}

/// Run a memory command (PostgreSQL backend).
#[cfg(feature = "postgres")]
pub async fn run_memory_command(
    cmd: MemoryCommand,
    pool: deadpool_postgres::Pool,
    embeddings: Option<Arc<dyn EmbeddingProvider>>,
) -> anyhow::Result<()> {
    let mut workspace = Workspace::new("default", pool);
    if let Some(emb) = embeddings {
        workspace = workspace.with_embeddings(emb);
    }

    match cmd {
        MemoryCommand::Search { query, limit } => search(&workspace, &query, limit).await,
        MemoryCommand::Read { path } => read(&workspace, &path).await,
        MemoryCommand::Write {
            path,
            content,
            append,
        } => write(&workspace, &path, content, append).await,
        MemoryCommand::Tree { path, depth } => tree(&workspace, &path, depth).await,
        MemoryCommand::Delete { path, dry_run, yes } => {
            delete(&workspace, &path, dry_run, yes).await
        }
        MemoryCommand::Status => status(&workspace).await,
    }
}

async fn delete(workspace: &Workspace, path: &str, dry_run: bool, yes: bool) -> anyhow::Result<()> {
    let action = match resolve_memory_delete_action(path) {
        Ok(action) => action,
        Err(error) => anyhow::bail!(error.to_string()),
    };
    let resolved = match action {
        MemoryDeleteAction::Delete { normalized_path } => normalized_path,
        MemoryDeleteAction::FinalizeBootstrap => crate::workspace::paths::BOOTSTRAP,
    };

    if !workspace.exists(resolved).await? {
        anyhow::bail!("Memory document not found: {resolved}");
    }

    if dry_run {
        println!("Would delete memory document: {resolved}");
        return Ok(());
    }
    require_delete_confirmation(resolved, yes)?;

    match action {
        MemoryDeleteAction::Delete { .. } => {
            workspace.delete(resolved).await?;
        }
        MemoryDeleteAction::FinalizeBootstrap => {
            // The sentinel preserves the shared memory_delete bootstrap-completion
            // contract: startup must not seed BOOTSTRAP.md again after completion.
            workspace
                .write(resolved, "<!-- bootstrap completed -->")
                .await?;
            if workspace.user_id() != "default" {
                workspace
                    .scoped_clone("default", workspace.agent_id())
                    .write(resolved, "<!-- bootstrap completed -->")
                    .await?;
            }
        }
    }
    println!("Deleted memory document: {resolved}");
    Ok(())
}

fn require_delete_confirmation(path: &str, yes: bool) -> anyhow::Result<()> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("memory deletion requires --yes in noninteractive mode");
    }
    eprint!("Permanently delete memory document {path}? [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        anyhow::bail!("memory deletion cancelled");
    }
    Ok(())
}

async fn search(workspace: &Workspace, query: &str, limit: usize) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let config = SearchConfig::default().with_limit(limit.min(50));
    let results = workspace.search_with_config(query, config).await?;

    if results.is_empty() {
        branding.print_banner("Memory", Some("Search workspace knowledge"));
        println!(
            "{}",
            branding.warn(format!("No results found for: {}", query))
        );
        return Ok(());
    }

    branding.print_banner("Memory", Some("Search workspace knowledge"));
    println!(
        "{}",
        branding.muted(format!(
            "Found {} result(s) for \"{}\":",
            results.len(),
            query
        ))
    );
    println!();

    for (i, result) in results.iter().enumerate() {
        let score_bar = score_indicator(result.score);
        println!(
            "{}. [{}] {} (score: {:.3}, document: {}, chunk: {}, fts-rank: {}, vector-rank: {})",
            i + 1,
            score_bar,
            result.path,
            result.score,
            result.document_id,
            result.chunk_id,
            result
                .fts_rank
                .map_or_else(|| "-".to_string(), |rank| rank.to_string()),
            result
                .vector_rank
                .map_or_else(|| "-".to_string(), |rank| rank.to_string()),
        );

        // Show a content preview (first 200 chars)
        let preview = truncate_content(&result.content, 200);
        for line in preview.lines() {
            println!("   {}", line);
        }
        println!();
    }

    Ok(())
}

async fn read(workspace: &Workspace, path: &str) -> anyhow::Result<()> {
    match workspace.read(path).await {
        Ok(doc) => {
            println!("{}", doc.content);
        }
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {
            anyhow::bail!("File not found: {}", path);
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

async fn write(
    workspace: &Workspace,
    path: &str,
    content: Option<String>,
    append: bool,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let content = match content {
        Some(c) => c,
        None => {
            // Read from stdin
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    if append {
        workspace.append(path, &content).await?;
        branding.print_banner("Memory", Some("Write workspace content"));
        println!("  {}", branding.good(format!("Appended to {}", path)));
    } else {
        workspace.write(path, &content).await?;
        branding.print_banner("Memory", Some("Write workspace content"));
        println!("  {}", branding.good(format!("Wrote to {}", path)));
    }

    Ok(())
}

async fn tree(workspace: &Workspace, path: &str, max_depth: usize) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let root = if path.is_empty() { "." } else { path };
    branding.print_banner("Memory", Some("Browse workspace structure"));
    println!("{}", branding.body_bold(format!("{}/", root)));
    print_tree(workspace, path, "", max_depth, 0).await?;
    Ok(())
}

async fn print_tree(
    workspace: &Workspace,
    path: &str,
    prefix: &str,
    max_depth: usize,
    current_depth: usize,
) -> anyhow::Result<()> {
    if current_depth >= max_depth {
        return Ok(());
    }

    let entries = workspace.list(path).await?;
    let count = entries.len();

    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        if entry.is_directory {
            println!("{}{}{}/", prefix, connector, entry.name());
            Box::pin(print_tree(
                workspace,
                &entry.path,
                &format!("{}{}", prefix, child_prefix),
                max_depth,
                current_depth + 1,
            ))
            .await?;
        } else {
            println!("{}{}{}", prefix, connector, entry.name());
        }
    }

    Ok(())
}

async fn status(workspace: &Workspace) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let all_paths = workspace.list_all().await?;
    let file_count = all_paths.len();

    // Count directories by collecting unique parent paths
    let mut dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in &all_paths {
        if let Some(parent) = path.rsplit_once('/') {
            dirs.insert(parent.0.to_string());
        }
    }

    branding.print_banner("Memory", Some("Inspect workspace health"));
    println!("{}", branding.key_value("User", workspace.user_id()));
    println!("{}", branding.key_value("Files", file_count));
    println!("{}", branding.key_value("Directories", dirs.len()));

    // Check key files
    let key_files = [
        "MEMORY.md",
        "HEARTBEAT.md",
        "IDENTITY.md",
        "SOUL.md",
        "AGENTS.md",
        "USER.md",
    ];
    println!();
    println!("{}", branding.body_bold("Identity files:"));
    for path in &key_files {
        let exists = workspace.exists(path).await.unwrap_or(false);
        let marker = if exists {
            branding.good("+")
        } else {
            branding.warn("-")
        };
        println!("    [{}] {}", marker, path);
    }

    Ok(())
}

fn truncate_content(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let safe_end = crate::util::floor_char_boundary(s, max_len);
        format!("{}...", &s[..safe_end])
    }
}

fn score_indicator(score: f32) -> &'static str {
    if score > 0.8_f32 {
        "=====>"
    } else if score > 0.5_f32 {
        "====>"
    } else if score > 0.3_f32 {
        "===>"
    } else if score > 0.1_f32 {
        "==>"
    } else {
        "=>"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_indicator() {
        assert_eq!(score_indicator(0.9_f32), "=====>");
        assert_eq!(score_indicator(0.6_f32), "====>");
        assert_eq!(score_indicator(0.4_f32), "===>");
        assert_eq!(score_indicator(0.2_f32), "==>");
        assert_eq!(score_indicator(0.05_f32), "=>");
    }

    #[test]
    fn test_truncate_content() {
        assert_eq!(truncate_content("hello", 10), "hello");
        assert_eq!(truncate_content("hello world", 5), "hello...");
    }
}
