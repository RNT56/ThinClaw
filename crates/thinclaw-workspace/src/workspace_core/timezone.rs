//! Timezone <-> `USER.md` synchronization on [`Workspace`].
//!
//! Extracts the IANA timezone from `USER.md`'s `**Timezone:**` field and keeps
//! the shared and actor-private `USER.md` documents in sync with the effective
//! timezone.

use chrono_tz::Tz;
use thinclaw_identity::{AccessContext, ConversationKind, ResolvedIdentity};
use thinclaw_types::error::WorkspaceError;

use super::Workspace;
use super::prompt_text::upsert_timezone_line;
use crate::document::paths;

impl Workspace {
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

    async fn timezone_from_user_document(&self, path: &str) -> Option<Tz> {
        self.extract_user_timezone_from_path(path)
            .await
            .and_then(|timezone| thinclaw_platform::timezone::parse_timezone(&timezone))
    }

    /// Resolve operational time for a canonical access scope. Direct actors
    /// may have their own USER.md timezone; group timelines deliberately use
    /// the principal/shared timezone so the same group log cannot switch dates
    /// depending on which participant wrote the next entry.
    pub async fn effective_timezone_for_access(&self, access: &AccessContext) -> Tz {
        if access.conversation_kind == ConversationKind::Direct
            && let Some(timezone) = self
                .timezone_from_user_document(&paths::actor_user(&access.actor_id))
                .await
        {
            return timezone;
        }
        if let Some(timezone) = self.timezone_from_user_document(paths::USER).await {
            return timezone;
        }
        self.effective_timezone()
    }

    pub async fn effective_timezone_for_identity(&self, identity: &ResolvedIdentity) -> Tz {
        self.effective_timezone_for_access(&identity.access_context("workspace"))
            .await
    }

    pub async fn local_now_for_access(&self, access: &AccessContext) -> chrono::DateTime<Tz> {
        chrono::Utc::now().with_timezone(&self.effective_timezone_for_access(access).await)
    }

    pub async fn local_today_for_identity(&self, identity: &ResolvedIdentity) -> chrono::NaiveDate {
        chrono::Utc::now()
            .with_timezone(&self.effective_timezone_for_identity(identity).await)
            .date_naive()
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
}
