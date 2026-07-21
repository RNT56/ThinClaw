//! Post-compaction context-fragment assembly and transient runtime cleanup.

use crate::agent::Agent;
use crate::context::post_compaction::{
    ContextInjector, PostCompactionConfig, extract_markdown_field_facts,
    extract_pinned_facts_from_markdown, extract_profile_facts,
};
use crate::context::read_audit::{ReadAuditConfig, ReadAuditor};
use crate::identity::ResolvedIdentity;
use crate::workspace::paths;
use thinclaw_agent::thread_ops::PostCompactionFactAccumulator;

impl Agent {
    /// Context monitor whose window is derived from the active model instead
    /// of the fixed 100k default, using the synchronous model catalog (no
    /// network call). Falls back to the default monitor when the model is
    /// unknown. The threshold ratio is preserved.
    pub(in crate::agent) fn effective_context_monitor(
        &self,
    ) -> crate::agent::context_monitor::ContextMonitor {
        self.context_monitor_for_model(&self.llm().active_model_name())
    }

    /// Build a monitor for the model that will receive the next request. This
    /// is intentionally model-name based so smart routing and `llm_select`
    /// swaps can recompute the limit on every agentic-loop iteration.
    pub(in crate::agent) fn context_monitor_for_model(
        &self,
        model_name: &str,
    ) -> crate::agent::context_monitor::ContextMonitor {
        match thinclaw_config::model_compat::find_model(&model_name) {
            Some(model) if model.context_window > 0 => self
                .context_monitor
                .with_limit(model.context_window as usize),
            _ => self.context_monitor,
        }
    }

    async fn collect_post_compaction_pinned_facts(
        &self,
        identity: Option<&ResolvedIdentity>,
    ) -> Vec<String> {
        const MAX_PINNED_FACTS: usize = 8;

        let Some(base_workspace) = self.workspace().cloned() else {
            return Vec::new();
        };

        // The agent owns a backend handle, not a globally-authoritative user
        // scope.  Compaction can run after a turn arrived through any channel,
        // so always re-scope storage from the canonical ingress identity before
        // reading private facts.  Using the base workspace here previously made
        // post-compaction recall depend on whichever principal happened to be
        // used when the runtime was constructed (notably `default` on desktop).
        let workspace = identity
            .map(|resolved| {
                std::sync::Arc::new(
                    base_workspace
                        .scoped_clone(resolved.principal_id.clone(), base_workspace.agent_id()),
                )
            })
            .unwrap_or(base_workspace);

        let mut facts = PostCompactionFactAccumulator::new(MAX_PINNED_FACTS);
        let is_group = identity.is_some_and(|resolved| {
            matches!(
                resolved.conversation_kind,
                crate::identity::ConversationKind::Group
            )
        });

        let mut actor_user_exists = false;
        let mut actor_profile_exists = false;
        let mut actor_memory_exists = false;
        if !is_group && let Some(actor_id) = identity.map(|resolved| resolved.actor_id.as_str()) {
            if let Ok(doc) = workspace.read(&paths::actor_user(actor_id)).await {
                actor_user_exists = true;
                let remaining = facts.remaining();
                facts.extend_source(
                    "Actor USER",
                    extract_markdown_field_facts(&doc.content, remaining),
                );
            }
            if let Ok(doc) = workspace.read(&paths::actor_profile(actor_id)).await {
                actor_profile_exists = true;
                let remaining = facts.remaining();
                facts.extend_source(
                    "Actor profile",
                    extract_profile_facts(&doc.content, remaining),
                );
            }
            if let Ok(doc) = workspace.read(&paths::actor_memory(actor_id)).await {
                actor_memory_exists = true;
                let remaining = facts.remaining();
                facts.extend_source(
                    "Actor memory",
                    extract_pinned_facts_from_markdown(&doc.content, remaining),
                );
            }
        }

        let use_legacy_private_root = !is_group
            && identity.is_some_and(|resolved| resolved.actor_id == resolved.principal_id);
        if use_legacy_private_root {
            if !actor_user_exists && let Ok(doc) = workspace.read(paths::USER).await {
                let remaining = facts.remaining();
                facts.extend_source(
                    "USER.md",
                    extract_markdown_field_facts(&doc.content, remaining),
                );
            }
            if !actor_profile_exists && let Ok(doc) = workspace.read(paths::PROFILE).await {
                let remaining = facts.remaining();
                facts.extend_source("Profile", extract_profile_facts(&doc.content, remaining));
            }
            if !actor_memory_exists && let Ok(doc) = workspace.read(paths::MEMORY).await {
                let remaining = facts.remaining();
                facts.extend_source(
                    "Memory",
                    extract_pinned_facts_from_markdown(&doc.content, remaining),
                );
            }
        }

        facts.into_facts()
    }

    pub(in crate::agent) async fn build_post_compaction_context_fragment(
        &self,
        query: Option<&str>,
        identity: Option<&ResolvedIdentity>,
    ) -> Option<String> {
        let workspace_root = self
            .config
            .workspace_root
            .clone()
            .or_else(|| std::env::current_dir().ok())?;
        let root = workspace_root.to_string_lossy().to_string();
        let mut auditor = ReadAuditor::new(ReadAuditConfig::default());
        auditor.scan_rules(&root);
        let appendix = auditor.build_appendix();

        let mut injector = ContextInjector::new(PostCompactionConfig::from_env());
        if !appendix.trim().is_empty() {
            injector.add_rules(&appendix);
        }
        for fact in self.collect_post_compaction_pinned_facts(identity).await {
            injector.add_pinned_fact(&fact);
        }
        if let Some(query) = query.filter(|query| !query.trim().is_empty()) {
            let active_skills = self.select_active_skills(query, None).await;
            for skill in active_skills {
                let prompt_content = skill.prompt_content.trim();
                let context = if prompt_content.is_empty() {
                    skill.manifest.description.clone()
                } else {
                    format!("{}\n\n{}", skill.manifest.description, prompt_content)
                };
                injector.add_skill_context(skill.name(), &context);
            }
        }
        let injected = injector.build();
        if injected.trim().is_empty() {
            None
        } else {
            Some(injected)
        }
    }
}
