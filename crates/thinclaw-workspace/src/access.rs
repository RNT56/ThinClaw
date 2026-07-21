//! Enforced memory/knowledge authorization.
//!
//! A raw [`Workspace`](crate::Workspace) is a principal-scoped persistence
//! primitive used by trusted control-plane code. Model tools and user-facing
//! APIs must use [`AuthorizedWorkspace`] so actor and group boundaries are
//! enforced independently of prompt instructions.

use chrono::NaiveDate;
use thinclaw_identity::{AccessContext, ConversationKind, ResolvedIdentity};
use thinclaw_types::error::WorkspaceError;

use crate::{MemoryDocument, SearchConfig, SearchResult, Workspace, WorkspaceEntry, paths};

/// Whether a caller-relative path belongs to the trusted principal control
/// plane rather than conversational knowledge.
///
/// This allowlist is intentionally narrow: unknown documents are treated as
/// actor/conversation-authored evidence, never silently promoted into the
/// system-prompt namespace. Workspace hook and skill definitions are control
/// plane because loading them can change runtime behavior or execute code.
pub fn is_control_plane_path(requested: &str) -> bool {
    let Ok(path) = normalize_path(requested) else {
        return false;
    };
    matches!(
        path.as_str(),
        paths::SOUL
            | paths::SOUL_LOCAL
            | paths::SOUL_LEGACY
            | paths::AGENTS
            | paths::IDENTITY
            | paths::README
            | paths::BOOT
            | paths::BOOTSTRAP
            | paths::TOOLS
    ) || path == "hooks"
        || path.starts_with("hooks/")
        || path == "skills"
        || path.starts_with("skills/")
        || path == paths::ACTORS_DIR
        || path.starts_with(&format!("{}/", paths::ACTORS_DIR))
        || path == paths::CONVERSATIONS_DIR
        || path.starts_with(&format!("{}/", paths::CONVERSATIONS_DIR))
        || path == ".thinclaw"
        || path.starts_with(".thinclaw/")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceAccessRole {
    /// Normal conversational access. Direct messages are actor-private and
    /// groups are isolated to the canonical conversation scope.
    Conversation,
    /// Trusted principal administration. This may inspect or mutate every
    /// document inside the already principal-scoped workspace.
    PrincipalAdmin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceOperation {
    Read,
    Write,
    Delete,
    List,
    Search,
}

impl WorkspaceOperation {
    fn is_mutating(self) -> bool {
        matches!(self, Self::Write | Self::Delete)
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceAccess {
    context: AccessContext,
    role: WorkspaceAccessRole,
}

impl WorkspaceAccess {
    pub fn conversation(identity: &ResolvedIdentity, channel: impl Into<String>) -> Self {
        Self {
            context: identity.access_context(channel),
            role: WorkspaceAccessRole::Conversation,
        }
    }

    pub fn principal_admin(identity: &ResolvedIdentity, channel: impl Into<String>) -> Self {
        Self {
            context: identity.access_context(channel),
            role: WorkspaceAccessRole::PrincipalAdmin,
        }
    }

    pub fn context(&self) -> &AccessContext {
        &self.context
    }

    pub fn role(&self) -> WorkspaceAccessRole {
        self.role
    }

    pub fn primary_root(&self) -> String {
        match self.context.conversation_kind {
            ConversationKind::Direct => paths::actor_root(&self.context.actor_id),
            ConversationKind::Group => paths::conversation_root(self.context.conversation_scope_id),
        }
    }

    pub fn memory_path(&self) -> String {
        format!("{}/MEMORY.md", self.primary_root())
    }

    pub fn readable_roots(&self) -> Vec<String> {
        if self.role == WorkspaceAccessRole::PrincipalAdmin {
            Vec::new()
        } else {
            vec![self.primary_root(), paths::SHARED_DIR.to_string()]
        }
    }

    pub fn daily_path(&self, date: NaiveDate) -> String {
        format!(
            "{}/daily/{}.md",
            self.primary_root(),
            date.format("%Y-%m-%d")
        )
    }

    /// Resolve a model/API path into the canonical authorized namespace.
    /// Relative paths are always rooted inside the active actor/conversation.
    /// Attempts to name a different reserved namespace are denied rather than
    /// silently rewritten.
    pub fn resolve_path(
        &self,
        requested: &str,
        operation: WorkspaceOperation,
    ) -> Result<String, WorkspaceError> {
        let path = normalize_path(requested)?;
        if self.role == WorkspaceAccessRole::PrincipalAdmin {
            return Ok(path);
        }

        let root = self.primary_root();
        if path.is_empty() {
            return Ok(root);
        }

        let lower = path.to_ascii_lowercase();
        let alias = match lower.as_str() {
            "memory" | "memory.md" => Some(format!("{root}/MEMORY.md")),
            "user" | "user.md" if self.context.conversation_kind == ConversationKind::Direct => {
                Some(format!("{root}/USER.md"))
            }
            "profile" | "context/profile.json"
                if self.context.conversation_kind == ConversationKind::Direct =>
            {
                Some(format!("{root}/context/profile.json"))
            }
            "identity" | "identity.md"
                if self.context.conversation_kind == ConversationKind::Direct =>
            {
                Some(format!("{root}/IDENTITY.md"))
            }
            "heartbeat" | "heartbeat.md" => Some(format!("{root}/HEARTBEAT.md")),
            _ => None,
        };
        if let Some(alias) = alias {
            return Ok(alias);
        }

        if path == root || path.starts_with(&format!("{root}/")) {
            return Ok(path);
        }

        if path == paths::SHARED_DIR || path.starts_with(&format!("{}/", paths::SHARED_DIR)) {
            if operation.is_mutating() {
                return Err(denied(
                    requested,
                    "conversation contexts may read but not mutate principal-shared knowledge",
                ));
            }
            return Ok(path);
        }

        if is_reserved_namespace(&path) {
            return Err(denied(
                requested,
                "the requested path belongs to another actor, conversation, or control-plane namespace",
            ));
        }

        // Custom relative knowledge paths remain flexible, but are always
        // rooted below the active actor/conversation boundary.
        Ok(format!("{root}/{path}"))
    }

    pub fn can_read_canonical_path(&self, path: &str) -> bool {
        if self.role == WorkspaceAccessRole::PrincipalAdmin {
            return true;
        }
        let Ok(path) = normalize_path(path) else {
            return false;
        };
        let root = self.primary_root();
        path == root
            || path.starts_with(&format!("{root}/"))
            || path == paths::SHARED_DIR
            || path.starts_with(&format!("{}/", paths::SHARED_DIR))
    }

    /// Project a canonical storage path back into the caller-visible
    /// namespace. Conversation clients address their own root relatively
    /// (`MEMORY.md`, `daily/...`) and may see principal-shared knowledge under
    /// `shared/...`; they must not need to know or leak the internal
    /// `actors/<id>` / `conversations/<uuid>` prefix.
    pub fn display_path(&self, canonical: &str) -> Option<String> {
        let path = normalize_path(canonical).ok()?;
        if path == ".thinclaw" || path.starts_with(".thinclaw/") {
            return None;
        }
        if self.role == WorkspaceAccessRole::PrincipalAdmin {
            return Some(path);
        }

        let root = self.primary_root();
        if path == root {
            return Some(String::new());
        }
        if let Some(relative) = path.strip_prefix(&format!("{root}/")) {
            if relative == ".thinclaw" || relative.starts_with(".thinclaw/") {
                return None;
            }
            return Some(relative.to_string());
        }
        if path == paths::SHARED_DIR || path.starts_with(&format!("{}/", paths::SHARED_DIR)) {
            return Some(path);
        }
        None
    }
}

#[derive(Clone)]
pub struct AuthorizedWorkspace {
    workspace: Workspace,
    access: WorkspaceAccess,
}

impl AuthorizedWorkspace {
    pub fn new(base: &Workspace, access: WorkspaceAccess) -> Self {
        Self {
            workspace: base.scoped_clone(access.context.principal_id.clone(), base.agent_id()),
            access,
        }
    }

    pub fn conversation(
        base: &Workspace,
        identity: &ResolvedIdentity,
        channel: impl Into<String>,
    ) -> Self {
        Self::new(base, WorkspaceAccess::conversation(identity, channel))
    }

    pub fn principal_admin(
        base: &Workspace,
        identity: &ResolvedIdentity,
        channel: impl Into<String>,
    ) -> Self {
        Self::new(base, WorkspaceAccess::principal_admin(identity, channel))
    }

    pub fn access(&self) -> &WorkspaceAccess {
        &self.access
    }

    pub fn user_id(&self) -> &str {
        self.workspace.user_id()
    }

    pub fn agent_id(&self) -> Option<uuid::Uuid> {
        self.workspace.agent_id()
    }

    async fn ensure_legacy_owner_namespace(&self) -> Result<(), WorkspaceError> {
        let context = self.access.context();
        if self.access.role() == WorkspaceAccessRole::Conversation
            && context.conversation_kind == ConversationKind::Direct
            && context.actor_id == context.principal_id
        {
            self.workspace
                .migrate_legacy_owner_knowledge(&context.actor_id)
                .await?;
        }
        Ok(())
    }

    pub async fn read(&self, path: &str) -> Result<MemoryDocument, WorkspaceError> {
        let path = self.access.resolve_path(path, WorkspaceOperation::Read)?;
        self.ensure_legacy_owner_namespace().await?;
        self.workspace.read(&path).await
    }

    pub async fn write(&self, path: &str, content: &str) -> Result<MemoryDocument, WorkspaceError> {
        let path = self.access.resolve_path(path, WorkspaceOperation::Write)?;
        self.ensure_legacy_owner_namespace().await?;
        self.workspace.write(&path, content).await
    }

    pub async fn append(&self, path: &str, content: &str) -> Result<(), WorkspaceError> {
        let path = self.access.resolve_path(path, WorkspaceOperation::Write)?;
        self.ensure_legacy_owner_namespace().await?;
        self.workspace.append(&path, content).await
    }

    pub async fn append_with_separator(
        &self,
        path: &str,
        content: &str,
        separator: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        let path = self.access.resolve_path(path, WorkspaceOperation::Write)?;
        self.ensure_legacy_owner_namespace().await?;
        self.workspace
            .append_with_separator(&path, content, separator)
            .await
    }

    pub async fn delete(&self, path: &str) -> Result<(), WorkspaceError> {
        let path = self.access.resolve_path(path, WorkspaceOperation::Delete)?;
        self.ensure_legacy_owner_namespace().await?;
        self.workspace.delete(&path).await
    }

    pub async fn list(&self, path: &str) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let path = self.access.resolve_path(path, WorkspaceOperation::List)?;
        self.ensure_legacy_owner_namespace().await?;
        self.workspace.list(&path).await
    }

    pub async fn list_all(&self) -> Result<Vec<String>, WorkspaceError> {
        self.ensure_legacy_owner_namespace().await?;
        let paths = self.workspace.list_all().await?;
        Ok(paths
            .into_iter()
            .filter(|path| self.access.can_read_canonical_path(path))
            .collect())
    }

    pub async fn search(
        &self,
        query: &str,
        config: SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        self.ensure_legacy_owner_namespace().await?;
        let requested_limit = config.limit;
        let internal = config.with_path_prefixes(self.access.readable_roots());
        let mut results = self.workspace.search_with_config(query, internal).await?;
        // Defense in depth: backend predicates are authoritative for ranking,
        // while this final check protects against a future backend regression.
        results.retain(|result| self.access.can_read_canonical_path(&result.path));
        results.truncate(requested_limit);
        Ok(results)
    }

    pub async fn append_memory(&self, entry: &str) -> Result<(), WorkspaceError> {
        self.append_with_separator("MEMORY.md", entry, "\n\n")
            .await?;
        Ok(())
    }

    pub async fn append_daily_log(&self, entry: &str) -> Result<String, WorkspaceError> {
        self.ensure_legacy_owner_namespace().await?;
        let now = self
            .workspace
            .local_now_for_access(self.access.context())
            .await;
        let path = self.access.daily_path(now.date_naive());
        let timestamped = format!("[{}] {}", now.format("%H:%M:%S"), entry);
        self.workspace.append(&path, &timestamped).await?;
        Ok(path)
    }

    /// Resolve the local calendar day for this exact authorization scope.
    /// Direct actors may use an actor-private timezone; group scopes use the
    /// principal timezone so one shared timeline cannot change dates based on
    /// which participant happened to trigger the operation.
    pub async fn local_today(&self) -> NaiveDate {
        self.workspace
            .local_now_for_access(self.access.context())
            .await
            .date_naive()
    }

    /// Read (without creating) a daily log in the active actor/conversation
    /// namespace.
    pub async fn daily_log(&self, date: NaiveDate) -> Result<MemoryDocument, WorkspaceError> {
        self.ensure_legacy_owner_namespace().await?;
        let path = self.access.daily_path(date);
        self.workspace.read(&path).await
    }

    /// Read the active scope's heartbeat checklist. A missing checklist means
    /// the heartbeat is disabled for this scope; callers do not need the
    /// comment-only seed used by the trusted principal bootstrap path.
    pub async fn heartbeat_checklist(&self) -> Result<Option<String>, WorkspaceError> {
        match self.read("HEARTBEAT.md").await {
            Ok(document) => Ok(Some(document.content)),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Build the trusted control-plane prompt for this exact identity. Actor
    /// and conversation-authored memory remains outside this block and is
    /// available only as evidence or through authorized memory tools.
    pub async fn trusted_system_prompt(&self, redact_pii: bool) -> Result<String, WorkspaceError> {
        self.ensure_legacy_owner_namespace().await?;
        let context = self.access.context();
        self.workspace
            .system_prompt_for_context_details(
                context.conversation_kind == ConversationKind::Group,
                Some(context.actor_id.as_str()),
                Some(context.conversation_scope_id),
                Some(context.channel.as_str()),
                redact_pii,
            )
            .await
    }
}

fn normalize_path(path: &str) -> Result<String, WorkspaceError> {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    let mut parts = Vec::new();
    for component in trimmed.split('/').filter(|part| !part.is_empty()) {
        match component {
            "." => {}
            ".." => {
                return Err(denied(path, "parent-directory traversal is not allowed"));
            }
            component if component.contains('\0') => {
                return Err(denied(path, "NUL bytes are not allowed"));
            }
            component => parts.push(component),
        }
    }
    Ok(parts.join("/"))
}

fn is_reserved_namespace(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or_default();
    matches!(
        first,
        paths::ACTORS_DIR | paths::CONVERSATIONS_DIR | paths::SHARED_DIR
    ) || matches!(
        path,
        paths::SOUL
            | paths::SOUL_LOCAL
            | paths::SOUL_LEGACY
            | paths::AGENTS
            | paths::IDENTITY
            | paths::USER
            | paths::MEMORY
            | paths::PROFILE
            | paths::BOOT
            | paths::BOOTSTRAP
            | paths::HEARTBEAT
            | paths::TOOLS
    )
}

fn denied(path: &str, reason: &str) -> WorkspaceError {
    WorkspaceError::AccessDenied {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_identity::{
        ConversationKind, ResolvedIdentity, direct_scope_id, scope_id_from_key,
    };

    fn direct(actor: &str) -> ResolvedIdentity {
        ResolvedIdentity {
            principal_id: "house".to_string(),
            actor_id: actor.to_string(),
            conversation_scope_id: direct_scope_id("house", actor),
            conversation_kind: ConversationKind::Direct,
            raw_sender_id: actor.to_string(),
            stable_external_conversation_key: format!("direct:{actor}"),
        }
    }

    fn group() -> ResolvedIdentity {
        ResolvedIdentity {
            principal_id: "house".to_string(),
            actor_id: "alice".to_string(),
            conversation_scope_id: scope_id_from_key("group:house:room"),
            conversation_kind: ConversationKind::Group,
            raw_sender_id: "alice".to_string(),
            stable_external_conversation_key: "group:house:room".to_string(),
        }
    }

    #[test]
    fn direct_aliases_and_custom_paths_are_actor_private() {
        let access = WorkspaceAccess::conversation(&direct("alice"), "test");
        assert_eq!(
            access
                .resolve_path("MEMORY.md", WorkspaceOperation::Read)
                .unwrap(),
            "actors/alice/MEMORY.md"
        );
        assert_eq!(
            access
                .resolve_path("projects/p/notes.md", WorkspaceOperation::Write)
                .unwrap(),
            "actors/alice/projects/p/notes.md"
        );
    }

    #[test]
    fn actor_cannot_name_a_sibling_namespace() {
        let access = WorkspaceAccess::conversation(&direct("alice"), "test");
        assert!(
            access
                .resolve_path("actors/bob/MEMORY.md", WorkspaceOperation::Read)
                .is_err()
        );
    }

    #[test]
    fn groups_use_conversation_memory_and_never_actor_overlays() {
        let identity = group();
        let access = WorkspaceAccess::conversation(&identity, "test");
        assert_eq!(
            access
                .resolve_path("memory", WorkspaceOperation::Write)
                .unwrap(),
            format!("conversations/{}/MEMORY.md", identity.conversation_scope_id)
        );
        assert!(
            access
                .resolve_path("USER.md", WorkspaceOperation::Read)
                .is_err()
        );
    }

    #[test]
    fn compaction_daily_paths_follow_the_same_actor_and_group_boundaries() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let direct_access = WorkspaceAccess::conversation(&direct("alice"), "test");
        let group_identity = group();
        let group_access = WorkspaceAccess::conversation(&group_identity, "test");

        assert_eq!(
            direct_access.daily_path(date),
            "actors/alice/daily/2026-07-18.md"
        );
        assert_eq!(
            group_access.daily_path(date),
            format!(
                "conversations/{}/daily/2026-07-18.md",
                group_identity.conversation_scope_id
            )
        );
    }

    #[test]
    fn shared_knowledge_is_read_only_for_conversations() {
        let access = WorkspaceAccess::conversation(&direct("alice"), "test");
        assert_eq!(
            access
                .resolve_path("shared/household.md", WorkspaceOperation::Read)
                .unwrap(),
            "shared/household.md"
        );
        assert!(
            access
                .resolve_path("shared/household.md", WorkspaceOperation::Write)
                .is_err()
        );
    }

    #[test]
    fn admin_access_remains_principal_scoped_but_path_complete() {
        let access = WorkspaceAccess::principal_admin(&direct("alice"), "gateway");
        assert_eq!(
            access
                .resolve_path("actors/bob/MEMORY.md", WorkspaceOperation::Write)
                .unwrap(),
            "actors/bob/MEMORY.md"
        );
    }

    #[test]
    fn conversation_paths_are_projected_without_internal_identity_prefixes() {
        let direct = WorkspaceAccess::conversation(&direct("alice"), "gateway");
        assert_eq!(
            direct.display_path("actors/alice/daily/2026-07-18.md"),
            Some("daily/2026-07-18.md".to_string())
        );
        assert_eq!(
            direct.display_path("shared/household.md"),
            Some("shared/household.md".to_string())
        );
        assert_eq!(direct.display_path("actors/bob/MEMORY.md"), None);

        let group_identity = group();
        let group = WorkspaceAccess::conversation(&group_identity, "gateway");
        assert_eq!(
            group.display_path(&format!(
                "conversations/{}/MEMORY.md",
                group_identity.conversation_scope_id
            )),
            Some("MEMORY.md".to_string())
        );
    }

    #[test]
    fn control_plane_classifier_defaults_unknown_documents_to_untrusted_knowledge() {
        for path in [
            "SOUL.md",
            "IDENTITY.md",
            "hooks/redact.hook.json",
            "skills/research/SKILL.md",
            "actors/alice/MEMORY.md",
        ] {
            assert!(
                is_control_plane_path(path),
                "{path} should be control-plane"
            );
        }
        for path in [
            "MEMORY.md",
            "USER.md",
            "HEARTBEAT.md",
            "daily/2026-07-18.md",
            "notes.md",
            "shared/household.md",
        ] {
            assert!(
                !is_control_plane_path(path),
                "{path} should remain conversational evidence"
            );
        }
    }
}
