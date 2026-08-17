use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use thinclaw_channels_core::OutgoingResponse;
use thinclaw_types::MediaContent;

const STATE_VERSION: u8 = 2;
const MAX_STATE_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_INBOX_EVENTS: usize = 256;
const MAX_OUTBOX_OPERATIONS: usize = 256;
const RETENTION_SECS: i64 = 7 * 24 * 60 * 60;
const MAX_RETRY_DELAY_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AcceptResult {
    Accepted,
    AlreadyAccepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkPhase {
    Pending,
    Processing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct InboxWork {
    pub event_id: String,
    pub document: Value,
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InboxRecord {
    document: Value,
    accepted_at: i64,
    attempts: u32,
    next_attempt_at: i64,
    phase: WorkPhase,
    #[serde(default)]
    last_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum OutboundTarget {
    ExistingChat {
        chat_id: String,
        reply_to: Option<String>,
        recipient: Option<String>,
    },
    PooledRecipient {
        recipient: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredMedia {
    data_base64: String,
    mime_type: String,
    filename: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StoredResponse {
    pub content: String,
    pub thread_id: Option<String>,
    pub metadata: Value,
    attachments: Vec<StoredMedia>,
}

impl StoredResponse {
    pub fn from_response(response: &OutgoingResponse) -> Self {
        Self {
            content: response.content.clone(),
            thread_id: response.thread_id.clone(),
            metadata: response.metadata.clone(),
            attachments: response
                .attachments
                .iter()
                .map(|media| StoredMedia {
                    data_base64: BASE64.encode(&media.data),
                    mime_type: media.mime_type.clone(),
                    filename: media.filename.clone(),
                })
                .collect(),
        }
    }

    pub fn to_response(&self, delivery_id: uuid::Uuid) -> Result<OutgoingResponse, String> {
        let attachments = self
            .attachments
            .iter()
            .map(|stored| {
                let bytes = BASE64
                    .decode(&stored.data_base64)
                    .map_err(|_| "outbox attachment encoding is invalid".to_string())?;
                let mut media = MediaContent::new(bytes, stored.mime_type.clone());
                media.filename.clone_from(&stored.filename);
                Ok(media)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(OutgoingResponse {
            delivery_id,
            content: self.content.clone(),
            thread_id: self.thread_id.clone(),
            metadata: self.metadata.clone(),
            attachments,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct OutboxWork {
    pub operation_id: String,
    pub target: OutboundTarget,
    pub response: StoredResponse,
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutboxRecord {
    target: OutboundTarget,
    response: StoredResponse,
    created_at: i64,
    attempts: u32,
    next_attempt_at: i64,
    phase: WorkPhase,
    #[serde(default)]
    last_error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum DeliveryHealth {
    Unknown,
    Healthy,
    AtRisk,
    Critical,
    OptedOut,
}

impl DeliveryHealth {
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or_default().trim().to_ascii_uppercase().as_str() {
            "HEALTHY" | "ACTIVE" | "VERIFIED" => Self::Healthy,
            "AT_RISK" | "DEGRADED" | "FLAGGED" => Self::AtRisk,
            "CRITICAL" | "INACTIVE" | "SUSPENDED" | "DISABLED" => Self::Critical,
            "OPTED_OUT" => Self::OptedOut,
            _ => Self::Unknown,
        }
    }

    pub const fn blocks_delivery(self) -> bool {
        matches!(Self::AtRisk | Self::Critical | Self::OptedOut, self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthRecord {
    status: DeliveryHealth,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatRecord {
    status: DeliveryHealth,
    owner_line: String,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecipientRecord {
    opted_out: bool,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StateFile {
    version: u8,
    #[serde(default)]
    completed_events: BTreeMap<String, i64>,
    #[serde(default)]
    inbox: BTreeMap<String, InboxRecord>,
    #[serde(default)]
    completed_outbox: BTreeMap<String, i64>,
    #[serde(default)]
    outbox: BTreeMap<String, OutboxRecord>,
    #[serde(default)]
    recipients: BTreeMap<String, RecipientRecord>,
    #[serde(default)]
    chats: BTreeMap<String, ChatRecord>,
    #[serde(default)]
    lines: BTreeMap<String, HealthRecord>,
}

impl Default for StateFile {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            completed_events: BTreeMap::new(),
            inbox: BTreeMap::new(),
            completed_outbox: BTreeMap::new(),
            outbox: BTreeMap::new(),
            recipients: BTreeMap::new(),
            chats: BTreeMap::new(),
            lines: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct LegacyLedger {
    version: u8,
    events: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StateStats {
    pub pending_inbox: usize,
    pub completed_inbox: usize,
    pub pending_outbox: usize,
    pub completed_outbox: usize,
    pub opted_out_recipients: usize,
    pub blocked_chats: usize,
    pub blocked_lines: usize,
}

pub(super) struct DurableState {
    path: PathBuf,
    inner: Mutex<StateFile>,
}

impl DurableState {
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "state file has no parent directory".to_string())?;
        thinclaw_platform::ensure_private_directory(parent)
            .map_err(|error| format!("state directory is unavailable: {error}"))?;
        let mut state = match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                thinclaw_platform::harden_private_regular_file(&path)
                    .map_err(|error| format!("state file is unsafe: {error}"))?;
                let bytes = thinclaw_platform::read_regular_file_bounded_single_link(
                    &path,
                    MAX_STATE_FILE_BYTES,
                )
                .map_err(|error| format!("state file is unreadable: {error}"))?;
                decode_state(&bytes)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => StateFile::default(),
            Err(error) => return Err(format!("state metadata is unavailable: {error}")),
        };
        // A process can die while holding a claim. Claims are process-local;
        // recovery always makes them eligible again with the same stable ID.
        for record in state.inbox.values_mut() {
            record.phase = WorkPhase::Pending;
            record.next_attempt_at = 0;
        }
        for record in state.outbox.values_mut() {
            record.phase = WorkPhase::Pending;
            record.next_attempt_at = 0;
        }
        prune(&mut state, Utc::now().timestamp());
        Ok(Self {
            path,
            inner: Mutex::new(state),
        })
    }

    pub fn load_with_legacy(path: PathBuf, legacy_path: Option<&Path>) -> Result<Self, String> {
        if !path.exists()
            && let Some(legacy_path) = legacy_path
            && legacy_path.exists()
        {
            let bytes = thinclaw_platform::read_regular_file_bounded_single_link(
                legacy_path,
                MAX_STATE_FILE_BYTES,
            )
            .map_err(|error| format!("legacy state is unreadable: {error}"))?;
            let state = decode_state(&bytes)?;
            let encoded = serde_json::to_vec(&state)
                .map_err(|error| format!("legacy state serialization failed: {error}"))?;
            let parent = path
                .parent()
                .ok_or_else(|| "state file has no parent directory".to_string())?;
            thinclaw_platform::ensure_private_directory(parent)
                .map_err(|error| format!("state directory is unavailable: {error}"))?;
            thinclaw_platform::write_private_file_atomic(&path, &encoded, true)
                .map_err(|error| format!("legacy state migration failed: {error}"))?;
        }
        Self::load(path)
    }

    pub async fn accept_inbox(
        &self,
        event_id: &str,
        document: Value,
    ) -> Result<AcceptResult, String> {
        let mut state = self.inner.lock().await;
        if state.completed_events.contains_key(event_id) || state.inbox.contains_key(event_id) {
            return Ok(AcceptResult::AlreadyAccepted);
        }
        if state.inbox.len() >= MAX_INBOX_EVENTS {
            return Err("inbox_capacity".to_string());
        }
        let before = state.clone();
        state.inbox.insert(
            event_id.to_string(),
            InboxRecord {
                document,
                accepted_at: Utc::now().timestamp(),
                attempts: 0,
                next_attempt_at: 0,
                phase: WorkPhase::Pending,
                last_error_code: None,
            },
        );
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(AcceptResult::Accepted)
    }

    pub async fn claim_inbox(&self) -> Result<Option<InboxWork>, String> {
        let now = Utc::now().timestamp();
        let mut state = self.inner.lock().await;
        let event_id = state
            .inbox
            .iter()
            .filter(|(_, record)| {
                record.phase == WorkPhase::Pending && record.next_attempt_at <= now
            })
            .min_by_key(|(_, record)| record.accepted_at)
            .map(|(event_id, _)| event_id.clone());
        let Some(event_id) = event_id else {
            return Ok(None);
        };
        let before = state.clone();
        let record = state.inbox.get_mut(&event_id).expect("selected inbox row");
        record.phase = WorkPhase::Processing;
        let work = InboxWork {
            event_id,
            document: record.document.clone(),
            attempts: record.attempts,
        };
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(Some(work))
    }

    pub async fn complete_inbox(&self, event_id: &str) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let before = state.clone();
        state.inbox.remove(event_id);
        state
            .completed_events
            .insert(event_id.to_string(), Utc::now().timestamp());
        prune(&mut state, Utc::now().timestamp());
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    pub async fn retry_inbox(&self, event_id: &str, error_code: &str) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let before = state.clone();
        if let Some(record) = state.inbox.get_mut(event_id) {
            record.phase = WorkPhase::Pending;
            record.attempts = record.attempts.saturating_add(1);
            record.next_attempt_at = Utc::now().timestamp()
                + retry_delay_seconds(record.attempts);
            record.last_error_code = Some(categorical(error_code));
        }
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    pub async fn enqueue_outbox(
        &self,
        operation_id: &str,
        target: OutboundTarget,
        response: StoredResponse,
    ) -> Result<AcceptResult, String> {
        let mut state = self.inner.lock().await;
        if state.completed_outbox.contains_key(operation_id) {
            return Ok(AcceptResult::AlreadyAccepted);
        }
        if let Some(existing) = state.outbox.get(operation_id) {
            if existing.target != target || existing.response != response {
                return Err("delivery_id_collision".to_string());
            }
            return Ok(AcceptResult::AlreadyAccepted);
        }
        if state.outbox.len() >= MAX_OUTBOX_OPERATIONS {
            return Err("outbox_capacity".to_string());
        }
        let before = state.clone();
        state.outbox.insert(
            operation_id.to_string(),
            OutboxRecord {
                target,
                response,
                created_at: Utc::now().timestamp(),
                attempts: 0,
                next_attempt_at: 0,
                phase: WorkPhase::Pending,
                last_error_code: None,
            },
        );
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(AcceptResult::Accepted)
    }

    pub async fn claim_outbox(&self, operation_id: &str) -> Result<Option<OutboxWork>, String> {
        let now = Utc::now().timestamp();
        let mut state = self.inner.lock().await;
        let Some(record) = state.outbox.get(operation_id) else {
            return Ok(None);
        };
        if record.phase != WorkPhase::Pending || record.next_attempt_at > now {
            return Ok(None);
        }
        let before = state.clone();
        let record = state
            .outbox
            .get_mut(operation_id)
            .expect("selected outbox row");
        record.phase = WorkPhase::Processing;
        let work = OutboxWork {
            operation_id: operation_id.to_string(),
            target: record.target.clone(),
            response: record.response.clone(),
            attempts: record.attempts,
        };
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(Some(work))
    }

    pub async fn claim_next_outbox(&self) -> Result<Option<OutboxWork>, String> {
        let now = Utc::now().timestamp();
        let operation_id = {
            let state = self.inner.lock().await;
            state
                .outbox
                .iter()
                .filter(|(_, record)| {
                    record.phase == WorkPhase::Pending && record.next_attempt_at <= now
                })
                .min_by_key(|(_, record)| record.created_at)
                .map(|(operation_id, _)| operation_id.clone())
        };
        match operation_id {
            Some(operation_id) => self.claim_outbox(&operation_id).await,
            None => Ok(None),
        }
    }

    pub async fn complete_outbox(&self, operation_id: &str) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let before = state.clone();
        state.outbox.remove(operation_id);
        state
            .completed_outbox
            .insert(operation_id.to_string(), Utc::now().timestamp());
        prune(&mut state, Utc::now().timestamp());
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    pub async fn retry_outbox(
        &self,
        operation_id: &str,
        error_code: &str,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let before = state.clone();
        if let Some(record) = state.outbox.get_mut(operation_id) {
            record.phase = WorkPhase::Pending;
            record.attempts = record.attempts.saturating_add(1);
            record.next_attempt_at = Utc::now().timestamp()
                + retry_delay_seconds(record.attempts);
            record.last_error_code = Some(categorical(error_code));
        }
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    pub async fn observe_inbound(
        &self,
        chat_id: &str,
        recipient: &str,
        owner_line: &str,
        health: DeliveryHealth,
        opted_out: bool,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let before = state.clone();
        let now = Utc::now().timestamp();
        state.chats.insert(
            chat_id.to_string(),
            ChatRecord {
                status: health,
                owner_line: owner_line.to_string(),
                updated_at: now,
            },
        );
        state.recipients.insert(
            recipient.to_string(),
            RecipientRecord {
                opted_out: opted_out || health == DeliveryHealth::OptedOut,
                updated_at: now,
            },
        );
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    pub async fn mark_opted_out(
        &self,
        chat_id: Option<&str>,
        recipient: Option<&str>,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let before = state.clone();
        let now = Utc::now().timestamp();
        if let Some(chat_id) = chat_id {
            let owner_line = state
                .chats
                .get(chat_id)
                .map_or_else(String::new, |record| record.owner_line.clone());
            state.chats.insert(
                chat_id.to_string(),
                ChatRecord {
                    status: DeliveryHealth::OptedOut,
                    owner_line,
                    updated_at: now,
                },
            );
        }
        if let Some(recipient) = recipient {
            state.recipients.insert(
                recipient.to_string(),
                RecipientRecord {
                    opted_out: true,
                    updated_at: now,
                },
            );
        }
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    pub async fn update_line_health(
        &self,
        line: &str,
        health: DeliveryHealth,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let before = state.clone();
        state.lines.insert(
            line.to_string(),
            HealthRecord {
                status: health,
                updated_at: Utc::now().timestamp(),
            },
        );
        if let Err(error) = persist(&self.path, &state).await {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    pub async fn delivery_block_reason(&self, target: &OutboundTarget) -> Option<&'static str> {
        let state = self.inner.lock().await;
        match target {
            OutboundTarget::ExistingChat {
                chat_id,
                recipient,
                ..
            } => {
                if let Some(chat) = state.chats.get(chat_id) {
                    if chat.status.blocks_delivery() {
                        return Some("chat_delivery_blocked");
                    }
                    if state
                        .lines
                        .get(&chat.owner_line)
                        .is_some_and(|record| record.status.blocks_delivery())
                    {
                        return Some("managed_line_unhealthy");
                    }
                }
                if recipient.as_ref().is_some_and(|recipient| {
                    state
                        .recipients
                        .get(recipient)
                        .is_some_and(|record| record.opted_out)
                }) {
                    return Some("recipient_opted_out");
                }
            }
            OutboundTarget::PooledRecipient { recipient } => {
                if state
                    .recipients
                    .get(recipient)
                    .is_some_and(|record| record.opted_out)
                {
                    return Some("recipient_opted_out");
                }
            }
        }
        None
    }

    pub async fn stats(&self) -> StateStats {
        let state = self.inner.lock().await;
        StateStats {
            pending_inbox: state.inbox.len(),
            completed_inbox: state.completed_events.len(),
            pending_outbox: state.outbox.len(),
            completed_outbox: state.completed_outbox.len(),
            opted_out_recipients: state
                .recipients
                .values()
                .filter(|record| record.opted_out)
                .count(),
            blocked_chats: state
                .chats
                .values()
                .filter(|record| record.status.blocks_delivery())
                .count(),
            blocked_lines: state
                .lines
                .values()
                .filter(|record| record.status.blocks_delivery())
                .count(),
        }
    }

    pub async fn outbox_completed(&self, operation_id: &str) -> bool {
        self.inner
            .lock()
            .await
            .completed_outbox
            .contains_key(operation_id)
    }

    #[cfg(test)]
    pub async fn make_inbox_retry_ready(&self, event_id: &str) {
        if let Some(record) = self.inner.lock().await.inbox.get_mut(event_id) {
            record.next_attempt_at = 0;
        }
    }

    #[cfg(test)]
    pub async fn make_outbox_retry_ready(&self, operation_id: &str) {
        if let Some(record) = self.inner.lock().await.outbox.get_mut(operation_id) {
            record.next_attempt_at = 0;
        }
    }

    #[cfg(test)]
    pub async fn has_pending_inbox(&self, event_id: &str) -> bool {
        self.inner.lock().await.inbox.contains_key(event_id)
    }

    #[cfg(test)]
    pub async fn has_pending_outbox(&self, operation_id: &str) -> bool {
        self.inner.lock().await.outbox.contains_key(operation_id)
    }
}

fn decode_state(bytes: &[u8]) -> Result<StateFile, String> {
    let version = serde_json::from_slice::<Value>(bytes)
        .map_err(|error| format!("state file is malformed: {error}"))?
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "state file version is missing".to_string())?;
    match version {
        1 => {
            let legacy: LegacyLedger = serde_json::from_slice(bytes)
                .map_err(|error| format!("legacy event ledger is malformed: {error}"))?;
            if legacy.version != 1 {
                return Err("legacy event ledger version is invalid".to_string());
            }
            Ok(StateFile {
                completed_events: legacy.events,
                ..StateFile::default()
            })
        }
        2 => serde_json::from_slice(bytes)
            .map_err(|error| format!("state file is malformed: {error}")),
        _ => Err("state file version is unsupported".to_string()),
    }
}

async fn persist(path: &Path, state: &StateFile) -> Result<(), String> {
    let bytes = serde_json::to_vec(state)
        .map_err(|error| format!("state serialization failed: {error}"))?;
    if bytes.len() as u64 > MAX_STATE_FILE_BYTES {
        return Err("state_capacity".to_string());
    }
    thinclaw_platform::write_private_file_atomic_async(path.to_path_buf(), bytes, true)
        .await
        .map_err(|error| format!("state_write_failed:{:?}", error.kind()))
}

fn retry_delay_seconds(attempts: u32) -> i64 {
    let shift = attempts.min(8);
    (1_i64 << shift).min(MAX_RETRY_DELAY_SECS)
}

fn categorical(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_lowercase() || *character == '_')
        .take(48)
        .collect()
}

fn prune(state: &mut StateFile, now: i64) {
    state
        .completed_events
        .retain(|_, timestamp| now.saturating_sub(*timestamp) <= RETENTION_SECS);
    state
        .completed_outbox
        .retain(|_, timestamp| now.saturating_sub(*timestamp) <= RETENTION_SECS);
    state
        .recipients
        .retain(|_, record| now.saturating_sub(record.updated_at) <= RETENTION_SECS * 8);
    state
        .chats
        .retain(|_, record| now.saturating_sub(record.updated_at) <= RETENTION_SECS * 8);
    state
        .lines
        .retain(|_, record| now.saturating_sub(record.updated_at) <= RETENTION_SECS * 8);
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn v1_ledger_migrates_and_processing_claims_recover() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("linq-state.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "events": {"2915e81c-5068-4796-ace2-21d2c94ad298": Utc::now().timestamp()}
            }))
            .unwrap(),
        )
        .unwrap();
        let state = DurableState::load(path).unwrap();
        assert_eq!(
            state
                .accept_inbox(
                    "2915e81c-5068-4796-ace2-21d2c94ad298",
                    serde_json::json!({})
                )
                .await
                .unwrap(),
            AcceptResult::AlreadyAccepted
        );
    }

    #[tokio::test]
    async fn outbox_reuses_one_operation_and_rejects_id_collision() {
        let temp = TempDir::new().unwrap();
        let state = DurableState::load(temp.path().join("state.json")).unwrap();
        let response = OutgoingResponse::text("hello");
        let stored = StoredResponse::from_response(&response);
        let target = OutboundTarget::PooledRecipient {
            recipient: "+12025550100".to_string(),
        };
        let id = response.delivery_id.to_string();
        assert_eq!(
            state
                .enqueue_outbox(&id, target.clone(), stored.clone())
                .await
                .unwrap(),
            AcceptResult::Accepted
        );
        assert_eq!(
            state
                .enqueue_outbox(&id, target, stored)
                .await
                .unwrap(),
            AcceptResult::AlreadyAccepted
        );
        assert!(
            state
                .enqueue_outbox(
                    &id,
                    OutboundTarget::PooledRecipient {
                        recipient: "+12025550101".to_string()
                    },
                    StoredResponse::from_response(&response)
                )
                .await
                .is_err()
        );
    }
}
