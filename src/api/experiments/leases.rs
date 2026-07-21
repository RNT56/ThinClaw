use super::*;

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{AeadInOut, Generate},
};

const LEASE_CREDENTIALS_ENVELOPE_VERSION: u64 = 1;
const MAX_LEASE_CREDENTIALS_BYTES: usize = 2 * 1024 * 1024;
const MAX_DURABLE_RUNNER_ARTIFACT_BYTES: usize =
    crate::experiments::artifact_store::MAX_DURABLE_ARTIFACT_BYTES;
const MAX_RUNNER_ARTIFACTS_PER_TRIAL: usize = 64;

fn lease_credentials_aad(lease_id: Uuid) -> String {
    format!("thinclaw-experiment-lease-credentials-v1:{lease_id}")
}

fn lease_credentials_key(token: &str) -> [u8; 32] {
    blake3::derive_key(
        "thinclaw experiment lease credentials encryption key v1",
        token.as_bytes(),
    )
}

fn validate_lease_credentials_payload(credentials: &serde_json::Value) -> ApiResult<()> {
    let env = credentials
        .get("env")
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            ApiError::InvalidInput("Lease credentials must contain an env object.".to_string())
        })?;
    if env.len() > 256 {
        return Err(ApiError::InvalidInput(
            "Lease credentials contain too many environment grants.".to_string(),
        ));
    }
    let mut total_bytes = 0usize;
    for (key, value) in env {
        let Some(value) = value.as_str() else {
            return Err(ApiError::InvalidInput(
                "Lease environment grant values must be strings.".to_string(),
            ));
        };
        if !crate::experiments::valid_env_name(key) || value.len() > 64 * 1024 {
            return Err(ApiError::InvalidInput(
                "Lease credentials contain an invalid environment grant.".to_string(),
            ));
        }
        total_bytes = total_bytes
            .saturating_add(key.len())
            .saturating_add(value.len());
    }
    if total_bytes > MAX_LEASE_CREDENTIALS_BYTES {
        return Err(ApiError::InvalidInput(
            "Lease environment grants exceed the size limit.".to_string(),
        ));
    }
    let references = credentials
        .get("secret_references")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            ApiError::InvalidInput(
                "Lease credentials must contain a secret_references array.".to_string(),
            )
        })?;
    let references = references
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                ApiError::InvalidInput("Lease secret references must be strings.".to_string())
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    crate::experiments::validate_secret_references(&references).map_err(ApiError::InvalidInput)?;
    Ok(())
}

fn seal_lease_credentials(
    lease_id: Uuid,
    token: &str,
    credentials: &serde_json::Value,
) -> ApiResult<serde_json::Value> {
    validate_lease_credentials_payload(credentials)?;
    let mut plaintext = serde_json::to_vec(credentials).map_err(ApiError::Serialization)?;
    if plaintext.len() > MAX_LEASE_CREDENTIALS_BYTES {
        return Err(ApiError::InvalidInput(format!(
            "Resolved runner credentials exceed the {} byte limit.",
            MAX_LEASE_CREDENTIALS_BYTES
        )));
    }
    let cipher = Aes256Gcm::new_from_slice(&lease_credentials_key(token))
        .map_err(|_| ApiError::Internal("failed to initialize lease credential cipher".into()))?;
    let nonce = Nonce::generate();
    cipher
        .encrypt_in_place(
            &nonce,
            lease_credentials_aad(lease_id).as_bytes(),
            &mut plaintext,
        )
        .map_err(|_| ApiError::Internal("failed to encrypt lease credentials".into()))?;
    let mut sealed = Vec::with_capacity(nonce.len() + plaintext.len());
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&plaintext);
    Ok(serde_json::json!({
        "sealed_credentials_version": LEASE_CREDENTIALS_ENVELOPE_VERSION,
        "sealed_credentials_base64": base64::engine::general_purpose::STANDARD.encode(sealed),
    }))
}

fn open_lease_credentials(
    lease_id: Uuid,
    token: &str,
    envelope: &serde_json::Value,
) -> ApiResult<serde_json::Value> {
    let version = envelope
        .get("sealed_credentials_version")
        .and_then(|value| value.as_u64());
    if version.is_none() {
        // Backward compatibility for leases created before credential sealing.
        // The caller immediately rewrites this legacy plaintext as an envelope.
        validate_lease_credentials_payload(envelope)?;
        return Ok(envelope.clone());
    }
    if version != Some(LEASE_CREDENTIALS_ENVELOPE_VERSION) {
        return Err(ApiError::Unavailable(
            "Unsupported lease credentials envelope version.".to_string(),
        ));
    }
    let encoded = envelope
        .get("sealed_credentials_base64")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ApiError::Unavailable("Lease credentials envelope is invalid.".into()))?;
    if encoded.len() > (MAX_LEASE_CREDENTIALS_BYTES + 64) * 2 {
        return Err(ApiError::Unavailable(
            "Lease credentials envelope exceeds the size limit.".into(),
        ));
    }
    let sealed = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::Unavailable("Lease credentials envelope is invalid.".into()))?;
    if sealed.len() < 12 + 16 || sealed.len() > MAX_LEASE_CREDENTIALS_BYTES + 12 + 16 {
        return Err(ApiError::Unavailable(
            "Lease credentials envelope has an invalid size.".into(),
        ));
    }
    let (nonce_bytes, ciphertext) = sealed.split_at(12);
    let nonce = Nonce::try_from(nonce_bytes)
        .map_err(|_| ApiError::Unavailable("Lease credentials envelope is invalid.".into()))?;
    let cipher = Aes256Gcm::new_from_slice(&lease_credentials_key(token))
        .map_err(|_| ApiError::Internal("failed to initialize lease credential cipher".into()))?;
    let mut plaintext = ciphertext.to_vec();
    cipher
        .decrypt_in_place(
            &nonce,
            lease_credentials_aad(lease_id).as_bytes(),
            &mut plaintext,
        )
        .map_err(|_| ApiError::Unavailable("Lease credentials could not be decrypted.".into()))?;
    let credentials: serde_json::Value = serde_json::from_slice(&plaintext).map_err(|_| {
        ApiError::Unavailable("Lease credentials payload is not valid JSON.".into())
    })?;
    validate_lease_credentials_payload(&credentials)?;
    Ok(credentials)
}

fn validate_remote_repository_url(raw: &str) -> ApiResult<String> {
    let value = raw.trim();
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
        || value.starts_with('-')
    {
        return Err(ApiError::InvalidInput(
            "Remote experiment repository URL is invalid.".to_string(),
        ));
    }

    if let Ok(url) = reqwest::Url::parse(value) {
        if !matches!(url.scheme(), "https" | "ssh") || url.host_str().is_none() {
            return Err(ApiError::InvalidInput(
                "Remote experiment repositories must use HTTPS or SSH.".to_string(),
            ));
        }
        if url.password().is_some()
            || (url.scheme() == "https" && !url.username().is_empty())
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ApiError::InvalidInput(
                "Remote experiment repository URLs must not embed credentials, query parameters, or fragments."
                    .to_string(),
            ));
        }
        return Ok(url.to_string());
    }

    // Git's SCP-like SSH syntax (`git@host:owner/repo.git`) is not an RFC URL.
    let Some((authority, path)) = value.split_once(':') else {
        return Err(ApiError::InvalidInput(
            "Remote experiment repositories must use HTTPS or SSH.".to_string(),
        ));
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let valid_authority = !authority.is_empty()
        && !host.is_empty()
        && authority.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'.' | b'_' | b'-' | b'[' | b']')
        });
    if !valid_authority
        || path.is_empty()
        || path.starts_with('-')
        || path.contains(['?', '#'])
        || path.split('/').any(|segment| segment == "..")
    {
        return Err(ApiError::InvalidInput(
            "Remote experiment SSH repository URL is invalid.".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn ensure_claimed_lease(lease: &ExperimentLease) -> ApiResult<()> {
    if lease.status != ExperimentLeaseStatus::Claimed {
        return Err(ApiError::Unavailable(
            "Experiment lease is not active and claimed.".to_string(),
        ));
    }
    Ok(())
}

fn lease_secret_values(lease: &ExperimentLease, token: &str) -> ApiResult<Vec<String>> {
    let credentials = open_lease_credentials(lease.id, token, &lease.credentials_payload)?;
    let mut values = credentials
        .get("env")
        .and_then(|value| value.as_object())
        .into_iter()
        .flat_map(|map| map.values())
        .filter_map(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .chain(std::iter::once(token.to_string()))
        .collect::<Vec<_>>();
    values.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    Ok(values)
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= max_bytes.saturating_sub(3))
        .last()
        .unwrap_or(0);
    format!("{}...", &value[..end])
}

fn sanitize_lease_text(value: &str, secrets: &[String], max_bytes: usize) -> String {
    let redacted = secrets.iter().fold(value.to_string(), |text, secret| {
        text.replace(secret, "[REDACTED]")
    });
    let without_controls = redacted
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect::<String>();
    truncate_utf8_bytes(&without_controls, max_bytes)
}

fn sanitize_lease_json(
    value: &serde_json::Value,
    secrets: &[String],
    max_serialized_bytes: usize,
) -> ApiResult<serde_json::Value> {
    fn visit(value: &serde_json::Value, secrets: &[String], depth: usize) -> serde_json::Value {
        if depth > 32 {
            return serde_json::Value::Null;
        }
        match value {
            serde_json::Value::String(text) => {
                serde_json::Value::String(sanitize_lease_text(text, secrets, 8 * 1024))
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(
                items
                    .iter()
                    .take(256)
                    .map(|item| visit(item, secrets, depth + 1))
                    .collect(),
            ),
            serde_json::Value::Object(map) => {
                let mut sanitized = serde_json::Map::new();
                for (index, (key, value)) in map.iter().take(128).enumerate() {
                    let mut key = sanitize_lease_text(key, secrets, 256);
                    if key.is_empty() {
                        key = format!("field_{index}");
                    }
                    if sanitized.contains_key(&key) {
                        key = format!("{key}_{index}");
                    }
                    sanitized.insert(key, visit(value, secrets, depth + 1));
                }
                serde_json::Value::Object(sanitized)
            }
            other => other.clone(),
        }
    }

    let sanitized = visit(value, secrets, 0);
    let size = serde_json::to_vec(&sanitized)
        .map_err(ApiError::Serialization)?
        .len();
    if size > max_serialized_bytes {
        return Err(ApiError::InvalidInput(format!(
            "Experiment runner metadata exceeds the {max_serialized_bytes} byte limit."
        )));
    }
    Ok(sanitized)
}

fn sanitize_runner_completion(
    lease: &ExperimentLease,
    token: &str,
    mut completion: ExperimentRunnerCompletion,
) -> ApiResult<ExperimentRunnerCompletion> {
    let secrets = lease_secret_values(lease, token)?;
    completion.metrics_json = sanitize_lease_json(&completion.metrics_json, &secrets, 256 * 1024)?;
    completion.artifact_manifest_json =
        sanitize_lease_json(&completion.artifact_manifest_json, &secrets, 256 * 1024)?;
    completion.summary = completion
        .summary
        .as_deref()
        .map(|value| sanitize_lease_text(value, &secrets, 8 * 1024));
    completion.log_preview_path = completion
        .log_preview_path
        .as_deref()
        .map(|value| sanitize_lease_text(value, &secrets, 4 * 1024));
    if completion
        .runtime_ms
        .is_some_and(|runtime_ms| runtime_ms > 365 * 24 * 60 * 60 * 1000)
    {
        return Err(ApiError::InvalidInput(
            "Experiment runner runtime exceeds the one-year limit.".to_string(),
        ));
    }
    if completion
        .attributed_cost_usd
        .is_some_and(|cost| !cost.is_finite() || !(0.0..=1_000_000.0).contains(&cost))
    {
        return Err(ApiError::InvalidInput(
            "Experiment runner attributed cost is invalid.".to_string(),
        ));
    }
    Ok(completion)
}

pub(super) async fn latest_active_lease(
    store: &Arc<dyn Database>,
    trial_id: Uuid,
) -> ApiResult<Option<ExperimentLease>> {
    let lease = store
        .get_experiment_lease_for_trial(trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(lease.filter(|lease| {
        matches!(
            lease.status,
            ExperimentLeaseStatus::Pending | ExperimentLeaseStatus::Claimed
        )
    }))
}

pub async fn lease_job(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLeaseJobResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut lease = verified_lease(store, lease_id, token).await?;
    if !matches!(
        lease.status,
        ExperimentLeaseStatus::Pending | ExperimentLeaseStatus::Claimed
    ) {
        return Err(ApiError::Unavailable(
            "Experiment lease is no longer active.".to_string(),
        ));
    }
    if lease.status == ExperimentLeaseStatus::Pending {
        lease.status = ExperimentLeaseStatus::Claimed;
        lease.claimed_at = Some(Utc::now());
        lease.updated_at = Utc::now();
        store
            .update_experiment_lease(&lease)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    let job: ExperimentRunnerJob =
        serde_json::from_value(lease.job_payload.clone()).map_err(ApiError::Serialization)?;
    Ok(ExperimentLeaseJobResponse { job })
}

pub async fn lease_credentials(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLeaseCredentialsResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut lease = verified_lease(store, lease_id, token).await?;
    if !matches!(
        lease.status,
        ExperimentLeaseStatus::Pending | ExperimentLeaseStatus::Claimed
    ) {
        return Err(ApiError::Unavailable(
            "Experiment lease is no longer active.".to_string(),
        ));
    }
    let was_legacy_plaintext = lease
        .credentials_payload
        .get("sealed_credentials_version")
        .is_none();
    let credentials = open_lease_credentials(lease.id, token, &lease.credentials_payload)?;
    if was_legacy_plaintext {
        lease.credentials_payload = seal_lease_credentials(lease.id, token, &credentials)?;
        lease.updated_at = Utc::now();
        store
            .update_experiment_lease(&lease)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    Ok(ExperimentLeaseCredentialsResponse { credentials })
}

pub async fn lease_status(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    req: ExperimentLeaseStatusRequest,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    ensure_claimed_lease(&lease)?;
    if !matches!(
        req.status.as_str(),
        "runner_started"
            | "running_prepare"
            | "running_benchmark"
            | "evaluating"
            | "uploading_artifacts"
            | "completing"
            | "failed"
    ) {
        return Err(ApiError::InvalidInput(
            "Unknown experiment runner status.".to_string(),
        ));
    }
    let secrets = lease_secret_values(&lease, token)?;
    let metadata = req
        .metadata
        .as_ref()
        .map(|value| sanitize_lease_json(value, &secrets, 64 * 1024))
        .transpose()?;
    let mut trial = store
        .get_experiment_trial(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(experiment_trial_not_found_message(lease.trial_id))
        })?;
    trial.provider_job_metadata = adapters::sanitize_provider_job_metadata(
        trial.runner_backend,
        &trial.provider_job_metadata,
    );
    trial.summary = Some(req.status.clone());
    trial.status = lease_runner_trial_status(&req.status, trial.status);
    if matches!(
        trial.status,
        ExperimentTrialStatus::Running | ExperimentTrialStatus::Evaluating
    ) && trial.started_at.is_none()
    {
        trial.started_at = Some(Utc::now());
    }
    if let Some(metadata) = metadata {
        trial.artifact_manifest_json = merge_json(&trial.artifact_manifest_json, &metadata);
    }
    trial.updated_at = Utc::now();
    store
        .update_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease status recorded.".to_string(),
    })
}

pub async fn lease_event(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    req: ExperimentLeaseEventRequest,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    ensure_claimed_lease(&lease)?;
    let secrets = lease_secret_values(&lease, token)?;
    let message = sanitize_lease_text(&req.message, &secrets, 4 * 1024);
    if message.trim().is_empty() {
        return Err(ApiError::InvalidInput(
            "Experiment runner event message must not be empty.".to_string(),
        ));
    }
    let metadata = req
        .metadata
        .as_ref()
        .map(|value| sanitize_lease_json(value, &secrets, 64 * 1024))
        .transpose()?;
    let mut trial = store
        .get_experiment_trial(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(experiment_trial_not_found_message(lease.trial_id))
        })?;
    trial.provider_job_metadata = adapters::sanitize_provider_job_metadata(
        trial.runner_backend,
        &trial.provider_job_metadata,
    );
    let mut manifest = if trial.artifact_manifest_json.is_object() {
        trial.artifact_manifest_json.clone()
    } else {
        serde_json::json!({})
    };
    let event_entry = serde_json::json!({
        "message": message,
        "metadata": metadata,
        "at": Utc::now().to_rfc3339(),
    });
    let events = manifest
        .as_object_mut()
        .ok_or_else(|| ApiError::Internal("failed to initialize trial artifact manifest".into()))?
        .entry("events".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let Some(array) = events.as_array_mut() {
        if array.len() >= 256 {
            let remove = array.len().saturating_sub(255);
            array.drain(..remove);
        }
        array.push(event_entry);
    }
    trial.artifact_manifest_json = manifest;
    trial.updated_at = Utc::now();
    store
        .update_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease event recorded.".to_string(),
    })
}

pub async fn lease_artifact(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    artifact: ExperimentRunnerArtifactUpload,
) -> ApiResult<ExperimentCampaignActionResponse> {
    let artifact_store = LocalArtifactStore::shared_default();
    lease_artifact_with_store(store, &artifact_store, user_id, lease_id, token, artifact).await
}

/// Core of [`lease_artifact`] parameterized over the durable [`ArtifactStore`] so
/// tests can inject a temp-rooted store. When the runner attaches inline
/// `content_base64`, the bytes are persisted to durable host storage and the
/// recorded `ExperimentArtifactRef` points at the durable path with
/// `fetchable: true`; otherwise the upload is recorded as posted (pod-local
/// breadcrumb only).
pub(super) async fn lease_artifact_with_store(
    store: &Arc<dyn Database>,
    artifact_store: &Arc<dyn ArtifactStore>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    artifact: ExperimentRunnerArtifactUpload,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    ensure_claimed_lease(&lease)?;
    let secrets = lease_secret_values(&lease, token)?;
    let kind = artifact.kind.trim();
    if kind.is_empty()
        || kind.len() > 64
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ApiError::InvalidInput(
            "Experiment artifact kind is invalid.".to_string(),
        ));
    }
    let kind = kind.to_string();
    let metadata = sanitize_lease_json(&artifact.metadata, &secrets, 64 * 1024)?;
    let mut artifacts = store
        .list_experiment_artifacts(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if artifacts.len() >= MAX_RUNNER_ARTIFACTS_PER_TRIAL {
        return Err(ApiError::InvalidInput(format!(
            "Experiment trial already has the maximum of {MAX_RUNNER_ARTIFACTS_PER_TRIAL} artifacts."
        )));
    }

    let artifact_id = Uuid::new_v4();
    let mut uri_or_local_path =
        sanitize_lease_text(&artifact.uri_or_local_path, &secrets, 4 * 1024);
    let mut size_bytes = artifact.size_bytes;
    let mut fetchable = artifact.fetchable;
    if let Some(content_base64) = artifact.content_base64 {
        if content_base64.len() > MAX_DURABLE_RUNNER_ARTIFACT_BYTES.saturating_mul(2) {
            return Err(ApiError::InvalidInput(format!(
                "Experiment artifact exceeds the {MAX_DURABLE_RUNNER_ARTIFACT_BYTES} byte limit."
            )));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(content_base64.as_bytes())
            .map_err(|_| ApiError::InvalidInput("Invalid artifact content_base64.".to_string()))?;
        if bytes.len() > MAX_DURABLE_RUNNER_ARTIFACT_BYTES {
            return Err(ApiError::InvalidInput(format!(
                "Experiment artifact exceeds the {MAX_DURABLE_RUNNER_ARTIFACT_BYTES} byte limit."
            )));
        }
        if artifact
            .size_bytes
            .is_some_and(|declared| declared != bytes.len() as u64)
        {
            return Err(ApiError::InvalidInput(
                "Experiment artifact size does not match its decoded content.".to_string(),
            ));
        }
        let durable = artifact_store
            .put(lease.trial_id, artifact_id, &kind, &bytes)
            .await
            .map_err(|e| ApiError::Internal(format!("failed to persist artifact: {e}")))?;
        size_bytes = Some(bytes.len() as u64);
        uri_or_local_path = durable;
        fetchable = true;
    }

    artifacts.push(ExperimentArtifactRef {
        id: artifact_id,
        trial_id: lease.trial_id,
        kind,
        uri_or_local_path,
        size_bytes,
        fetchable,
        metadata,
        created_at: Utc::now(),
    });
    store
        .replace_experiment_artifacts(lease.trial_id, &artifacts)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message: "Artifact recorded.".to_string(),
    })
}

pub async fn lease_complete(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    completion: ExperimentRunnerCompletion,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut lease = verified_lease(store, lease_id, token).await?;
    ensure_claimed_lease(&lease)?;
    let completion = sanitize_runner_completion(&lease, token, completion)?;
    let mut campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let mut trial = get_trial(store, user_id, lease.trial_id).await?;
    complete_trial_terminal(
        store,
        &project,
        &mut campaign,
        &mut trial,
        Some(&mut lease),
        completion,
    )
    .await?;
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease completed.".to_string(),
    })
}

pub async fn lease_owner_user_id(
    store: &Arc<dyn Database>,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<String> {
    let lease = verified_lease(store, lease_id, token).await?;
    let campaign = store
        .get_experiment_campaign(lease.campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(experiment_campaign_not_found_message(lease.campaign_id))
        })?;
    Ok(campaign.owner_user_id)
}

pub(super) async fn create_lease(
    store: &Arc<dyn Database>,
    user_id: &str,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    campaign: &ExperimentCampaign,
    trial: &ExperimentTrial,
) -> ApiResult<ExperimentLeaseAuthentication> {
    let token = format!("exp_{}_{}", short_id(campaign.id), Uuid::new_v4().simple());
    let repo_url = validate_remote_repository_url(
        &git_output(
            &project.workspace_path,
            &["remote", "get-url", &project.git_remote_name],
        )
        .await?,
    )?;
    let resolved_env_grants = resolved_runner_env_grants(user_id, runner).await?;
    let git_ref = campaign.experiment_branch.clone().ok_or_else(|| {
        ApiError::InvalidInput(experiment_campaign_missing_experiment_branch_message().to_string())
    })?;
    if git_ref.len() > 512
        || git_ref.starts_with('-')
        || git_ref.chars().any(char::is_control)
        || git_ref.chars().any(char::is_whitespace)
    {
        return Err(ApiError::InvalidInput(
            "Experiment branch is not a valid remote git reference.".to_string(),
        ));
    }
    let job = ExperimentRunnerJob {
        lease_id: Uuid::new_v4(),
        trial_id: trial.id,
        campaign_id: campaign.id,
        project_id: project.id,
        runner_profile_id: runner.id,
        backend: runner.backend,
        repo_url,
        git_ref,
        workdir: project.workdir.clone(),
        prepare_command: project.prepare_command.clone(),
        run_command: project.run_command.clone(),
        primary_metric: project.primary_metric.clone(),
        secondary_metrics: project.secondary_metrics.clone(),
        // All environment values travel through the separately encrypted
        // credentials payload. The ordinary job document must never duplicate
        // plaintext secrets.
        env_grants: serde_json::json!({}),
        artifact_paths: vec!["run.log".to_string(), "summary.json".to_string()],
    };
    let credentials = serde_json::json!({
        "env": resolved_env_grants,
        "secret_references": runner.secret_references,
    });
    let sealed_credentials = seal_lease_credentials(job.lease_id, &token, &credentials)?;
    let lease = ExperimentLease {
        id: job.lease_id,
        campaign_id: campaign.id,
        trial_id: trial.id,
        runner_profile_id: runner.id,
        status: ExperimentLeaseStatus::Pending,
        token_hash: hash_lease_token(&token),
        job_payload: serde_json::to_value(&job).map_err(|e| ApiError::Internal(e.to_string()))?,
        credentials_payload: sealed_credentials,
        expires_at: Utc::now() + chrono::Duration::minutes(DEFAULT_REMOTE_LEASE_MINUTES),
        claimed_at: None,
        completed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store
        .create_experiment_lease(&lease)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentLeaseAuthentication {
        lease_id: lease.id,
        token,
    })
}

pub(super) async fn verified_lease(
    store: &Arc<dyn Database>,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLease> {
    let lease = store
        .get_experiment_lease(lease_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_lease_not_found_message(lease_id)))?;
    if lease.expires_at < Utc::now() {
        return Err(ApiError::Unavailable(
            experiment_lease_expired_message().to_string(),
        ));
    }
    if lease.token_hash != hash_lease_token(token) {
        return Err(ApiError::InvalidInput(
            invalid_experiment_lease_token_message().to_string(),
        ));
    }
    Ok(lease)
}

pub(super) async fn latest_trial(
    store: &Arc<dyn Database>,
    campaign_id: Uuid,
) -> ApiResult<Option<ExperimentTrial>> {
    let mut trials = store
        .list_experiment_trials(campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(trials.pop())
}

pub(super) async fn active_trial(
    store: &Arc<dyn Database>,
    campaign_id: Uuid,
) -> ApiResult<Option<ExperimentTrial>> {
    let trials = store
        .list_experiment_trials(campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(trials.into_iter().find(|trial| {
        matches!(
            trial.status,
            ExperimentTrialStatus::Preparing
                | ExperimentTrialStatus::Running
                | ExperimentTrialStatus::Evaluating
        )
    }))
}

#[cfg(test)]
mod lease_security_tests {
    use super::*;

    #[test]
    fn lease_credentials_are_authenticated_and_not_plaintext_at_rest() {
        let lease_id = Uuid::new_v4();
        let token = "exp_0123456789ab_0123456789abcdef0123456789abcdef";
        let credentials = serde_json::json!({
            "env": { "API_KEY": "super-secret-value" },
            "secret_references": ["research_api_key"]
        });
        let envelope = seal_lease_credentials(lease_id, token, &credentials).unwrap();
        let encoded = envelope.to_string();
        assert!(!encoded.contains("super-secret-value"));
        assert!(!encoded.contains("API_KEY"));
        assert_eq!(
            open_lease_credentials(lease_id, token, &envelope).unwrap(),
            credentials
        );
        assert!(open_lease_credentials(lease_id, "wrong-token", &envelope).is_err());
        assert!(open_lease_credentials(Uuid::new_v4(), token, &envelope).is_err());
    }

    #[test]
    fn legacy_plaintext_credentials_are_validated_before_migration() {
        let lease_id = Uuid::new_v4();
        let legacy = serde_json::json!({
            "env": { "SAFE_NAME": "value" },
            "secret_references": []
        });
        assert_eq!(
            open_lease_credentials(lease_id, "token", &legacy).unwrap(),
            legacy
        );
        let malformed = serde_json::json!({ "env": { "BAD-NAME": "value" } });
        assert!(open_lease_credentials(lease_id, "token", &malformed).is_err());
        let malformed_references = serde_json::json!({
            "env": {},
            "secret_references": ["secret:BAD-NAME"]
        });
        assert!(open_lease_credentials(lease_id, "token", &malformed_references).is_err());
        let wrong_reference_type = serde_json::json!({
            "env": {},
            "secret_references": "secret"
        });
        assert!(open_lease_credentials(lease_id, "token", &wrong_reference_type).is_err());
    }

    #[test]
    fn callback_metadata_redacts_tokens_credentials_and_excess_structure() {
        let token = "exp_0123456789ab_0123456789abcdef0123456789abcdef";
        let secrets = vec!["super-secret-value".to_string(), token.to_string()];
        let value = serde_json::json!({
            "message": format!("token={token}; key=super-secret-value"),
            "nested": [{ "secret": "super-secret-value" }]
        });
        let sanitized = sanitize_lease_json(&value, &secrets, 64 * 1024).unwrap();
        let encoded = sanitized.to_string();
        assert!(!encoded.contains(token));
        assert!(!encoded.contains("super-secret-value"));
        assert!(encoded.contains("[REDACTED]"));
    }

    #[test]
    fn remote_repository_urls_reject_credentials_and_option_injection() {
        assert!(validate_remote_repository_url("https://github.com/o/r.git").is_ok());
        assert!(validate_remote_repository_url("git@github.com:o/r.git").is_ok());
        for invalid in [
            "https://token@github.com/o/r.git",
            "https://github.com/o/r.git?token=secret",
            "file:///tmp/repo",
            "/tmp/repo",
            "--upload-pack=evil",
            "git@github.com:../private",
        ] {
            assert!(
                validate_remote_repository_url(invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }
}
