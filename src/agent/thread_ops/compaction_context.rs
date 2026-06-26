//! Post-compaction context-fragment assembly and transient runtime cleanup.

use std::sync::Arc;

use uuid::Uuid;

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
    async fn collect_post_compaction_pinned_facts(
        &self,
        identity: Option<&ResolvedIdentity>,
    ) -> Vec<String> {
        const MAX_PINNED_FACTS: usize = 8;

        let Some(workspace) = self.workspace().cloned() else {
            return Vec::new();
        };

        let mut facts = PostCompactionFactAccumulator::new(MAX_PINNED_FACTS);
        let is_group = identity.is_some_and(|resolved| {
            matches!(
                resolved.conversation_kind,
                crate::identity::ConversationKind::Group
            )
        });

        if !is_group && let Some(actor_id) = identity.map(|resolved| resolved.actor_id.as_str()) {
            if let Ok(doc) = workspace.read(&paths::actor_user(actor_id)).await {
                let remaining = facts.remaining();
                facts.extend_source(
                    "Actor USER",
                    extract_markdown_field_facts(&doc.content, remaining),
                );
            }
            if let Ok(doc) = workspace.read(&paths::actor_profile(actor_id)).await {
                let remaining = facts.remaining();
                facts.extend_source(
                    "Actor profile",
                    extract_profile_facts(&doc.content, remaining),
                );
            }
            if let Ok(doc) = workspace.read(&paths::actor_memory(actor_id)).await {
                let remaining = facts.remaining();
                facts.extend_source(
                    "Actor memory",
                    extract_pinned_facts_from_markdown(&doc.content, remaining),
                );
            }
        }

        if let Ok(doc) = workspace.read(paths::USER).await {
            let remaining = facts.remaining();
            facts.extend_source(
                "USER.md",
                extract_markdown_field_facts(&doc.content, remaining),
            );
        }
        if let Ok(doc) = workspace.read(paths::PROFILE).await {
            let remaining = facts.remaining();
            facts.extend_source("Profile", extract_profile_facts(&doc.content, remaining));
        }
        if let Ok(doc) = workspace.read(paths::MEMORY).await {
            let remaining = facts.remaining();
            facts.extend_source(
                "Memory",
                extract_pinned_facts_from_markdown(&doc.content, remaining),
            );
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

    pub(in crate::agent) async fn update_post_compaction_context(
        &self,
        thread_id: Uuid,
        fragment: Option<String>,
    ) {
        let Some(store) = self.runtime_ports().threads.as_ref().map(Arc::clone) else {
            return;
        };

        if let Err(err) = thinclaw_agent::thread_ops::set_post_compaction_context(
            store.as_ref(),
            thread_id,
            fragment,
        )
        .await
        {
            tracing::debug!(
                thread = %thread_id,
                error = %err,
                "Failed to update post-compaction context"
            );
        }
    }

    pub(in crate::agent) async fn clear_thread_runtime_transients(&self, thread_id: Uuid) {
        let Some(store) = self.runtime_ports().threads.as_ref().map(Arc::clone) else {
            return;
        };

        if let Err(err) =
            thinclaw_agent::thread_ops::clear_thread_runtime_transients(store.as_ref(), thread_id)
                .await
        {
            tracing::debug!(
                thread = %thread_id,
                error = %err,
                "Failed to clear transient thread runtime state"
            );
        }
    }
}
