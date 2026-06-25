//! System prompt assembly on [`Workspace`].
//!
//! Composes the lean identity prompt from workspace files: canonical soul
//! (+ optional local overlay), compact identity, distilled AGENTS.md
//! instructions, tiered user profile, actor-private overlay, linked recall,
//! and the context manifest. Bootstrap (first-run) mode is handled here too.

use thinclaw_identity::{ConversationKind, LinkedConversationRecall, ResolvedIdentity};

use thinclaw_types::error::WorkspaceError;

use super::Workspace;
use super::profile::summarize_profile_json;
use super::prompt_text::{
    FILE_MAX_CHARS, cap_chars, extract_essential_instructions, extract_markdown_fields,
    is_effectively_empty,
};
use super::redaction::{
    PromptRedaction, format_linked_recall, linked_recall_is_empty, sanitize_prompt_context,
    summarize_actor_memory_content,
};
use super::soul::read_home_soul;
use crate::document::paths;

impl Workspace {
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
            let mut bootstrap_prompt =
                sanitize_prompt_context(paths::BOOTSTRAP, &doc.content, redaction);

            if let Ok(home_soul) = read_home_soul()
                && !home_soul.trim().is_empty()
            {
                let soul_content = sanitize_prompt_context(paths::SOUL, &home_soul, redaction);
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
                let local_content =
                    sanitize_prompt_context(paths::SOUL_LOCAL, &local_soul.content, redaction);
                bootstrap_prompt.push_str("\n\n---\n\n");
                bootstrap_prompt.push_str("## This Workspace's Local Soul Overlay\n\n");
                bootstrap_prompt.push_str(&cap_chars(&local_content, FILE_MAX_CHARS));
            }

            // Append AGENTS.md so the agent knows its workspace conventions
            if let Ok(agents) = self.read(paths::AGENTS).await
                && !agents.content.is_empty()
            {
                let agents_content =
                    sanitize_prompt_context(paths::AGENTS, &agents.content, redaction);
                bootstrap_prompt.push_str("\n\n---\n\n");
                bootstrap_prompt.push_str("## Your Workspace Guide (operational reference)\n\n");
                bootstrap_prompt.push_str(&agents_content);
            }

            if let Some(actor_id) = actor_id
                && let Some(actor_overlay) = self
                    .actor_overlay_section_for_prompt(actor_id, redaction)
                    .await?
            {
                bootstrap_prompt.push_str("\n\n---\n\n");
                bootstrap_prompt.push_str(&actor_overlay);
            }

            if let Some(recall) = linked_recall
                && !linked_recall_is_empty(recall)
            {
                bootstrap_prompt.push_str("\n\n---\n\n");
                let linked = format_linked_recall(recall, redaction);
                bootstrap_prompt.push_str(&sanitize_prompt_context(
                    "linked recall",
                    &linked,
                    redaction,
                ));
            }

            return Ok(bootstrap_prompt);
        }

        // ── Normal mode: lean identity prompt ────────────────────────────
        let mut parts = Vec::new();

        if let Ok(home_soul) = read_home_soul()
            && !home_soul.trim().is_empty()
        {
            let soul_content = sanitize_prompt_context(paths::SOUL, &home_soul, redaction);
            let soul_block = thinclaw_soul::render_canonical_prompt_block(&soul_content);
            parts.push(cap_chars(&soul_block, FILE_MAX_CHARS));
        }

        if let Ok(local_soul) = self.read(paths::SOUL_LOCAL).await
            && !local_soul.content.is_empty()
        {
            let local_content =
                sanitize_prompt_context(paths::SOUL_LOCAL, &local_soul.content, redaction);
            if let Ok(local_block) = thinclaw_soul::render_local_prompt_block(&local_content) {
                parts.push(cap_chars(&local_block, FILE_MAX_CHARS / 2));
            }
        }

        // 1. Compact identity (name, nature, presentation, user info)
        let identity = self.compact_identity_for_prompt(redaction).await?;
        if !identity.is_empty() {
            parts.push(format!("## Identity\n\n{}", identity));
        }

        if !is_group_chat
            && let Some(actor_id) = actor_id
            && let Some(actor_overlay) = self
                .actor_overlay_section_for_prompt(actor_id, redaction)
                .await?
        {
            parts.push(actor_overlay);
        }

        // 2. Essential operational instructions (distilled from AGENTS.md)
        if let Ok(doc) = self.read(paths::AGENTS).await
            && !doc.content.is_empty()
        {
            let sanitized_agents = sanitize_prompt_context(paths::AGENTS, &doc.content, redaction);
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
            && let Some(summary) = summarize_profile_json(&doc.content)
        {
            let summary = sanitize_prompt_context(paths::PROFILE, &summary, redaction);
            parts.push(format!("## User Profile\n\n{}", summary));
        }

        if !is_group_chat
            && let Some(recall) = linked_recall
            && !linked_recall_is_empty(recall)
        {
            let linked = format_linked_recall(recall, redaction);
            parts.push(sanitize_prompt_context("linked recall", &linked, redaction));
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
        self.compact_identity_for_prompt(PromptRedaction::new(None, false))
            .await
    }

    async fn compact_identity_for_prompt(
        &self,
        redaction: PromptRedaction<'_>,
    ) -> Result<String, WorkspaceError> {
        let mut lines = Vec::new();

        // IDENTITY.md → extract filled key-value pairs
        if let Ok(doc) = self.read(paths::IDENTITY).await {
            let identity_content =
                sanitize_prompt_context(paths::IDENTITY, &doc.content, redaction);
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
            let user_content = sanitize_prompt_context(paths::USER, &doc.content, redaction);
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
        self.actor_overlay_section_for_prompt(actor_id, PromptRedaction::new(None, false))
            .await
    }

    async fn actor_overlay_section_for_prompt(
        &self,
        actor_id: &str,
        redaction: PromptRedaction<'_>,
    ) -> Result<Option<String>, WorkspaceError> {
        let mut sections = Vec::new();

        if let Ok(doc) = self.read(&paths::actor_user(actor_id)).await
            && !doc.content.is_empty()
        {
            let actor_user_content =
                sanitize_prompt_context("actor USER.md", &doc.content, redaction);
            let fields = extract_markdown_fields(&actor_user_content);
            if !fields.is_empty() {
                sections.push(format!("## Actor USER.md\n\n{}", fields.join("\n")));
            }
        }

        if let Ok(doc) = self.read(&paths::actor_memory(actor_id)).await
            && !doc.content.is_empty()
        {
            let actor_memory_content =
                sanitize_prompt_context("actor MEMORY.md", &doc.content, redaction);
            let summary = summarize_actor_memory_content(&actor_memory_content);
            if !summary.is_empty() {
                sections.push(format!("## Actor MEMORY.md\n\n{}", summary));
            }
            let capped = cap_chars(&actor_memory_content, FILE_MAX_CHARS);
            sections.push(format!("## Actor MEMORY.md (recent context)\n\n{}", capped));
        }

        if let Ok(doc) = self.read(&paths::actor_profile(actor_id)).await
            && !doc.content.is_empty()
            && let Some(summary) = summarize_profile_json(&doc.content)
        {
            let summary = sanitize_prompt_context("actor profile.json", &summary, redaction);
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
}
