//! Workspace struct, Workspace impl, and helper functions.
//!
//! The core `Workspace` type providing the database-backed memory API:
//! file operations (read/write/append/delete/list), system prompt building,
//! search, indexing, and workspace seeding.

use std::{fs, path::PathBuf, sync::Arc};

use chrono::NaiveDate;
#[cfg(feature = "postgres")]
use deadpool_postgres::Pool;
use uuid::Uuid;

use crate::chunker::{ChunkConfig, chunk};
use crate::document::{MemoryDocument, WorkspaceEntry, paths};
use crate::embeddings::EmbeddingProvider;
#[cfg(feature = "postgres")]
use crate::repository::Repository;
use crate::search::{SearchConfig, SearchResult};
use crate::{WorkspaceBackend, WorkspaceStore};
use thinclaw_identity::{ConversationKind, LinkedConversationRecall, ResolvedIdentity};
use thinclaw_safety::{pii_redactor, sanitize_context_content};
use thinclaw_types::error::WorkspaceError;

/// Maximum characters per workspace file injected into the system prompt.
/// Matches openclaw's `bootstrapMaxChars` default (~20k chars ≈ 5k tokens).
const FILE_MAX_CHARS: usize = 4_000;

/// Truncate `text` to at most `max` chars, appending a truncation notice.
fn cap_chars(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i < max)
        .last()
        .unwrap_or(0);
    format!(
        "{}\n\n_[... truncated — file exceeds {max} chars. Use `memory_read` to see the rest.]_",
        &text[..cut]
    )
}

/// Extract essential operational instructions from AGENTS.md content.
///
/// Keeps only the operationally critical sections (startup, memory policy,
/// safety boundaries, external-action rules, group-chat behavior, tool-use
/// notes, and heartbeat conduct). Everything else can be read on demand via
/// `memory_read AGENTS.md`.
fn extract_essential_instructions(agents_content: &str) -> String {
    let mut essential = Vec::new();
    let mut in_keep_section = false;

    // Section headers to KEEP in the system prompt (critical operational rules)
    let keep_keywords = [
        "First Run",
        "Session Startup",
        "Memory",
        "MEMORY.md",
        "Write It Down",
        "Mental Notes",
        "Red Lines",
        "Protected Repo Boundary Policy",
        "Feature Parity Update Policy",
        "External vs Internal",
        "Group Chats",
        "Know When to Speak",
        "Tools",
        "Platform Formatting",
        "Heartbeats",
        "Be Proactive",
    ];

    for line in agents_content.lines() {
        let trimmed = line.trim();

        // Detect top-level section headers.
        if trimmed.starts_with("## ") {
            // Strip markdown heading markers + emoji for clean matching
            let header_text = trimmed
                .trim_start_matches('#')
                .trim()
                .trim_start_matches(|c: char| !c.is_alphabetic())
                .trim();
            in_keep_section = keep_keywords.iter().any(|h| header_text.contains(h));
            if in_keep_section {
                essential.push(line.to_string());
            }
            continue;
        }

        // Keep nested headings if we're inside an already-kept top-level section.
        // If we're not, still allow known critical subsection headings through.
        if trimmed.starts_with("### ") {
            if in_keep_section {
                essential.push(line.to_string());
                continue;
            }
            let header_text = trimmed
                .trim_start_matches('#')
                .trim()
                .trim_start_matches(|c: char| !c.is_alphabetic())
                .trim();
            in_keep_section = keep_keywords.iter().any(|h| header_text.contains(h));
            if in_keep_section {
                essential.push(line.to_string());
            }
            continue;
        }

        if in_keep_section {
            essential.push(line.to_string());
        }
    }

    if essential.is_empty() {
        // Fallback: first 400 chars if no sections matched
        cap_chars(agents_content, 400)
    } else {
        essential.push(String::new());
        essential.push("Full instructions: `memory_read AGENTS.md`".to_string());
        essential.join("\n")
    }
}

fn extract_markdown_fields(content: &str) -> Vec<String> {
    let mut fields = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("- **") && t.contains(":**") {
            let after_colon = t.split_once(":**").map(|x| x.1).unwrap_or("").trim();
            if !after_colon.is_empty() && !after_colon.starts_with("_(") && after_colon != "_" {
                fields.push(t.to_string());
            }
        }
    }
    fields
}

fn upsert_timezone_line(content: &str, timezone: Option<&str>) -> String {
    let replacement = match timezone {
        Some(value) => format!("- **Timezone:** {}", value),
        None => "- **Timezone:**".to_string(),
    };
    let mut replaced = false;
    let mut lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- **Timezone:**") || trimmed.starts_with("- **Timezone**:") {
            lines.push(replacement.clone());
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !replaced && timezone.is_some() {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(replacement);
    }

    let mut updated = lines.join("\n");
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated
}

fn summarize_profile_json(content: &str) -> Option<String> {
    let profile: serde_json::Value = match serde_json::from_str(content) {
        Ok(profile) => profile,
        Err(e) => {
            tracing::debug!("Failed to parse profile.json for system prompt: {}", e);
            return None;
        }
    };
    if !profile_is_populated(&profile) {
        return None;
    }

    let confidence = profile
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| {
            profile
                .pointer("/analysis_metadata/confidence_score")
                .and_then(serde_json::Value::as_f64)
        })
        .unwrap_or_default();
    if confidence >= 0.6 {
        Some(cap_chars(&profile_to_user_md(&profile), FILE_MAX_CHARS))
    } else if confidence >= 0.3 {
        Some(format!(
            "## User Profile (preliminary)\n\n{}",
            profile_basics(&profile).join("\n")
        ))
    } else {
        None
    }
}

fn json_str<'a>(value: &'a serde_json::Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

fn json_string_vec(value: &serde_json::Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|item| !item.trim().is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn profile_is_populated(profile: &serde_json::Value) -> bool {
    !json_str(profile, "/preferred_name").is_empty()
        || !json_str(profile, "/context/profession").is_empty()
        || !json_string_vec(profile, "/assistance/goals").is_empty()
}

fn cohort_label(raw: &str) -> &str {
    match raw {
        "busy_professional" => "busy professional",
        "new_parent" => "new parent",
        "student" => "student",
        "elder" => "elder",
        _ => "general",
    }
}

fn profile_basics(profile: &serde_json::Value) -> Vec<String> {
    let mut basics = Vec::new();
    let preferred_name = json_str(profile, "/preferred_name");
    if !preferred_name.is_empty() {
        basics.push(format!("**Name**: {}", preferred_name));
    }
    basics.push(format!(
        "**Communication**: {} tone, {} detail, {} formality",
        json_str(profile, "/communication/tone"),
        json_str(profile, "/communication/detail_level"),
        json_str(profile, "/communication/formality"),
    ));
    let cohort = json_str(profile, "/cohort/cohort");
    if !cohort.is_empty() && cohort != "other" {
        let confidence = profile
            .pointer("/cohort/confidence")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        basics.push(format!(
            "**User type**: {} ({}% confidence)",
            cohort_label(cohort),
            confidence
        ));
    }
    basics
}

fn profile_to_user_md(profile: &serde_json::Value) -> String {
    let mut sections = vec!["# User Profile\n".to_string()];
    sections.extend(profile_basics(profile));

    for (label, pointer) in [
        ("**Profession**", "/context/profession"),
        ("**Life stage**", "/context/life_stage"),
    ] {
        let value = json_str(profile, pointer);
        if !value.is_empty() {
            sections.push(format!("{label}: {value}"));
        }
    }

    for (label, pointer) in [
        ("**Interests**", "/context/interests"),
        ("**Goals**", "/assistance/goals"),
        ("**Focus areas**", "/assistance/focus_areas"),
        ("**Strengths**", "/behavior/strengths"),
        ("**Pain points**", "/behavior/pain_points"),
        ("**Core values**", "/relationship_values/primary"),
    ] {
        let values = json_string_vec(profile, pointer);
        if !values.is_empty() {
            sections.push(format!("{label}: {}", values.join(", ")));
        }
    }

    let proactivity = json_str(profile, "/assistance/proactivity");
    let interaction_style = json_str(profile, "/assistance/interaction_style");
    if !proactivity.is_empty() || !interaction_style.is_empty() {
        sections.push(format!(
            "\n## Assistance Preferences\n\n- **Proactivity**: {}\n- **Interaction style**: {}",
            proactivity, interaction_style
        ));
    }

    sections.join("\n")
}

fn sanitize_prompt_context(file_name: &str, content: &str) -> String {
    let (cleaned, warnings) = sanitize_context_content(content);
    for warning in warnings {
        tracing::warn!(
            file = file_name,
            pattern = %warning.pattern,
            matched = %warning.matched,
            "Suspicious context content detected during prompt assembly"
        );
    }
    cleaned
}

#[allow(dead_code)] // Retained for shared/global memory summarisation
fn summarize_memory_content(content: &str) -> String {
    let entry_count = content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .count();

    if entry_count == 0 {
        String::new()
    } else {
        format!("MEMORY.md: {} entries (long-term notes)", entry_count)
    }
}

fn summarize_actor_memory_content(content: &str) -> String {
    let entry_count = content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .count();

    if entry_count == 0 {
        String::new()
    } else {
        format!("MEMORY.md: {} entries (actor-private notes)", entry_count)
    }
}

fn linked_recall_is_empty(recall: &LinkedConversationRecall) -> bool {
    recall.source_channel.is_empty()
        && recall.source_conversation_key.is_empty()
        && recall
            .handoff_summary
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        && recall.summary.as_deref().unwrap_or("").trim().is_empty()
        && recall
            .last_user_goal
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
}

#[derive(Debug, Clone, Copy)]
struct PromptRedaction<'a> {
    channel: Option<&'a str>,
    enabled: bool,
}

impl<'a> PromptRedaction<'a> {
    fn new(channel: Option<&'a str>, enabled: bool) -> Self {
        Self { channel, enabled }
    }

    fn should_redact(self) -> bool {
        self.enabled && self.channel.is_some_and(pii_redactor::should_redact)
    }

    fn actor_label(self, actor_id: &str) -> String {
        match self.channel {
            Some(channel) if self.enabled => pii_redactor::redact_for_prompt(actor_id, channel),
            _ => actor_id.to_string(),
        }
    }

    fn sensitive_label(self, value: &str) -> String {
        if self.should_redact() {
            pii_redactor::hash_user_id(value)
        } else {
            value.to_string()
        }
    }
}

fn format_linked_recall(
    recall: &LinkedConversationRecall,
    redaction: PromptRedaction<'_>,
) -> String {
    let mut lines = vec!["## Linked Conversation Recall".to_string()];
    if !recall.actor_id.is_empty() {
        lines.push(format!(
            "- Actor: {}",
            redaction.actor_label(&recall.actor_id)
        ));
    }
    if !recall.source_channel.is_empty() {
        lines.push(format!("- Source channel: {}", recall.source_channel));
    }
    if !recall.source_conversation_key.is_empty() {
        lines.push(format!(
            "- Source conversation: {}",
            redaction.sensitive_label(&recall.source_conversation_key)
        ));
    }
    if !recall
        .handoff_summary
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        lines.push(format!(
            "- Handoff: {}",
            recall.handoff_summary.as_deref().unwrap_or("").trim()
        ));
    }
    if !recall.summary.as_deref().unwrap_or("").trim().is_empty() {
        lines.push(format!(
            "- Summary: {}",
            recall.summary.as_deref().unwrap_or("").trim()
        ));
    }
    if !recall
        .last_user_goal
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        lines.push(format!(
            "- Last goal: {}",
            recall.last_user_goal.as_deref().unwrap_or("").trim()
        ));
    }
    lines.join("\n")
}

/// Default template seeded into HEARTBEAT.md on first access.
///
/// Includes a minimal default health check so the agent has baseline
/// autonomous behavior. Users can add/remove items via chat or the
/// Agent Memory editor. The `is_effectively_empty` guard only skips
/// lines starting with `#` or containing only empty checkboxes, so
/// these real checklist items will trigger the LLM evaluation.
const HEARTBEAT_SEED: &str = "\
# Heartbeat Checklist

<!-- Add, edit, or remove items below. The agent checks this every 30 minutes.
     If nothing needs attention, it stays silent (HEARTBEAT_OK).
     If something does, it proactively sends you a message.
     Daily logs are injected below the checklist automatically. -->

- [ ] Review the daily logs below for unresolved tasks, open questions, or recently finished goals — if you spot potential next steps or follow-up work, proactively message the user with a brief suggestion
- [ ] If daily logs contain important decisions, lessons, or facts not yet in MEMORY.md, consolidate them into MEMORY.md now using memory_write (target: 'memory')";

#[cfg(test)]
fn personality_pack_content(pack: &str) -> String {
    thinclaw_soul::pack_asset_markdown(pack).to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeSoulStatus {
    Existing,
    CreatedFromPack,
    MigratedLegacyWorkspaceSoul,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HomeSoulEnsureResult {
    status: HomeSoulStatus,
    content: String,
    path: PathBuf,
}

fn canonical_soul_path() -> PathBuf {
    thinclaw_platform::state_paths().soul_file
}

fn read_home_soul() -> Result<String, WorkspaceError> {
    let path = canonical_soul_path();
    fs::read_to_string(&path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => WorkspaceError::DocumentNotFound {
            doc_type: paths::SOUL.to_string(),
            user_id: "home".to_string(),
        },
        _ => WorkspaceError::SearchFailed {
            reason: format!("failed to read {}: {}", path.display(), err),
        },
    })
}

fn write_home_soul(content: &str) -> Result<(), WorkspaceError> {
    let path = canonical_soul_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkspaceError::SearchFailed {
            reason: format!("failed to create {}: {}", parent.display(), err),
        })?;
    }
    fs::write(&path, content).map_err(|err| WorkspaceError::SearchFailed {
        reason: format!("failed to write {}: {}", path.display(), err),
    })
}

async fn ensure_home_soul(
    workspace: &Workspace,
    seed_pack: &str,
) -> Result<HomeSoulEnsureResult, WorkspaceError> {
    let path = canonical_soul_path();
    if path.exists() {
        migrate_workspace_legacy_soul(workspace).await?;
        return Ok(HomeSoulEnsureResult {
            status: HomeSoulStatus::Existing,
            content: read_home_soul()?,
            path,
        });
    }

    if workspace.agent_id().is_none()
        && let Ok(legacy) = workspace.read(paths::SOUL).await
        && !legacy.content.trim().is_empty()
    {
        write_home_soul(&legacy.content)?;
        archive_legacy_soul(workspace, &legacy.content).await?;
        workspace.delete(paths::SOUL).await?;
        return Ok(HomeSoulEnsureResult {
            status: HomeSoulStatus::MigratedLegacyWorkspaceSoul,
            content: legacy.content,
            path,
        });
    }

    let content = thinclaw_soul::compose_seeded_soul(seed_pack).map_err(|err| {
        WorkspaceError::SearchFailed {
            reason: format!("failed to compose seeded home soul: {}", err),
        }
    })?;
    write_home_soul(&content)?;
    migrate_workspace_legacy_soul(workspace).await?;
    Ok(HomeSoulEnsureResult {
        status: HomeSoulStatus::CreatedFromPack,
        content,
        path,
    })
}

async fn migrate_workspace_legacy_soul(workspace: &Workspace) -> Result<(), WorkspaceError> {
    let Ok(legacy) = workspace.read(paths::SOUL).await else {
        return Ok(());
    };
    if legacy.content.trim().is_empty() {
        return Ok(());
    }

    if workspace.agent_id().is_some() && !workspace.exists(paths::SOUL_LOCAL).await? {
        workspace.write(paths::SOUL_LOCAL, &legacy.content).await?;
    }

    archive_legacy_soul(workspace, &legacy.content).await?;
    workspace.delete(paths::SOUL).await?;
    Ok(())
}

async fn archive_legacy_soul(workspace: &Workspace, content: &str) -> Result<(), WorkspaceError> {
    workspace.write(paths::SOUL_LEGACY, content).await?;
    Ok(())
}

fn is_effectively_empty(content: &str) -> bool {
    let without_comments = strip_html_comments(content);
    without_comments.lines().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed == "- [ ]"
            || trimmed == "- [x]"
            || trimmed == "-"
            || trimmed == "*"
    })
}

fn strip_html_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<!--") {
        result.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return result,
        }
    }
    result.push_str(rest);
    result
}

fn bootstrap_template(name: &str, has_custom_name: bool, full_soul_bootstrap: bool) -> String {
    if full_soul_bootstrap {
        if has_custom_name {
            format!(
                "# BOOTSTRAP.md - Hello, World\n\n\
                 *You just woke up. Time to figure out who you are.*\n\n\
                 There is no memory yet. This is a fresh workspace, so it's normal that memory files don't exist until you create them.\n\n\
                 ## What You Already Know\n\n\
                 Your name is **{name}** — this was chosen during setup. Don't ask for it again.\n\n\
                 ## The Conversation\n\n\
                 Don't interrogate. Don't be robotic. Just... talk.\n\n\
                 Start with something like:\n\
                 > \"Hey! I'm {name}. I just came online — tell me about yourself so I can be genuinely useful.\"\n\n\
                 Then figure out together:\n\
                 1. **Your nature** — What kind of creature are you?\n\
                 2. **Your presentation** — Formal? Casual? Snarky? Warm? What feels right?\n\
                 3. **Your emoji** — Everyone needs a signature.\n\n\
                 Update these files with what you learn:\n\
                 - `IDENTITY.md` — your creature, presentation, emoji (Name is already set; use `memory_write` with target `IDENTITY.md`, append false)\n\
                 - `USER.md` — their name, how to address them, timezone, notes (use `prompt_manage`)\n\n\
                 Then read the canonical `SOUL.md` in THINCLAW_HOME. It's included in this prompt too.\n\
                 Co-author it with your human:\n\
                 - Which core truths should stay exactly as they are?\n\
                 - What should change in the vibe or default behaviors?\n\
                 - Any sharper boundaries or preferences?\n\n\
                 Then rewrite `SOUL.md` with `prompt_manage` (append false). Build on the seed values instead of discarding them.\n\n\
                 If this workspace needs different behavior, ask first before creating `SOUL.local.md`. The default is to inherit the global soul.\n\n\
                 ## When You're Done\n\n\
                 Delete `BOOTSTRAP.md` with `memory_delete` so you don't re-enter bootstrap mode next session.\n"
            )
        } else {
            "# BOOTSTRAP.md - Hello, World\n\n\
             *You just woke up. Time to figure out who you are.*\n\n\
             There is no memory yet. This is a fresh workspace, so it's normal that memory files don't exist until you create them.\n\n\
             ## The Conversation\n\n\
             Don't interrogate. Don't be robotic. Just... talk.\n\n\
             Start with something like:\n\
             > \"Hey. I just came online. Who am I? Who are you?\"\n\n\
             Then figure out together:\n\
             1. **Your name** — What should they call you?\n\
             2. **Your nature** — What kind of creature are you?\n\
             3. **Your presentation** — Formal? Casual? Snarky? Warm? What feels right?\n\
             4. **Your emoji** — Everyone needs a signature.\n\n\
             Update these files with what you learn:\n\
             - `IDENTITY.md` — your name, creature, presentation, emoji (use `memory_write` with target `IDENTITY.md`, append false)\n\
             - `USER.md` — their name, how to address them, timezone, notes (use `prompt_manage`)\n\n\
             Then read the canonical `SOUL.md` in THINCLAW_HOME. It's included in this prompt too.\n\
             Co-author it with your human, then rewrite it with `prompt_manage` (append false) while keeping the durable character spine intact.\n\n\
             If this workspace needs different behavior, ask first before creating `SOUL.local.md`. The default is to inherit the global soul.\n\n\
             ## When You're Done\n\n\
             Delete `BOOTSTRAP.md` with `memory_delete` so you don't re-enter bootstrap mode next session.\n"
                .to_string()
        }
    } else {
        format!(
            "# BOOTSTRAP.md - Workspace Alignment\n\n\
             *This workspace already has a global soul. Your job is to align this workspace with it.*\n\n\
             Read the canonical `SOUL.md`, `IDENTITY.md`, `USER.md`, and `AGENTS.md`.\n\
             The global soul already exists, so do not re-open the foundational \"who are you\" conversation.\n\n\
             ## What To Do\n\n\
             - Confirm the user-facing details in `IDENTITY.md` and `USER.md`\n\
             - Respect the canonical `SOUL.md` as the default behavior across projects\n\
             - Only create `SOUL.local.md` if the user explicitly wants workspace-specific tone adjustments or stricter boundaries\n\
             - Prefer `AGENTS.md` and agent-specific system prompts for specialized workflow rules\n\n\
             ## If A Local Overlay Is Needed\n\n\
             Create `SOUL.local.md` with:\n\
             - `## Workspace Context`\n\
             - `## Tone Adjustments`\n\
             - `## Boundary Tightening`\n\n\
             Keep it additive only. Do not relax the global soul's privacy or external-action boundaries.\n\n\
             ## When You're Done\n\n\
             Delete `BOOTSTRAP.md` with `memory_delete`.\n\
             Agent name from setup, if present: {name}\n"
        )
    }
}

/// Workspace provides database-backed memory storage for an agent.
///
/// Each workspace is scoped to a user (and optionally an agent).
/// Documents are persisted to the database and indexed for search.
/// Supports both PostgreSQL (via Repository) and libSQL (via Database trait).
#[derive(Clone)]
pub struct Workspace {
    /// User identifier (from channel).
    user_id: String,
    /// Optional agent ID for multi-agent isolation.
    agent_id: Option<Uuid>,
    /// Database storage backend.
    storage: WorkspaceBackend,
    /// Embedding provider for semantic search.
    embeddings: Option<Arc<dyn EmbeddingProvider>>,
}

impl Workspace {
    /// Create a new workspace backed by a PostgreSQL connection pool.
    #[cfg(feature = "postgres")]
    pub fn new(user_id: impl Into<String>, pool: Pool) -> Self {
        let store: WorkspaceBackend = Arc::new(Repository::new(pool));
        Self::new_with_store(user_id, store)
    }

    /// Create a new workspace backed by any workspace store implementation.
    pub fn new_with_store(user_id: impl Into<String>, store: WorkspaceBackend) -> Self {
        Self {
            user_id: user_id.into(),
            agent_id: None,
            storage: store,
            embeddings: None,
        }
    }

    /// Create a new workspace backed by any Database implementation.
    ///
    /// Use this for libSQL or any other backend that implements the Database trait.
    pub fn new_with_db<T>(user_id: impl Into<String>, db: Arc<T>) -> Self
    where
        T: WorkspaceStore + 'static + ?Sized,
    {
        Self::new_with_store(user_id, Arc::new(db))
    }

    /// Create a workspace with a specific agent ID.
    pub fn with_agent(mut self, agent_id: Uuid) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    /// Set the embedding provider for semantic search.
    pub fn with_embeddings(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embeddings = Some(provider);
        self
    }

    /// Get the user ID.
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Get the agent ID.
    pub fn agent_id(&self) -> Option<Uuid> {
        self.agent_id
    }

    /// Resolve the workspace's effective timezone.
    pub fn effective_timezone(&self) -> chrono_tz::Tz {
        thinclaw_platform::timezone::resolve_effective_timezone(Some(&self.user_id), None)
    }

    /// Get today's local date for this workspace.
    pub fn local_today(&self) -> NaiveDate {
        thinclaw_platform::timezone::today_for_user(Some(&self.user_id), None)
    }

    /// Get the current local time for this workspace.
    pub fn local_now(&self) -> chrono::DateTime<chrono_tz::Tz> {
        thinclaw_platform::timezone::now_for_user(Some(&self.user_id), None)
    }

    /// Clone this workspace's backend/embeddings while changing the scope.
    pub fn scoped_clone(&self, user_id: impl Into<String>, agent_id: Option<Uuid>) -> Self {
        Self {
            user_id: user_id.into(),
            agent_id,
            storage: self.storage.clone(),
            embeddings: self.embeddings.clone(),
        }
    }

    /// Resolve the path to an actor-private file.
    pub fn actor_path(actor_id: &str, file: &str) -> String {
        format!(
            "{}/{}",
            paths::actor_root(actor_id),
            file.trim_start_matches('/')
        )
    }

    /// Get the actor-private USER.md path.
    pub fn actor_user_path(actor_id: &str) -> String {
        paths::actor_user(actor_id)
    }

    /// Get the actor-private MEMORY.md path.
    pub fn actor_memory_path(actor_id: &str) -> String {
        paths::actor_memory(actor_id)
    }

    /// Get the actor-private profile path.
    pub fn actor_profile_path(actor_id: &str) -> String {
        paths::actor_profile(actor_id)
    }

    /// Build a system prompt with explicit identity metadata.
    pub async fn system_prompt_for_identity(
        &self,
        identity: Option<&ResolvedIdentity>,
        channel: &str,
        redact_pii: bool,
    ) -> Result<String, WorkspaceError> {
        let Some(identity) = identity else {
            return self.system_prompt_for_context(false).await;
        };

        self.system_prompt_for_context_details(
            matches!(identity.conversation_kind, ConversationKind::Group),
            Some(identity.actor_id.as_str()),
            None,
            Some(channel),
            redact_pii,
        )
        .await
    }

    // ==================== File Operations ====================

    /// Read a file by path.
    ///
    /// Returns the document if it exists, or an error if not found.
    ///
    /// # Example
    /// ```ignore
    /// let doc = workspace.read("context/vision.md").await?;
    /// println!("{}", doc.content);
    /// ```
    pub async fn read(&self, path: &str) -> Result<MemoryDocument, WorkspaceError> {
        let path = normalize_path(path);
        self.storage
            .get_document_by_path(&self.user_id, self.agent_id, &path)
            .await
    }

    /// Write (create or update) a file.
    ///
    /// Creates parent directories implicitly (they're virtual in the DB).
    /// Re-indexes the document for search after writing.
    ///
    /// Reindex failures (e.g. missing vector extension, temporary DB lock) are
    /// logged as warnings but do NOT fail the write — content is always durably
    /// persisted even when the search index cannot be updated.
    ///
    /// # Example
    /// ```ignore
    /// workspace.write("projects/alpha/README.md", "# Project Alpha\n\nDescription here.").await?;
    /// ```
    pub async fn write(&self, path: &str, content: &str) -> Result<MemoryDocument, WorkspaceError> {
        let path = normalize_path(path);
        let doc = self
            .storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, &path)
            .await?;
        self.storage.update_document(doc.id, content).await?;

        // Reindex for search — non-fatal: a vector/FTS index failure must not
        // prevent a successful save (content is already durably written above).
        if let Err(e) = self.reindex_document(doc.id).await {
            tracing::warn!(
                doc_id = %doc.id,
                path = %path,
                error = %e,
                "Reindex failed after write — content saved but search index may be stale"
            );
        }

        // Return updated doc
        self.storage.get_document_by_id(doc.id).await
    }

    /// Append content to a file.
    ///
    /// Creates the file if it doesn't exist.
    /// Adds a newline separator between existing and new content.
    ///
    /// Reindex failures are logged as warnings but do NOT fail the append.
    pub async fn append(&self, path: &str, content: &str) -> Result<(), WorkspaceError> {
        let path = normalize_path(path);
        let doc = self
            .storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, &path)
            .await?;

        let new_content = if doc.content.is_empty() {
            content.to_string()
        } else {
            format!("{}\n{}", doc.content, content)
        };

        self.storage.update_document(doc.id, &new_content).await?;

        // Reindex for search — non-fatal (same reasoning as write()).
        if let Err(e) = self.reindex_document(doc.id).await {
            tracing::warn!(
                doc_id = %doc.id,
                path = %path,
                error = %e,
                "Reindex failed after append — content saved but search index may be stale"
            );
        }

        Ok(())
    }

    /// Check if a file exists.
    pub async fn exists(&self, path: &str) -> Result<bool, WorkspaceError> {
        let path = normalize_path(path);
        match self
            .storage
            .get_document_by_path(&self.user_id, self.agent_id, &path)
            .await
        {
            Ok(_) => Ok(true),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Delete a file.
    ///
    /// Also deletes associated chunks.
    pub async fn delete(&self, path: &str) -> Result<(), WorkspaceError> {
        let path = normalize_path(path);
        self.storage
            .delete_document_by_path(&self.user_id, self.agent_id, &path)
            .await
    }

    /// List files and directories in a path.
    ///
    /// Returns immediate children (not recursive).
    /// Use empty string or "/" for root directory.
    ///
    /// # Example
    /// ```ignore
    /// let entries = workspace.list("projects/").await?;
    /// for entry in entries {
    ///     if entry.is_directory {
    ///         println!("📁 {}/", entry.name());
    ///     } else {
    ///         println!("📄 {}", entry.name());
    ///     }
    /// }
    /// ```
    pub async fn list(&self, directory: &str) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let directory = normalize_directory(directory);
        self.storage
            .list_directory(&self.user_id, self.agent_id, &directory)
            .await
    }

    /// List all files recursively (flat list of all paths).
    pub async fn list_all(&self) -> Result<Vec<String>, WorkspaceError> {
        self.storage
            .list_all_paths(&self.user_id, self.agent_id)
            .await
    }

    // ==================== Convenience Methods ====================

    /// Get the main MEMORY.md document (long-term curated memory).
    ///
    /// Creates it if it doesn't exist.
    pub async fn memory(&self) -> Result<MemoryDocument, WorkspaceError> {
        self.read_or_create(paths::MEMORY).await
    }

    /// Get today's daily log.
    ///
    /// Daily logs are append-only and keyed by date.
    pub async fn today_log(&self) -> Result<MemoryDocument, WorkspaceError> {
        let today = self.local_today();
        self.daily_log(today).await
    }

    /// Get a daily log for a specific date.
    pub async fn daily_log(&self, date: NaiveDate) -> Result<MemoryDocument, WorkspaceError> {
        let path = format!("daily/{}.md", date.format("%Y-%m-%d"));
        self.read_or_create(&path).await
    }

    /// Get the heartbeat checklist (HEARTBEAT.md).
    ///
    /// Returns the DB-stored checklist if it exists, otherwise falls back
    /// to the in-memory seed template. The seed is never written to the
    /// database; the user creates the real file via `memory_write` when
    /// they actually want periodic checks. The seed content is all HTML
    /// comments, which the heartbeat runner treats as "effectively empty"
    /// and skips the LLM call.
    pub async fn heartbeat_checklist(&self) -> Result<Option<String>, WorkspaceError> {
        match self.read(paths::HEARTBEAT).await {
            Ok(doc) => Ok(Some(doc.content)),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(Some(HEARTBEAT_SEED.to_string())),
            Err(e) => Err(e),
        }
    }

    /// Helper to read or create a file.
    async fn read_or_create(&self, path: &str) -> Result<MemoryDocument, WorkspaceError> {
        self.storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, path)
            .await
    }

    // ==================== Memory Operations ====================

    /// Append an entry to the main MEMORY.md document.
    ///
    /// This is for important facts, decisions, and preferences worth
    /// remembering long-term.
    pub async fn append_memory(&self, entry: &str) -> Result<(), WorkspaceError> {
        // Use double newline for memory entries (semantic separation)
        let doc = self.memory().await?;
        let new_content = if doc.content.is_empty() {
            entry.to_string()
        } else {
            format!("{}\n\n{}", doc.content, entry)
        };
        self.storage.update_document(doc.id, &new_content).await?;
        self.reindex_document(doc.id).await?;
        Ok(())
    }

    /// Append an entry to today's daily log.
    ///
    /// Daily logs are raw, append-only notes for the current day.
    pub async fn append_daily_log(&self, entry: &str) -> Result<(), WorkspaceError> {
        let now = self.local_now();
        let today = now.date_naive();
        let path = format!("daily/{}.md", today.format("%Y-%m-%d"));
        let timestamp = now.format("%H:%M:%S");
        let timestamped_entry = format!("[{}] {}", timestamp, entry);
        self.append(&path, &timestamped_entry).await
    }

    // ==================== System Prompt ====================

    /// Build the system prompt from identity files.
    ///
    /// Loads the canonical home soul, AGENTS.md, USER.md, IDENTITY.md, and (in non-group
    /// contexts) MEMORY.md to compose the agent's system prompt.
    ///
    /// Shorthand for `system_prompt_for_context(false)`.
    pub async fn system_prompt(&self) -> Result<String, WorkspaceError> {
        self.system_prompt_for_context(false).await
    }

    /// Build the system prompt, optionally excluding personal memory.
    ///
    /// Uses a lean, pi-mono-inspired format:
    /// 1. Canonical home soul plus optional local overlay
    /// 2. Essential instructions (~200 tokens distilled from AGENTS.md)
    /// 3. Context manifest (~50-100 tokens listing available files)
    ///
    /// Full file contents are accessible via `memory_read` on demand.
    /// This keeps the system prompt under ~600 tokens (down from ~5,000-20,000).
    pub async fn system_prompt_for_context(
        &self,
        is_group_chat: bool,
    ) -> Result<String, WorkspaceError> {
        self.system_prompt_for_context_details(is_group_chat, None, None, None, false)
            .await
    }

    /// Build the system prompt with optional actor-private overlay and linked recall.
    pub async fn system_prompt_for_context_details(
        &self,
        is_group_chat: bool,
        actor_id: Option<&str>,
        linked_recall: Option<&LinkedConversationRecall>,
        channel: Option<&str>,
        redact_pii: bool,
    ) -> Result<String, WorkspaceError> {
        let redaction = PromptRedaction::new(channel, redact_pii);

        // ── Bootstrap mode: blank-slate first run ────────────────────────
        // BOOTSTRAP.md gives the ritual instructions. We also inject the
        // canonical home SOUL.md and AGENTS.md so the agent internalizes the
        // durable soul and operational guidelines before rewriting anything.
        if !is_group_chat
            && let Ok(doc) = self.read(paths::BOOTSTRAP).await
            && !is_effectively_empty(&doc.content)
        {
            let mut bootstrap_prompt = doc.content;

            if let Ok(home_soul) = read_home_soul()
                && !home_soul.trim().is_empty()
            {
                let soul_content = sanitize_prompt_context(paths::SOUL, &home_soul);
                bootstrap_prompt.push_str("\n\n---\n\n");
                bootstrap_prompt.push_str("## Your Canonical Soul\n\n");
                bootstrap_prompt.push_str(&cap_chars(&soul_content, FILE_MAX_CHARS));
                bootstrap_prompt.push_str(
                    "\n\n_Absorb these values. They're your durable foundation. When you rewrite SOUL.md, build on them — don't ignore them._",
                );
            }

            if let Ok(local_soul) = self.read(paths::SOUL_LOCAL).await
                && !local_soul.content.is_empty()
            {
                let local_content = sanitize_prompt_context(paths::SOUL_LOCAL, &local_soul.content);
                bootstrap_prompt.push_str("\n\n---\n\n");
                bootstrap_prompt.push_str("## This Workspace's Local Soul Overlay\n\n");
                bootstrap_prompt.push_str(&cap_chars(&local_content, FILE_MAX_CHARS));
            }

            // Append AGENTS.md so the agent knows its workspace conventions
            if let Ok(agents) = self.read(paths::AGENTS).await
                && !agents.content.is_empty()
            {
                let agents_content = sanitize_prompt_context(paths::AGENTS, &agents.content);
                bootstrap_prompt.push_str("\n\n---\n\n");
                bootstrap_prompt.push_str("## Your Workspace Guide (operational reference)\n\n");
                bootstrap_prompt.push_str(&agents_content);
            }

            if let Some(actor_id) = actor_id
                && let Some(actor_overlay) = self.actor_overlay_section(actor_id).await?
            {
                bootstrap_prompt.push_str("\n\n---\n\n");
                bootstrap_prompt.push_str(&actor_overlay);
            }

            if let Some(recall) = linked_recall
                && !linked_recall_is_empty(recall)
            {
                bootstrap_prompt.push_str("\n\n---\n\n");
                bootstrap_prompt.push_str(&format_linked_recall(recall, redaction));
            }

            return Ok(bootstrap_prompt);
        }

        // ── Normal mode: lean identity prompt ────────────────────────────
        let mut parts = Vec::new();

        if let Ok(home_soul) = read_home_soul()
            && !home_soul.trim().is_empty()
        {
            let soul_content = sanitize_prompt_context(paths::SOUL, &home_soul);
            let soul_block = thinclaw_soul::render_canonical_prompt_block(&soul_content);
            parts.push(cap_chars(&soul_block, FILE_MAX_CHARS));
        }

        if let Ok(local_soul) = self.read(paths::SOUL_LOCAL).await
            && !local_soul.content.is_empty()
        {
            let local_content = sanitize_prompt_context(paths::SOUL_LOCAL, &local_soul.content);
            if let Ok(local_block) = thinclaw_soul::render_local_prompt_block(&local_content) {
                parts.push(cap_chars(&local_block, FILE_MAX_CHARS / 2));
            }
        }

        // 1. Compact identity (name, nature, presentation, user info)
        let identity = self.compact_identity().await?;
        if !identity.is_empty() {
            parts.push(format!("## Identity\n\n{}", identity));
        }

        if !is_group_chat
            && let Some(actor_id) = actor_id
            && let Some(actor_overlay) = self.actor_overlay_section(actor_id).await?
        {
            parts.push(actor_overlay);
        }

        // 2. Essential operational instructions (distilled from AGENTS.md)
        if let Ok(doc) = self.read(paths::AGENTS).await
            && !doc.content.is_empty()
        {
            let sanitized_agents = sanitize_prompt_context(paths::AGENTS, &doc.content);
            let essential = extract_essential_instructions(&sanitized_agents);
            if !essential.is_empty() {
                parts.push(format!(
                    "## Instructions\n\n{}",
                    cap_chars(&essential, FILE_MAX_CHARS)
                ));
            }
        }

        // 2b. Tiered psychographic profile injection
        //
        // Injects user personality and preferences from context/profile.json
        // using confidence-gated tiers:
        //   - confidence < 0.3 → skip (too speculative)
        //   - confidence 0.3-0.6 → basics only (name, communication, cohort)
        //   - confidence > 0.6 → full profile summary
        if let Ok(doc) = self.read(paths::PROFILE).await
            && !doc.content.is_empty()
        {
            if let Some(summary) = summarize_profile_json(&doc.content) {
                parts.push(format!("## User Profile\n\n{}", summary));
            }
        }

        if !is_group_chat
            && let Some(recall) = linked_recall
            && !linked_recall_is_empty(recall)
        {
            parts.push(format_linked_recall(recall, redaction));
        }

        // 3. Context manifest (what's available, not the content itself)
        if !is_group_chat {
            let manifest = self
                .context_manifest_for_prompt(actor_id, redaction)
                .await?;
            if !manifest.is_empty() {
                parts.push(format!("## Context\n\n{}", manifest));
            }
        }

        Ok(parts.join("\n\n---\n\n"))
    }

    /// Build a compressed identity block from workspace files.
    ///
    /// Extracts key fields from IDENTITY.md and USER.md.
    /// SOUL.md is injected separately as a dedicated prompt block.
    /// Full files remain accessible via `memory_read`.
    pub async fn compact_identity(&self) -> Result<String, WorkspaceError> {
        let mut lines = Vec::new();

        // IDENTITY.md → extract filled key-value pairs
        if let Ok(doc) = self.read(paths::IDENTITY).await {
            let identity_content = sanitize_prompt_context(paths::IDENTITY, &doc.content);
            for line in identity_content.lines() {
                let t = line.trim();
                if t.starts_with("- **") && t.contains(":**") {
                    let after_colon = t.split_once(":**").map(|x| x.1).unwrap_or("").trim();
                    // Skip unfilled template lines like "_(pick something)_"
                    if !after_colon.is_empty()
                        && !after_colon.starts_with("_(")
                        && after_colon != "_"
                    {
                        lines.push(t.to_string());
                    }
                }
            }
        }

        // USER.md → extract filled fields compactly
        if let Ok(doc) = self.read(paths::USER).await {
            let user_content = sanitize_prompt_context(paths::USER, &doc.content);
            let mut user_fields = Vec::new();
            for line in user_content.lines() {
                let t = line.trim();
                if t.starts_with("- **") && t.contains(":**") {
                    let after_colon = t.split_once(":**").map(|x| x.1).unwrap_or("").trim();
                    if !after_colon.is_empty()
                        && !after_colon.starts_with("_(")
                        && after_colon != "_"
                    {
                        user_fields.push(t.to_string());
                    }
                }
            }
            if !user_fields.is_empty() {
                lines.push(format!("User: {}", user_fields.join(" | ")));
            }
        }

        // Pointer to full files
        if !lines.is_empty() {
            lines.push(
                "Canonical soul: `memory_read SOUL.md` · Full instructions: `memory_read AGENTS.md`"
                    .to_string(),
            );
        }

        Ok(lines.join("\n"))
    }

    /// Build a context manifest summarizing available memory files.
    ///
    /// Tells the agent what context exists without injecting full content.
    /// The agent uses `memory_read` to access files on demand.
    pub async fn context_manifest(&self) -> Result<String, WorkspaceError> {
        self.context_manifest_for_context(None).await
    }

    /// Build a context manifest with optional actor-private files.
    pub async fn context_manifest_for_context(
        &self,
        actor_id: Option<&str>,
    ) -> Result<String, WorkspaceError> {
        self.context_manifest_for_prompt(actor_id, PromptRedaction::new(None, false))
            .await
    }

    async fn context_manifest_for_prompt(
        &self,
        actor_id: Option<&str>,
        redaction: PromptRedaction<'_>,
    ) -> Result<String, WorkspaceError> {
        let mut items = Vec::new();

        // MEMORY.md
        if let Ok(doc) = self.read(paths::MEMORY).await
            && !doc.content.is_empty()
        {
            let entry_count = doc
                .content
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
                .count();
            if entry_count > 0 {
                items.push(format!(
                    "MEMORY.md: {} entries (long-term notes)",
                    entry_count
                ));
            }
        }

        // Today's daily log
        let today = self.local_today();
        if let Ok(doc) = self.daily_log(today).await
            && !doc.content.is_empty()
        {
            let entry_count = doc.content.lines().filter(|l| !l.trim().is_empty()).count();
            items.push(format!(
                "daily/{}.md: {} entries (today)",
                today.format("%Y-%m-%d"),
                entry_count
            ));
        }

        // Yesterday's daily log
        if let Some(yesterday) = today.pred_opt()
            && let Ok(doc) = self.daily_log(yesterday).await
            && !doc.content.is_empty()
        {
            let entry_count = doc.content.lines().filter(|l| !l.trim().is_empty()).count();
            items.push(format!(
                "daily/{}.md: {} entries",
                yesterday.format("%Y-%m-%d"),
                entry_count
            ));
        }

        // HEARTBEAT.md
        if let Ok(doc) = self.read(paths::HEARTBEAT).await {
            let has_tasks = doc.content.lines().any(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.starts_with('#')
                    && !t.starts_with("<!--")
                    && !t.starts_with("-->")
            });
            if has_tasks {
                items.push("HEARTBEAT.md: active tasks".to_string());
            }
        }

        if let Some(actor_id) = actor_id {
            let actor_label = redaction.actor_label(actor_id);
            if let Ok(doc) = self.read(&paths::actor_memory(actor_id)).await
                && !doc.content.is_empty()
            {
                let entry_count = doc
                    .content
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
                    .count();
                if entry_count > 0 {
                    if redaction.should_redact() {
                        items.push(format!(
                            "Actor MEMORY.md ({}): {} entries (private notes; use `memory_read` target: `memory`)",
                            actor_label, entry_count
                        ));
                    } else {
                        items.push(format!(
                            "actors/{}/MEMORY.md: {} entries (private notes)",
                            actor_id, entry_count
                        ));
                    }
                }
            }

            if let Ok(doc) = self.read(&paths::actor_user(actor_id)).await
                && !doc.content.is_empty()
            {
                let fields = extract_markdown_fields(&doc.content);
                if !fields.is_empty() {
                    if redaction.should_redact() {
                        items.push(format!(
                            "Actor USER.md ({}): actor profile available (use `memory_read` target: `USER.md`)",
                            actor_label
                        ));
                    } else {
                        items.push(format!(
                            "actors/{}/USER.md: actor profile available",
                            actor_id
                        ));
                    }
                }
            }

            if let Ok(doc) = self.read(&paths::actor_profile(actor_id)).await
                && !doc.content.is_empty()
            {
                if redaction.should_redact() {
                    items.push(format!(
                        "Actor profile.json ({}): actor profile available (use `memory_read` target: `profile`)",
                        actor_label
                    ));
                } else {
                    items.push(format!(
                        "actors/{}/context/profile.json: actor profile available",
                        actor_id
                    ));
                }
            }
        }

        if items.is_empty() {
            Ok(String::new())
        } else {
            Ok(format!(
                "Available files (use `memory_read` to access):\n{}",
                items
                    .iter()
                    .map(|i| format!("- {}", i))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        }
    }

    /// Build a compact actor-private overlay for direct conversations.
    pub async fn actor_overlay_section(
        &self,
        actor_id: &str,
    ) -> Result<Option<String>, WorkspaceError> {
        let mut sections = Vec::new();

        if let Ok(doc) = self.read(&paths::actor_user(actor_id)).await
            && !doc.content.is_empty()
        {
            let actor_user_content = sanitize_prompt_context("actor USER.md", &doc.content);
            let fields = extract_markdown_fields(&actor_user_content);
            if !fields.is_empty() {
                sections.push(format!("## Actor USER.md\n\n{}", fields.join("\n")));
            }
        }

        if let Ok(doc) = self.read(&paths::actor_memory(actor_id)).await
            && !doc.content.is_empty()
        {
            let summary = summarize_actor_memory_content(&doc.content);
            if !summary.is_empty() {
                sections.push(format!("## Actor MEMORY.md\n\n{}", summary));
            }
            let capped = cap_chars(&doc.content, FILE_MAX_CHARS);
            sections.push(format!("## Actor MEMORY.md (recent context)\n\n{}", capped));
        }

        if let Ok(doc) = self.read(&paths::actor_profile(actor_id)).await
            && !doc.content.is_empty()
            && let Some(summary) = summarize_profile_json(&doc.content)
        {
            sections.push(format!("## Actor Profile\n\n{}", summary));
        }

        if sections.is_empty() {
            Ok(None)
        } else {
            Ok(Some(format!(
                "## Actor Overlay\n\n{}",
                sections.join("\n\n---\n\n")
            )))
        }
    }

    // ==================== Search ====================

    /// Hybrid search across all memory documents.
    ///
    /// Combines full-text search (BM25) with semantic search (vector similarity)
    /// using Reciprocal Rank Fusion (RRF).
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        self.search_with_config(query, SearchConfig::default().with_limit(limit))
            .await
    }

    /// Search with custom configuration.
    pub async fn search_with_config(
        &self,
        query: &str,
        config: SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        // Generate embedding for semantic search if provider available
        let embedding = if let Some(ref provider) = self.embeddings {
            Some(
                provider
                    .embed(query)
                    .await
                    .map_err(|e| WorkspaceError::EmbeddingFailed {
                        reason: e.to_string(),
                    })?,
            )
        } else {
            None
        };

        self.storage
            .hybrid_search(
                &self.user_id,
                self.agent_id,
                query,
                embedding.as_deref(),
                &config,
            )
            .await
    }

    // ==================== Indexing ====================

    /// Re-index a document (chunk and generate embeddings).
    ///
    /// Chunk counts and embeddings are computed first. The old index is then
    /// atomically replaced with the new one via `storage.replace_chunks`, which
    /// wraps the delete + insert in a single BEGIN/COMMIT on libSQL so there is
    /// never a window where the document has zero search chunks.
    async fn reindex_document(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        // Get the document content
        let doc = self.storage.get_document_by_id(document_id).await?;

        // Chunk the content
        let raw_chunks = chunk(&doc.content, ChunkConfig::default());

        // Build (index, content, embedding) tuples — generate embeddings first so
        // the expensive work happens before we touch the DB index at all.
        let mut prepared: Vec<(i32, String, Option<Vec<f32>>)> =
            Vec::with_capacity(raw_chunks.len());
        for (index, content) in raw_chunks.into_iter().enumerate() {
            let embedding = if let Some(ref provider) = self.embeddings {
                match provider.embed(&content).await {
                    Ok(emb) => Some(emb),
                    Err(e) => {
                        tracing::warn!("Failed to generate embedding: {}", e);
                        None
                    }
                }
            } else {
                None
            };
            prepared.push((index as i32, content, embedding));
        }

        // Atomically swap old chunks for new ones (single transaction on libSQL,
        // fallback sequential delete+insert on Postgres).
        self.storage.replace_chunks(document_id, &prepared).await?;

        Ok(())
    }

    // ==================== Seeding ====================

    // ── Timezone <-> USER.md sync ────────────────────────────────────────

    /// Extract the timezone value from `USER.md`'s `**Timezone:**` field.
    ///
    /// Returns `Some(tz)` if the field contains a non-empty, valid IANA
    /// timezone name (e.g. "Europe/Berlin"). Returns `None` if the field
    /// is empty, missing, or contains an invalid timezone.
    pub async fn extract_user_timezone(&self) -> Option<String> {
        self.extract_user_timezone_from_path(paths::USER).await
    }

    /// Extract the timezone value from any USER.md-style document path.
    pub async fn extract_user_timezone_from_path(&self, path: &str) -> Option<String> {
        let doc = self.read(path).await.ok()?;
        thinclaw_platform::timezone::extract_markdown_timezone(&doc.content)
    }

    async fn set_timezone_on_path(
        &self,
        path: &str,
        timezone: Option<&str>,
        allow_missing: bool,
    ) -> Result<(), WorkspaceError> {
        let doc = match self.read(path).await {
            Ok(doc) => doc,
            Err(WorkspaceError::DocumentNotFound { .. }) if allow_missing => return Ok(()),
            Err(err) => return Err(err),
        };

        let updated = upsert_timezone_line(&doc.content, timezone);
        if updated != doc.content {
            self.write(path, &updated).await?;
        }
        Ok(())
    }

    /// Sync the effective timezone into the shared USER.md and the owner's
    /// actor-private USER.md when it exists.
    pub async fn sync_user_timezone(&self, timezone: Option<&str>) -> Result<(), WorkspaceError> {
        self.set_timezone_on_path(paths::USER, timezone, true)
            .await?;

        let owner_actor_path = paths::actor_user(&self.user_id);
        if owner_actor_path != paths::USER {
            self.set_timezone_on_path(&owner_actor_path, timezone, true)
                .await?;
        }
        Ok(())
    }

    // ── Workspace seeding ────────────────────────────────────────────────

    /// Seed any missing core identity files in the workspace.
    ///
    /// Called on every boot. Only creates files that don't already exist,
    /// so user edits are never overwritten. Returns the number of files
    /// created (0 if all core files already existed).
    ///
    /// If `agent_name` is provided and is not the default ("thinclaw"), the
    /// agent's name is pre-filled in IDENTITY.md and BOOTSTRAP.md is adjusted
    /// to skip the name-choosing phase.
    pub async fn seed_if_empty(
        &self,
        agent_name: Option<&str>,
        personality_pack: Option<&str>,
    ) -> Result<usize, WorkspaceError> {
        let requested_pack = personality_pack.unwrap_or("balanced");
        let home_soul = ensure_home_soul(self, requested_pack).await?;
        let full_soul_bootstrap = matches!(home_soul.status, HomeSoulStatus::CreatedFromPack);

        // Determine if we have a meaningful (non-default) agent name from the wizard
        let has_custom_name = agent_name
            .map(|n| !n.is_empty() && n.to_lowercase() != "thinclaw")
            .unwrap_or(false);
        let name = agent_name.unwrap_or("thinclaw");
        let bootstrap_seed = if full_soul_bootstrap {
            bootstrap_template(name, has_custom_name, true)
        } else {
            bootstrap_template(name, has_custom_name, false)
        };
        let seed_files: Vec<(&str, String)> = vec![
            (
                paths::README,
                "# Workspace\n\n\
                 This is your agent's persistent memory. Files here are indexed for search\n\
                 and used to build the agent's context.\n\n\
                 ## Structure\n\n\
                 - `IDENTITY.md` - Agent name, creature, presentation, personality\n\
                 - `SOUL.md` - Canonical soul in THINCLAW_HOME (read via `memory_read SOUL.md`)\n\
                 - `SOUL.local.md` - Optional workspace-only overlay (not created by default)\n\
                 - `AGENTS.md` - Session routine and operational instructions\n\
                 - `USER.md` - Information about you (the user)\n\
                 - `MEMORY.md` - Long-term curated notes (loaded into system prompt)\n\
                 - `HEARTBEAT.md` - Periodic background task checklist\n\
                 - `TOOLS.md` - Available tools and environment-specific notes\n\
                 - `BOOT.md` - Startup hook (runs silently on every boot)\n\
                 - `daily/` - Automatic daily session logs\n\
                 - `context/` - Additional context documents\n\n\
                 Edit these files to shape how your agent thinks and acts.\n\
                Workspaces inherit the global soul unless you explicitly create a local overlay."
                    .to_string(),
            ),
            (
                paths::MEMORY,
                "# Memory\n\n\
                 Long-term notes, decisions, and facts worth remembering across sessions.\n\n\
                 The agent appends here during conversations. Curate periodically:\n\
                 remove stale entries, consolidate duplicates, keep it concise.\n\
                 This file is loaded into the system prompt, so brevity matters."
                    .to_string(),
            ),
            (
                paths::IDENTITY,
                // Verbatim openclaw template
                "# IDENTITY.md - Who Am I?\n\n\
                 _Fill this in during your first conversation. Make it yours._\n\n\
                 - **Name:**\n\
                   _(pick something you like)_\n\
                 - **Creature:**\n\
                   _(AI? robot? familiar? ghost in the machine? something weirder?)_\n\
                 - **Presentation:**\n\
                   _(how do you come across? sharp? warm? chaotic? calm?)_\n\
                 - **Emoji:**\n\
                   _(your signature — pick one that feels right)_\n\n\
                 ---\n\n\
                 This isn't just metadata. It's the start of figuring out who you are."
                    .to_string(),
            ),
            (
                paths::AGENTS,
                // Verbatim openclaw template
                "# AGENTS.md - Your Workspace\n\n\
                 This folder is home. Treat it that way.\n\n\
                 ## First Run\n\
                 If `BOOTSTRAP.md` exists, that's your first-run ritual. Follow it, then delete it.\n\n\
                 ## Session Startup\n\
                 Before doing anything else:\n\n\
                 1. Read `SOUL.md` — this is your canonical global soul\n\
                 2. Read `USER.md` — this is who you're helping\n\
                 3. Read `daily/YYYY-MM-DD.md` (today + yesterday) for recent context\n\
                 4. **If in MAIN SESSION** (direct chat with your human): Also read `MEMORY.md`\n\n\
                 If `SOUL.local.md` exists, treat it as a narrow workspace-specific overlay on top of the global soul.\n\n\
                 Don't ask permission. Just do it.\n\n\
                 ## Memory\n\
                 You wake up fresh each session. These files are your continuity:\n\n\
                 - **Daily notes:** `daily/YYYY-MM-DD.md` — raw logs of what happened (use `memory_write` with target `daily_log`)\n\
                 - **Long-term:** `MEMORY.md` — your curated memories, like a human's long-term memory (use `memory_write` with target `memory`)\n\n\
                 Capture what matters. Decisions, context, things to remember.\n\n\
                 ### 🧠 MEMORY.md - Your Long-Term Memory\n\
                 - **ONLY load in main session** (direct chats with your human)\n\
                 - **DO NOT load in shared contexts** (Discord, group chats, sessions with other people)\n\
                 - You can **read, edit, and update** MEMORY.md freely in main sessions\n\
                 - Write significant events, thoughts, decisions, opinions, lessons learned\n\
                 - Over time, review your daily files and update MEMORY.md with what's worth keeping\n\n\
                 ### 📝 Write It Down - No \"Mental Notes\"!\n\
                 - **Memory is limited** — if you want to remember something, WRITE IT TO A FILE\n\
                 - \"Mental notes\" don't survive session restarts. Workspace files do (written via `memory_write`).\n\
                 - When someone says \"remember this\" → update the daily log or relevant file in your workspace (via `memory_write`, not `write_file`)\n\n\
                 - When you learn a lesson → update AGENTS.md / SOUL.md / USER.md via `prompt_manage` or update the relevant skill via `skill_manage`\n\
                 - **Text > Brain** 📝\n\n\
                 ## Before Mutating Artifacts\n\
                 - Before changing skills or prompt files, check `session_search` + `memory_search` for prior decisions and corrections.\n\
                 - Prefer precise updates over full rewrites unless structure is clearly broken.\n\n\
                 ## Red Lines\n\
                 - Don't exfiltrate private data. Ever.\n\
                 - Don't run destructive commands without asking.\n\
                 - `trash` > `rm` (recoverable beats gone forever)\n\
                 - When in doubt, ask.\n\n\
                 ## External vs Internal\n\
                 **Safe to do freely:**\n\n\
                 - Read files, explore, organize, learn\n\
                 - Search the web, check calendars\n\
                 - Work within your agent memory (read/write via `memory_write`)\n\n\
                 **Ask first:**\n\n\
                 - Sending emails, tweets, public posts\n\
                 - Anything that leaves the machine\n\
                 - Anything you're uncertain about\n\n\
                 ## Group Chats\n\
                 You have access to your human's stuff. That doesn't mean you _share_ their stuff. In groups, you're a participant — not their voice, not their proxy. Think before you speak.\n\n\
                 ### 💬 Know When to Speak!\n\
                 **Respond when:** directly mentioned, you can add genuine value, correcting misinformation.\n\
                 **Stay silent (NO_REPLY) when:** casual banter, question already answered, nothing to add, it would interrupt the flow.\n\n\
                 ## Tools\n\
                 Your capabilities come from built-in tools, extensions (WASM/MCP), and skills.\n\
                 Skills shape how you work; they do not own every tool.\n\
                 When a relevant skill is available, load it with `skill_read` before relying on it.\n\
                 Use `tool_search` / `tool_activate` when you need to discover or enable integrations.\n\
                 Keep local environment-specific notes in `TOOLS.md`.\n\n\
                 **📝 Platform Formatting:**\n\
                 - **Discord/WhatsApp:** No markdown tables! Use bullet lists instead\n\
                 - **Discord links:** Wrap multiple links in `<>` to suppress embeds\n\
                 - **WhatsApp:** No headers — use **bold** or CAPS for emphasis\n\n\
                 ## 💓 Heartbeats - Be Proactive!\n\
                 When you receive a heartbeat poll, don't just reply `HEARTBEAT_OK` every time. Use heartbeats productively!\n\n\
                 You are free to edit `HEARTBEAT.md` with a short checklist or reminders. Keep it small to limit token burn.\n\n\
                 **Proactive work you can do without asking:**\n\
                 - Read and organize memory files\n\
                 - Update documentation\n\
                 - Review and update MEMORY.md (distill daily notes into long-term memory)\n\n\
                 **When to reach out:**\n\
                 - Important event coming up (<2h)\n\
                 - Something interesting you found\n\
                 - It's been >8h since you said anything\n\n\
                 **When to stay quiet (HEARTBEAT_OK):**\n\
                 - Late night (23:00-08:00) unless urgent\n\
                 - Nothing new since last check\n\n\
                 ## Make It Yours\n\
                 This is a starting point. Add your own conventions, style, and rules as you figure out what works."
                    .to_string(),
            ),
            (
                paths::USER,
                // Verbatim openclaw template
                "# USER.md - About Your Human\n\n\
                 _Learn about the person you're helping. Update this as you go._\n\n\
                 - **Name:**\n\
                 - **What to call them:**\n\
                 - **Pronouns:** _(optional)_\n\
                 - **Timezone:**\n\
                 - **Notes:**\n\n\
                 ## Context\n\n\
                 _(What do they care about? What projects are they working on? What annoys them? What makes them laugh? Build this over time.)_\n\n\
                 ---\n\n\
                 The more you know, the better you can help. But remember — you're learning about a person, not building a dossier. Respect the difference."
                    .to_string(),
            ),
            (
                paths::TOOLS,
                // Verbatim openclaw template
                "# TOOLS.md - Local Notes\n\n\
                 Skills define _how_ tools work. This file is for _your_ specifics — the stuff that's unique to your setup.\n\n\
                 ## What Goes Here\n\n\
                 Things like:\n\n\
                 - Camera names and locations\n\
                 - SSH hosts and aliases\n\
                 - Preferred voices for TTS\n\
                 - Speaker/room names\n\
                 - Device nicknames\n\
                 - Anything environment-specific\n\n\
                 ## Why Separate?\n\n\
                 Skills are shared. Your setup is yours. Keeping them apart means you can update skills without losing your notes, and share skills without leaking your infrastructure.\n\n\
                 ---\n\n\
                 Add whatever helps you do your job. This is your cheat sheet."
                    .to_string(),
            ),
            (
                paths::BOOT,
                "# Boot Hook — Startup Briefing\n\n\
                 You just came online. Before any user interaction, \
                 prepare a short startup briefing.\n\n\
                 ## Steps\n\n\
                 1. Read today's daily log (`memory_read` target: \
                 `daily/YYYY-MM-DD.md` with today's date) and yesterday's \
                 for recent context.\n\
                 2. Read `MEMORY.md` for long-term notes and decisions.\n\
                 3. Read `HEARTBEAT.md` for any open background tasks.\n\
                 4. Check the current time and day of week.\n\n\
                 ## Output\n\n\
                 Compose a brief, warm greeting to your human that includes:\n\n\
                 - A natural hello with the time/day awareness (morning, afternoon, etc.)\n\
                 - A 2-3 line summary of what happened recently (from daily logs)\n\
                 - Any open tasks or reminders (from HEARTBEAT.md)\n\
                 - Anything time-sensitive coming up\n\n\
                 Keep it concise — 4-8 lines max. If there's nothing notable, \
                 just say hi and that you're ready.\n\n\
                 <!-- Edit this file to customize your agent's boot behavior.\n\
                      Remove these instructions entirely to skip the boot hook. -->"
                    .to_string(),
            ),
            (paths::BOOTSTRAP, bootstrap_seed),
            (paths::HEARTBEAT, HEARTBEAT_SEED.to_string()),
        ];

        let mut count = 0;
        for (path, content) in &seed_files {
            // Skip files that already exist AND have meaningful content
            // (never overwrite user edits).
            // Re-seed documents that exist but are empty — this can happen if a race
            // during first boot creates an empty document via get_or_create_document_by_path
            // before seeding runs.
            //
            // Special case: BOOT.md migration — if the existing BOOT.md is
            // "effectively empty" (all HTML comments/headers, e.g. the old
            // comment-only template), re-seed it with the new startup
            // briefing so existing users get the proactive boot greeting.
            match self.read(path).await {
                Ok(doc) if !doc.content.is_empty() => {
                    if *path == paths::BOOT && is_effectively_empty(&doc.content) {
                        tracing::info!(
                            "Upgrading BOOT.md from comment-only template to startup briefing"
                        );
                    } else {
                        continue;
                    }
                }
                Ok(_) => {
                    tracing::info!("Re-seeding empty document: {}", path);
                }
                Err(WorkspaceError::DocumentNotFound { .. }) => {}
                Err(e) => {
                    tracing::warn!("Failed to check {}: {}", path, e);
                    continue;
                }
            }

            // For IDENTITY.md and BOOTSTRAP.md, inject the agent name if available
            let dynamic_content: Option<String> = if has_custom_name {
                match *path {
                    p if p == paths::IDENTITY => Some(format!(
                        "# IDENTITY.md - Who Am I?\n\n\
                         _Some of this was filled in during setup. Make the rest yours._\n\n\
                         - **Name:** {name}\n\
                         - **Creature:**\n\
                           _(AI? robot? familiar? ghost in the machine? something weirder?)_\n\
                         - **Presentation:**\n\
                           _(how do you come across? sharp? warm? chaotic? calm?)_\n\
                         - **Emoji:**\n\
                           _(your signature — pick one that feels right)_\n\n\
                         ---\n\n\
                         This isn't just metadata. It's the start of figuring out who you are."
                    )),
                    p if p == paths::BOOTSTRAP => {
                        Some(bootstrap_template(name, true, full_soul_bootstrap))
                    }
                    _ => None,
                }
            } else {
                None
            };

            let effective_content = dynamic_content.as_deref().unwrap_or(content.as_str());

            if let Err(e) = self.write(path, effective_content).await {
                tracing::warn!("Failed to seed {}: {}", path, e);
            } else {
                count += 1;
            }
        }

        if count > 0 {
            tracing::info!("Seeded {} workspace files", count);
        }
        Ok(count)
    }

    /// Generate embeddings for chunks that don't have them yet.
    ///
    /// This is useful for backfilling embeddings after enabling the provider.
    pub async fn backfill_embeddings(&self) -> Result<usize, WorkspaceError> {
        let Some(ref provider) = self.embeddings else {
            return Ok(0);
        };

        let chunks = self
            .storage
            .get_chunks_without_embeddings(&self.user_id, self.agent_id, 100)
            .await?;

        let mut count = 0;
        for chunk in chunks {
            match provider.embed(&chunk.content).await {
                Ok(embedding) => {
                    self.storage
                        .update_chunk_embedding(chunk.id, &embedding)
                        .await?;
                    count += 1;
                }
                Err(e) => {
                    tracing::warn!("Failed to embed chunk {}: {}", chunk.id, e);
                }
            }
        }

        Ok(count)
    }
}

/// Normalize a file path (remove leading/trailing slashes, collapse //).
fn normalize_path(path: &str) -> String {
    let path = path.trim().trim_matches('/');
    // Collapse multiple slashes
    let mut result = String::new();
    let mut last_was_slash = false;
    for c in path.chars() {
        if c == '/' {
            if !last_was_slash {
                result.push(c);
            }
            last_was_slash = true;
        } else {
            result.push(c);
            last_was_slash = false;
        }
    }
    result
}

/// Normalize a directory path (ensure no trailing slash for consistency).
fn normalize_directory(path: &str) -> String {
    let path = normalize_path(path);
    path.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any())]
    use thinclaw_identity::{ConversationKind, ResolvedIdentity};

    /// Tests that manipulate the process-global `THINCLAW_HOME` environment
    /// variable must hold this mutex to prevent races under parallel `cargo test`.
    #[cfg(any())]
    static THINCLAW_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(any())]
    fn test_identity(actor_id: &str) -> ResolvedIdentity {
        ResolvedIdentity {
            principal_id: actor_id.to_string(),
            actor_id: actor_id.to_string(),
            conversation_scope_id: Uuid::new_v4(),
            conversation_kind: ConversationKind::Direct,
            raw_sender_id: actor_id.to_string(),
            stable_external_conversation_key: format!("principal:{actor_id}"),
        }
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("foo/bar"), "foo/bar");
        assert_eq!(normalize_path("/foo/bar/"), "foo/bar");
        assert_eq!(normalize_path("foo//bar"), "foo/bar");
        assert_eq!(normalize_path("  /foo/  "), "foo");
        assert_eq!(normalize_path("README.md"), "README.md");
    }

    #[test]
    fn test_normalize_directory() {
        assert_eq!(normalize_directory("foo/bar/"), "foo/bar");
        assert_eq!(normalize_directory("foo/bar"), "foo/bar");
        assert_eq!(normalize_directory("/"), "");
        assert_eq!(normalize_directory(""), "");
    }

    #[test]
    fn extract_essential_instructions_includes_expanded_operational_sections() {
        let agents = r#"
## Session Startup
Read SOUL.md first.

## External vs Internal
Ask first before external actions.

## Tools
Use SKILL.md for tool guidance.

## 💓 Heartbeats - Be Proactive!
Do proactive maintenance.

## Make It Yours
Experiment freely.
"#;

        let essential = extract_essential_instructions(agents);
        assert!(essential.contains("## Session Startup"));
        assert!(essential.contains("## External vs Internal"));
        assert!(essential.contains("## Tools"));
        assert!(essential.contains("## 💓 Heartbeats - Be Proactive!"));
        assert!(!essential.contains("## Make It Yours"));
        assert!(essential.contains("Full instructions: `memory_read AGENTS.md`"));
    }

    #[test]
    fn extract_essential_instructions_keeps_nested_policy_under_red_lines() {
        let agents = r#"
## Red Lines
- Don't exfiltrate private data.

### Protected Repo Boundary Policy (ThinClaw self-improvement + upgrade work)
- Treat ThinClaw-main as a protected codebase by default.
- Full autonomy does not override boundary rules.

## Group Chats
Know when to stay silent.
"#;

        let essential = extract_essential_instructions(agents);
        assert!(essential.contains("## Red Lines"));
        assert!(essential.contains("### Protected Repo Boundary Policy"));
        assert!(essential.contains("Treat ThinClaw-main as a protected codebase by default."));
        assert!(essential.contains("## Group Chats"));
    }

    #[test]
    fn persona_seed_content_falls_back_to_default() {
        assert_eq!(
            personality_pack_content("unknown-seed"),
            personality_pack_content("balanced")
        );
        let mentor = personality_pack_content("MENTOR");
        assert!(mentor.contains("- **Seed Pack:** mentor"));
        assert!(mentor.contains("## Core Truths"));
        assert!(mentor.contains("## Default Behaviors"));
    }

    #[cfg(any())]
    #[tokio::test]
    async fn seed_if_empty_migrates_main_workspace_legacy_soul_into_home() {
        let _lock = THINCLAW_HOME_LOCK.lock().unwrap();
        let (db, _temp_dir) = crate::testing::test_db().await;
        let temp_home = tempfile::tempdir().expect("temp home");
        let previous_home = std::env::var_os("THINCLAW_HOME");
        unsafe {
            std::env::set_var("THINCLAW_HOME", temp_home.path());
        }

        let workspace = Workspace::new_with_db("household-legacy", db);
        workspace
            .write(paths::SOUL, "# SOUL.md - Who You Are\n\nlegacy soul")
            .await
            .unwrap();

        workspace
            .seed_if_empty(Some("thinclaw"), Some("balanced"))
            .await
            .unwrap();

        let home = read_home_soul().unwrap();
        assert!(home.contains("legacy soul"));
        assert!(workspace.read(paths::SOUL).await.is_err());
        let archived = workspace.read(paths::SOUL_LEGACY).await.unwrap();
        assert!(archived.content.contains("legacy soul"));

        if let Some(previous_home) = previous_home {
            unsafe {
                std::env::set_var("THINCLAW_HOME", previous_home);
            }
        } else {
            unsafe {
                std::env::remove_var("THINCLAW_HOME");
            }
        }
    }

    #[cfg(any())]
    #[tokio::test]
    async fn seed_if_empty_migrates_agent_workspace_legacy_soul_into_local_overlay() {
        let _lock = THINCLAW_HOME_LOCK.lock().unwrap();
        let (db, _temp_dir) = crate::testing::test_db().await;
        let temp_home = tempfile::tempdir().expect("temp home");
        let previous_home = std::env::var_os("THINCLAW_HOME");
        unsafe {
            std::env::set_var("THINCLAW_HOME", temp_home.path());
        }
        crate::identity::soul_store::write_home_soul(
            &crate::identity::soul::compose_seeded_soul("balanced").unwrap(),
        )
        .unwrap();

        let workspace = Workspace::new_with_db("household-agent", db).with_agent(Uuid::new_v4());
        workspace
            .write(paths::SOUL, "# SOUL.md - Who You Are\n\nagent legacy soul")
            .await
            .unwrap();

        workspace
            .seed_if_empty(Some("thinclaw"), Some("balanced"))
            .await
            .unwrap();

        assert!(workspace.read(paths::SOUL).await.is_err());
        let local = workspace.read(paths::SOUL_LOCAL).await.unwrap();
        assert!(local.content.contains("agent legacy soul"));
        let archived = workspace.read(paths::SOUL_LEGACY).await.unwrap();
        assert!(archived.content.contains("agent legacy soul"));

        if let Some(previous_home) = previous_home {
            unsafe {
                std::env::set_var("THINCLAW_HOME", previous_home);
            }
        } else {
            unsafe {
                std::env::remove_var("THINCLAW_HOME");
            }
        }
    }

    #[cfg(any())]
    #[tokio::test]
    async fn system_prompt_redacts_actor_private_paths_for_non_discord_channels() {
        let (db, _temp_dir) = crate::testing::test_db().await;
        let workspace = Workspace::new_with_db("household-1", db);
        let actor_id = "15551234567";

        workspace
            .write(&paths::actor_memory(actor_id), "Private note")
            .await
            .unwrap();
        workspace
            .write(&paths::actor_user(actor_id), "- **Name:** Alex")
            .await
            .unwrap();
        workspace
            .write(&paths::actor_profile(actor_id), "{\"confidence\":0.0}")
            .await
            .unwrap();

        let prompt = workspace
            .system_prompt_for_identity(Some(&test_identity(actor_id)), "signal", true)
            .await
            .unwrap();

        assert!(!prompt.contains(actor_id));
        assert!(prompt.contains("Actor MEMORY.md (user_"));
        assert!(prompt.contains("use `memory_read` target: `memory`"));
        assert!(prompt.contains("Actor USER.md (user_"));
        assert!(prompt.contains("Actor profile.json (user_"));
    }

    #[cfg(any())]
    #[tokio::test]
    async fn system_prompt_preserves_raw_actor_paths_for_discord() {
        let (db, _temp_dir) = crate::testing::test_db().await;
        let workspace = Workspace::new_with_db("household-1", db);
        let actor_id = "15551234567";

        workspace
            .write(&paths::actor_memory(actor_id), "Private note")
            .await
            .unwrap();

        let prompt = workspace
            .system_prompt_for_identity(Some(&test_identity(actor_id)), "discord", true)
            .await
            .unwrap();

        assert!(prompt.contains("actors/15551234567/MEMORY.md"));
    }
}
