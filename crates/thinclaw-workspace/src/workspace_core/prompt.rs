//! System prompt assembly on [`Workspace`].
//!
//! Composes the trusted identity prompt from workspace-controlled files and
//! exposes actor/group-authored material separately as typed untrusted
//! evidence. Bootstrap (first-run) mode is handled here too.

use thinclaw_identity::{AccessContext, ConversationKind, ResolvedIdentity};

use thinclaw_types::error::WorkspaceError;

use super::Workspace;
use super::profile::summarize_profile_json;
use super::prompt_text::{
    FILE_MAX_CHARS, cap_chars, extract_essential_instructions, extract_markdown_fields,
    is_effectively_empty,
};
use super::redaction::{PromptRedaction, sanitize_prompt_context};
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
            Some(identity.conversation_scope_id),
            Some(channel),
            redact_pii,
        )
        .await
    }

    // ==================== System Prompt ====================

    /// Build the system prompt from identity files.
    ///
    /// Loads the canonical soul, workspace instructions, identity, and a
    /// scope-aware context manifest. Actor-authored profile and memory content
    /// is assembled separately by [`Self::untrusted_context_for_identity`].
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
    /// Actor/conversation knowledge remains accessible through `memory_read`;
    /// canonical control-plane prompt files stay runtime-managed.
    pub async fn system_prompt_for_context(
        &self,
        is_group_chat: bool,
    ) -> Result<String, WorkspaceError> {
        self.system_prompt_for_context_details(is_group_chat, None, None, None, false)
            .await
    }

    /// Build the trusted system prompt for an explicit conversation scope.
    pub async fn system_prompt_for_context_details(
        &self,
        is_group_chat: bool,
        actor_id: Option<&str>,
        conversation_scope_id: Option<uuid::Uuid>,
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
        let identity = self
            .compact_identity_for_prompt(actor_id, is_group_chat, redaction)
            .await?;
        if !identity.is_empty() {
            parts.push(format!("## Identity\n\n{}", identity));
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

        // 3. Context manifest (what's available, not the content itself).
        // Actor/group-authored content is intentionally loaded later through
        // `untrusted_context_for_identity` and compiled as evidence.
        {
            let manifest = self
                .context_manifest_for_prompt(
                    actor_id,
                    conversation_scope_id,
                    is_group_chat,
                    redaction,
                )
                .await?;
            if !manifest.is_empty() {
                parts.push(format!("## Context\n\n{}", manifest));
            }
        }

        Ok(parts.join("\n\n---\n\n"))
    }

    /// Load actor/group-authored workspace material as evidence rather than
    /// system authority. The prompt compiler transports this through a typed
    /// `UntrustedData` segment, preventing notes, profiles, or recalled text
    /// from overriding policy and tool permissions.
    pub async fn untrusted_context_for_identity(
        &self,
        identity: &ResolvedIdentity,
        channel: &str,
        redact_pii: bool,
    ) -> Result<Option<String>, WorkspaceError> {
        let redaction = PromptRedaction::new(Some(channel), redact_pii);
        match identity.conversation_kind {
            ConversationKind::Direct => {
                self.actor_overlay_section_for_prompt(&identity.actor_id, redaction)
                    .await
            }
            ConversationKind::Group => {
                self.conversation_overlay_section_for_prompt(
                    identity.conversation_scope_id,
                    redaction,
                )
                .await
            }
        }
    }

    /// Build a compressed identity block from workspace files.
    ///
    /// Extracts key fields from workspace-controlled IDENTITY.md.
    /// SOUL.md is injected separately as a dedicated prompt block.
    /// Full files remain accessible via `memory_read`.
    pub async fn compact_identity(&self) -> Result<String, WorkspaceError> {
        self.compact_identity_for_prompt(None, false, PromptRedaction::new(None, false))
            .await
    }

    async fn compact_identity_for_prompt(
        &self,
        _actor_id: Option<&str>,
        _is_group_chat: bool,
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

        // Pointer to full files
        if !lines.is_empty() {
            lines.push("Personal memory: `memory_read` with `path: MEMORY.md`".to_string());
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
        self.context_manifest_for_prompt(actor_id, None, false, PromptRedaction::new(None, false))
            .await
    }

    async fn context_manifest_for_prompt(
        &self,
        actor_id: Option<&str>,
        conversation_scope_id: Option<uuid::Uuid>,
        is_group_chat: bool,
        _redaction: PromptRedaction<'_>,
    ) -> Result<String, WorkspaceError> {
        let mut items = Vec::new();
        let root = if is_group_chat {
            conversation_scope_id.map(paths::conversation_root)
        } else {
            actor_id.map(paths::actor_root)
        };
        let legacy_fallback =
            !is_group_chat && actor_id.is_some_and(|actor| actor == self.user_id());

        let memory_path = root
            .as_ref()
            .map(|root| format!("{root}/MEMORY.md"))
            .unwrap_or_else(|| paths::MEMORY.to_string());
        let mut memory_doc = self.read(&memory_path).await.ok();
        if memory_doc.is_none() && legacy_fallback {
            memory_doc = self.read(paths::MEMORY).await.ok();
        }
        if let Some(doc) = memory_doc.filter(|doc| !doc.content.is_empty()) {
            let entry_count = doc
                .content
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
                .count();
            if entry_count > 0 {
                let scope_label = if is_group_chat { "group" } else { "private" };
                items.push(format!(
                    "MEMORY.md: {entry_count} entries ({scope_label} long-term notes; read with `path: MEMORY.md`)"
                ));
            }
        }

        let today = if let Some(actor_id) = actor_id {
            let context = AccessContext {
                principal_id: self.user_id().to_string(),
                actor_id: actor_id.to_string(),
                conversation_scope_id: conversation_scope_id.unwrap_or_else(uuid::Uuid::nil),
                conversation_kind: if is_group_chat {
                    ConversationKind::Group
                } else {
                    ConversationKind::Direct
                },
                channel: "workspace-prompt".to_string(),
            };
            self.local_now_for_access(&context).await.date_naive()
        } else {
            self.local_today()
        };
        let daily_path = root
            .as_ref()
            .map(|root| format!("{root}/daily/{}.md", today.format("%Y-%m-%d")))
            .unwrap_or_else(|| format!("daily/{}.md", today.format("%Y-%m-%d")));
        if let Ok(doc) = self.read(&daily_path).await
            && !doc.content.is_empty()
        {
            let entry_count = doc.content.lines().filter(|l| !l.trim().is_empty()).count();
            items.push(format!(
                "daily/{}.md: {} entries (today)",
                today.format("%Y-%m-%d"),
                entry_count
            ));
        }

        if let Some(yesterday) = today.pred_opt()
            && let Some(root) = root.as_ref()
            && let Ok(doc) = self
                .read(&format!("{root}/daily/{}.md", yesterday.format("%Y-%m-%d")))
                .await
            && !doc.content.is_empty()
        {
            let entry_count = doc.content.lines().filter(|l| !l.trim().is_empty()).count();
            items.push(format!(
                "daily/{}.md: {} entries",
                yesterday.format("%Y-%m-%d"),
                entry_count
            ));
        }

        let heartbeat_path = root
            .as_ref()
            .map(|root| format!("{root}/HEARTBEAT.md"))
            .unwrap_or_else(|| paths::HEARTBEAT.to_string());
        if let Ok(doc) = self.read(&heartbeat_path).await {
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

        if !is_group_chat && let Some(actor_id) = actor_id {
            let mut user_doc = self.read(&paths::actor_user(actor_id)).await.ok();
            if user_doc.is_none() && legacy_fallback {
                user_doc = self.read(paths::USER).await.ok();
            }
            if let Some(doc) = user_doc.filter(|doc| !doc.content.is_empty()) {
                let fields = extract_markdown_fields(&doc.content);
                if !fields.is_empty() {
                    items.push("USER.md: private actor profile (`path: USER.md`)".to_string());
                }
            }

            let mut profile_doc = self.read(&paths::actor_profile(actor_id)).await.ok();
            if profile_doc.is_none() && legacy_fallback {
                profile_doc = self.read(paths::PROFILE).await.ok();
            }
            if profile_doc.is_some_and(|doc| !doc.content.is_empty()) {
                items.push("profile: private actor profile (`path: profile`)".to_string());
            }
        }

        if self
            .list(paths::SHARED_DIR)
            .await
            .is_ok_and(|entries| !entries.is_empty())
        {
            items.push(
                "shared/: principal-shared knowledge (read-only in conversation tools)".to_string(),
            );
        }

        if items.is_empty() {
            Ok(String::new())
        } else {
            Ok(format!(
                "Available authorized files (use `memory_read` with a `path`):\n{}",
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
        let legacy_fallback = actor_id == self.user_id();

        let mut user_doc = self.read(&paths::actor_user(actor_id)).await.ok();
        if user_doc.is_none() && legacy_fallback {
            user_doc = self.read(paths::USER).await.ok();
        }
        if let Some(doc) = user_doc.filter(|doc| !doc.content.is_empty()) {
            let actor_user_content =
                sanitize_prompt_context("actor USER.md", &doc.content, redaction);
            let fields = extract_markdown_fields(&actor_user_content);
            if !fields.is_empty() {
                sections.push(format!("## Actor USER.md\n\n{}", fields.join("\n")));
            }
        }

        let actor_identity_path = Workspace::actor_path(actor_id, paths::IDENTITY);
        if let Ok(doc) = self.read(&actor_identity_path).await
            && !doc.content.is_empty()
        {
            let identity = sanitize_prompt_context("actor IDENTITY.md", &doc.content, redaction);
            let fields = extract_markdown_fields(&identity);
            if !fields.is_empty() {
                sections.push(format!("## Actor Identity\n\n{}", fields.join("\n")));
            }
        }

        let mut memory_doc = self.read(&paths::actor_memory(actor_id)).await.ok();
        if memory_doc.is_none() && legacy_fallback {
            memory_doc = self.read(paths::MEMORY).await.ok();
        }
        if let Some(doc) = memory_doc.filter(|doc| !doc.content.is_empty()) {
            let actor_memory_content =
                sanitize_prompt_context("actor MEMORY.md", &doc.content, redaction);
            let capped = cap_chars(&actor_memory_content, FILE_MAX_CHARS);
            sections.push(format!("## Actor Memory\n\n{}", capped));
        }

        let mut profile_doc = self.read(&paths::actor_profile(actor_id)).await.ok();
        if profile_doc.is_none() && legacy_fallback {
            profile_doc = self.read(paths::PROFILE).await.ok();
        }
        if let Some(doc) = profile_doc.filter(|doc| !doc.content.is_empty())
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

    /// Group memory is isolated to the canonical conversation scope. No actor
    /// USER/profile/memory data is consulted by this path.
    async fn conversation_overlay_section_for_prompt(
        &self,
        scope_id: uuid::Uuid,
        redaction: PromptRedaction<'_>,
    ) -> Result<Option<String>, WorkspaceError> {
        let path = paths::conversation_memory(scope_id);
        let Ok(doc) = self.read(&path).await else {
            return Ok(None);
        };
        if doc.content.trim().is_empty() {
            return Ok(None);
        }

        let content = sanitize_prompt_context("group MEMORY.md", &doc.content, redaction);
        Ok(Some(format!(
            "## Group Memory (this conversation only)\n\n{}",
            cap_chars(&content, FILE_MAX_CHARS)
        )))
    }
}
