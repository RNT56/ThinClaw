use super::*;

pub(super) fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("ThinClaw/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(PROVIDER_CONNECT_TIMEOUT)
        .timeout(PROVIDER_REQUEST_TIMEOUT)
        .build()
        .map_err(|err| format!("failed to build HTTP client: {err}"))
}

pub(super) async fn response_error(context: &str, response: reqwest::Response) -> String {
    let status = response.status();
    // Provider errors may echo submitted cloud-init, environment values, or the
    // lease bootstrap command. Never reflect a remote body into durable campaign
    // summaries or operator-visible errors.
    format!("{context}: HTTP {status}")
}

pub(super) async fn validate_runpod_credentials(api_key: &str) -> Result<String, String> {
    let client = http_client()?;
    let response = client
        .get(format!("{RUNPOD_API_BASE}/pods"))
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|err| format!("RunPod validation request failed: {err}"))?;
    match response.status() {
        StatusCode::OK => {
            Ok("RunPod credentials validated against the official Pods API.".to_string())
        }
        StatusCode::UNAUTHORIZED => {
            Err("RunPod credentials were rejected by the Pods API.".to_string())
        }
        _ => Err(response_error("RunPod validation failed", response).await),
    }
}

pub(super) async fn validate_vast_credentials(api_key: &str) -> Result<String, String> {
    let client = http_client()?;
    let response = client
        .get(format!("{VAST_API_BASE}/api/v0/users/current/"))
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|err| format!("Vast.ai validation request failed: {err}"))?;
    match response.status() {
        StatusCode::OK => {
            Ok("Vast.ai credentials validated against the official user API.".to_string())
        }
        StatusCode::UNAUTHORIZED => {
            Err("Vast.ai credentials were rejected by the API.".to_string())
        }
        _ => Err(response_error("Vast.ai validation failed", response).await),
    }
}

pub(super) async fn validate_lambda_credentials(api_key: &str) -> Result<String, String> {
    let client = http_client()?;
    let response = client
        .get(format!("{LAMBDA_API_BASE}/instance-types"))
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|err| format!("Lambda validation request failed: {err}"))?;
    match response.status() {
        StatusCode::OK => {
            Ok("Lambda credentials validated against the instance-types API.".to_string())
        }
        StatusCode::UNAUTHORIZED => {
            Err("Lambda credentials were rejected by the Cloud API.".to_string())
        }
        _ => Err(response_error("Lambda validation failed", response).await),
    }
}

pub(super) fn lambda_response_instance_id(value: &serde_json::Value) -> Option<String> {
    value
        .get("instance_id")
        .and_then(value_to_string)
        .or_else(|| value.get("id").and_then(value_to_string))
        .or_else(|| {
            value
                .get("instance_ids")
                .and_then(|items| items.as_array())
                .and_then(|items| items.first())
                .and_then(value_to_string)
        })
        .or_else(|| value.get("data").and_then(lambda_response_instance_id))
}

pub(super) fn lambda_response_instance_metadata(value: &serde_json::Value) -> serde_json::Value {
    if let Some(instance) = value.get("instance")
        && instance.is_object()
    {
        return instance.clone();
    }
    if let Some(items) = value.get("instances").and_then(|entry| entry.as_array())
        && let Some(instance) = items.iter().find(|entry| entry.is_object())
    {
        return instance.clone();
    }
    if let Some(data) = value.get("data") {
        if let Some(instance) = data.get("instance")
            && instance.is_object()
        {
            return instance.clone();
        }
        if let Some(items) = data.get("instances").and_then(|entry| entry.as_array())
            && let Some(instance) = items.iter().find(|entry| entry.is_object())
        {
            return instance.clone();
        }
        if data.is_object() {
            return data.clone();
        }
    }
    value.clone()
}

pub(super) async fn launch_lambda_instance(
    runner: &ExperimentRunnerProfile,
    auth: &ExperimentLeaseAuthentication,
    bootstrap_command: &str,
    api_key: &str,
) -> Result<RunnerLaunchOutcome, String> {
    let payload = lambda_launch_payload(runner, bootstrap_command, auth).ok_or_else(|| {
        "Lambda launch requires backend_config.launch_payload with the official Lambda Cloud API request body.".to_string()
    })?;
    let client = http_client()?;
    let response = client
        .post(format!("{LAMBDA_API_BASE}/instances/launch"))
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("Lambda launch request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("Lambda launch failed", response).await);
    }
    let body: serde_json::Value =
        crate::http_response::bounded_json(response, MAX_PROVIDER_RESPONSE_BYTES)
            .await
            .map_err(|err| format!("failed to decode Lambda launch response: {err}"))?;
    let instance_id = validate_provider_id(
        "Lambda launch",
        &lambda_response_instance_id(&body).ok_or_else(|| {
            "Lambda launch succeeded but response did not include an instance id.".to_string()
        })?,
    )?;
    let provider_job_metadata = sanitize_provider_job_metadata(
        ExperimentRunnerBackend::Lambda,
        &serde_json::json!({
            "provider": "lambda",
            "instance_id": instance_id,
            "launch_request": payload,
            "instance": lambda_response_instance_metadata(&body),
        }),
    );
    Ok(RunnerLaunchOutcome {
        message: format!("Lambda instance {instance_id} launched."),
        bootstrap_command: None,
        provider_template: None,
        provider_job_id: Some(instance_id.clone()),
        provider_job_metadata,
        auto_launched: true,
        requires_operator_action: false,
    })
}

pub(super) async fn revoke_lambda_instance(
    runner: &ExperimentRunnerProfile,
    api_key: &str,
    instance_id: &str,
    _action: RemoteLaunchAction,
) -> Result<String, String> {
    let instance_id = validate_provider_id("Lambda terminate", instance_id)?;
    let client = http_client()?;
    let payload = lambda_terminate_payload(runner, &instance_id);
    let response = client
        .post(format!("{LAMBDA_API_BASE}/instances/terminate"))
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("Lambda terminate request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("Lambda terminate failed", response).await);
    }
    Ok(format!(
        "Lambda instance termination requested: {instance_id}"
    ))
}

pub(super) async fn launch_runpod_pod(
    runner: &ExperimentRunnerProfile,
    auth: &ExperimentLeaseAuthentication,
    bootstrap_command: &str,
    api_key: &str,
) -> Result<RunnerLaunchOutcome, String> {
    let client = http_client()?;
    let image = runner
        .image_or_runtime
        .clone()
        .or_else(|| backend_string(runner, "image"))
        .ok_or_else(|| {
            "RunPod launch requires image_or_runtime or backend_config.image".to_string()
        })?;
    let mut payload = serde_json::Map::new();
    payload.insert(
        "name".to_string(),
        serde_json::json!(short_launch_name("thinclaw-exp", auth)),
    );
    payload.insert("imageName".to_string(), serde_json::json!(image));
    payload.insert("computeType".to_string(), serde_json::json!("GPU"));
    payload.insert("gpuCount".to_string(), serde_json::json!(gpu_count(runner)));
    payload.insert(
        "env".to_string(),
        serde_json::Value::Object(provider_env_map(runner)),
    );
    payload.insert(
        "dockerEntrypoint".to_string(),
        serde_json::json!(["sh", "-lc"]),
    );
    payload.insert(
        "dockerStartCmd".to_string(),
        serde_json::json!([bootstrap_command]),
    );
    if let Some(cloud_type) = backend_string(runner, "cloud_type") {
        payload.insert("cloudType".to_string(), serde_json::json!(cloud_type));
    }
    let gpu_type_ids = if backend_array_strings(runner, "gpu_type_ids").is_empty() {
        gpu_type_hint(runner).into_iter().collect::<Vec<_>>()
    } else {
        backend_array_strings(runner, "gpu_type_ids")
    };
    if !gpu_type_ids.is_empty() {
        payload.insert("gpuTypeIds".to_string(), serde_json::json!(gpu_type_ids));
    }
    let data_center_ids = backend_array_strings(runner, "data_center_ids");
    if !data_center_ids.is_empty() {
        payload.insert(
            "dataCenterIds".to_string(),
            serde_json::json!(data_center_ids),
        );
    }
    let ports = backend_array_strings(runner, "ports");
    if !ports.is_empty() {
        payload.insert("ports".to_string(), serde_json::json!(ports));
    }
    if let Some(container_disk_gb) =
        backend_u64(runner, "container_disk_gb").or_else(|| backend_u64(runner, "disk_gb"))
    {
        payload.insert(
            "containerDiskInGb".to_string(),
            serde_json::json!(container_disk_gb),
        );
    }
    if let Some(volume_gb) = backend_u64(runner, "volume_gb") {
        payload.insert("volumeInGb".to_string(), serde_json::json!(volume_gb));
    }
    if let Some(template_id) = backend_string(runner, "template_id") {
        payload.insert("templateId".to_string(), serde_json::json!(template_id));
    }
    if let Some(interruptible) = backend_bool(runner, "interruptible") {
        payload.insert(
            "interruptible".to_string(),
            serde_json::json!(interruptible),
        );
    }
    if let Some(public_ip) = backend_bool(runner, "support_public_ip") {
        payload.insert("supportPublicIp".to_string(), serde_json::json!(public_ip));
    }

    let response = client
        .post(format!("{RUNPOD_API_BASE}/pods"))
        .bearer_auth(api_key)
        .json(&serde_json::Value::Object(payload.clone()))
        .send()
        .await
        .map_err(|err| format!("RunPod launch request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("RunPod launch failed", response).await);
    }
    let pod: serde_json::Value =
        crate::http_response::bounded_json(response, MAX_PROVIDER_RESPONSE_BYTES)
            .await
            .map_err(|err| format!("failed to decode RunPod launch response: {err}"))?;
    let pod_id = validate_provider_id(
        "RunPod launch",
        pod.get("id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                "RunPod launch succeeded but response did not include a pod id.".to_string()
            })?,
    )?;
    let provider_job_metadata = sanitize_provider_job_metadata(
        ExperimentRunnerBackend::Runpod,
        &serde_json::json!({
            "provider": "runpod",
            "pod_id": pod_id,
            "launch_request": payload,
            "pod": pod,
        }),
    );
    Ok(RunnerLaunchOutcome {
        message: format!("RunPod pod {pod_id} launched."),
        bootstrap_command: None,
        provider_template: None,
        provider_job_id: Some(pod_id.clone()),
        provider_job_metadata,
        auto_launched: true,
        requires_operator_action: false,
    })
}

pub(super) async fn revoke_runpod_pod(
    api_key: &str,
    pod_id: &str,
    action: RemoteLaunchAction,
) -> Result<String, String> {
    let pod_id = validate_provider_id("RunPod revoke", pod_id)?;
    let client = http_client()?;
    let (request, label) = match action {
        RemoteLaunchAction::Cancel => (
            client
                .delete(provider_url(RUNPOD_API_BASE, &["pods", &pod_id])?)
                .bearer_auth(api_key),
            "RunPod pod deleted",
        ),
        RemoteLaunchAction::Pause | RemoteLaunchAction::Reissue => (
            client
                .post(provider_url(RUNPOD_API_BASE, &["pods", &pod_id, "stop"])?)
                .bearer_auth(api_key),
            "RunPod pod stopped",
        ),
    };
    let response = request
        .send()
        .await
        .map_err(|err| format!("RunPod revoke request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("RunPod revoke failed", response).await);
    }
    Ok(format!("{label}: {pod_id}"))
}

pub(super) fn normalized_vast_gpu_name(runner: &ExperimentRunnerProfile) -> Option<String> {
    backend_string(runner, "gpu_name").or_else(|| {
        gpu_type_hint(runner).map(|value| {
            value
                .replace("NVIDIA ", "")
                .replace("AMD ", "")
                .replace("GeForce ", "")
                .replace("  ", " ")
                .replace([' ', '-'], "_")
        })
    })
}

pub(super) async fn select_vast_offer(
    runner: &ExperimentRunnerProfile,
    api_key: &str,
) -> Result<(u64, serde_json::Value), String> {
    let client = http_client()?;
    let mut body = serde_json::Map::new();
    body.insert("limit".to_string(), serde_json::json!(3));
    body.insert(
        "type".to_string(),
        serde_json::json!(
            backend_string(runner, "offer_type").unwrap_or_else(|| "ondemand".to_string())
        ),
    );
    body.insert("verified".to_string(), serde_json::json!({ "eq": true }));
    body.insert("rentable".to_string(), serde_json::json!({ "eq": true }));
    body.insert("rented".to_string(), serde_json::json!({ "eq": false }));
    body.insert(
        "order".to_string(),
        serde_json::json!([["dph_total", "asc"]]),
    );
    body.insert(
        "num_gpus".to_string(),
        serde_json::json!({ "gte": gpu_count(runner) }),
    );
    if let Some(min_vram_gb) = min_vram_gb(runner) {
        body.insert(
            "gpu_ram".to_string(),
            serde_json::json!({ "gte": min_vram_gb * 1024 }),
        );
    }
    if let Some(gpu_name) = normalized_vast_gpu_name(runner) {
        body.insert(
            "gpu_name".to_string(),
            serde_json::json!({ "in": [gpu_name] }),
        );
    }
    let response = client
        .post(format!("{VAST_API_BASE}/api/v0/bundles/"))
        .bearer_auth(api_key)
        .json(&serde_json::Value::Object(body.clone()))
        .send()
        .await
        .map_err(|err| format!("Vast.ai offer search failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("Vast.ai offer search failed", response).await);
    }
    let result: serde_json::Value =
        crate::http_response::bounded_json(response, MAX_PROVIDER_RESPONSE_BYTES)
            .await
            .map_err(|err| format!("failed to decode Vast.ai offer search response: {err}"))?;
    let offer = result
        .get("offers")
        .and_then(|value| value.as_array())
        .and_then(|offers| offers.first())
        .cloned()
        .ok_or_else(|| {
            "Vast.ai search returned no matching offers for the configured GPU requirements."
                .to_string()
        })?;
    let ask_id = offer
        .get("id")
        .and_then(value_to_u64)
        .ok_or_else(|| "Vast.ai offer search response did not include an offer id.".to_string())?;
    Ok((ask_id, offer))
}

pub(super) async fn launch_vast_instance(
    runner: &ExperimentRunnerProfile,
    auth: &ExperimentLeaseAuthentication,
    bootstrap_command: &str,
    api_key: &str,
) -> Result<RunnerLaunchOutcome, String> {
    let client = http_client()?;
    let image = runner
        .image_or_runtime
        .clone()
        .or_else(|| backend_string(runner, "image"))
        .ok_or_else(|| {
            "Vast.ai launch requires image_or_runtime or backend_config.image".to_string()
        })?;
    let explicit_ask_id = backend_u64(runner, "offer_id").or_else(|| backend_u64(runner, "ask_id"));
    let (ask_id, selected_offer) = match explicit_ask_id {
        Some(id) => (
            id,
            serde_json::json!({ "id": id, "source": "backend_config" }),
        ),
        None => select_vast_offer(runner, api_key).await?,
    };
    let mut payload = serde_json::Map::new();
    payload.insert("image".to_string(), serde_json::json!(image));
    payload.insert(
        "label".to_string(),
        serde_json::json!(short_launch_name("thinclaw-exp", auth)),
    );
    payload.insert("target_state".to_string(), serde_json::json!("running"));
    payload.insert(
        "disk".to_string(),
        serde_json::json!(backend_u64(runner, "disk_gb").unwrap_or(50)),
    );
    payload.insert(
        "runtype".to_string(),
        serde_json::json!(backend_string(runner, "runtype").unwrap_or_else(|| "ssh".to_string())),
    );
    payload.insert("onstart".to_string(), serde_json::json!(bootstrap_command));
    if let Some(template_hash_id) = backend_string(runner, "template_hash_id") {
        payload.insert(
            "template_hash_id".to_string(),
            serde_json::json!(template_hash_id),
        );
    }
    if let Some(cancel_unavail) = backend_bool(runner, "cancel_unavail") {
        payload.insert(
            "cancel_unavail".to_string(),
            serde_json::json!(cancel_unavail),
        );
    }
    let env = vast_env_flags(runner);
    if !env.is_empty() {
        payload.insert("env".to_string(), serde_json::json!(env));
    }
    let response = client
        .put(format!("{VAST_API_BASE}/api/v0/asks/{ask_id}/"))
        .bearer_auth(api_key)
        .json(&serde_json::Value::Object(payload.clone()))
        .send()
        .await
        .map_err(|err| format!("Vast.ai launch request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("Vast.ai launch failed", response).await);
    }
    let instance: serde_json::Value =
        crate::http_response::bounded_json(response, MAX_PROVIDER_RESPONSE_BYTES)
            .await
            .map_err(|err| format!("failed to decode Vast.ai launch response: {err}"))?;
    let instance_id = validate_numeric_provider_id(
        "Vast.ai launch",
        &instance
            .get("new_contract")
            .and_then(value_to_u64)
            .map(|value| value.to_string())
            .or_else(|| instance.get("instance_id").and_then(value_to_string))
            .ok_or_else(|| {
                "Vast.ai launch succeeded but response did not include an instance id.".to_string()
            })?,
    )?;
    let provider_job_metadata = sanitize_provider_job_metadata(
        ExperimentRunnerBackend::Vast,
        &serde_json::json!({
            "provider": "vast",
            "instance_id": instance_id,
            "ask_id": ask_id,
            "selected_offer": selected_offer,
            "instance": instance,
        }),
    );
    Ok(RunnerLaunchOutcome {
        message: format!("Vast.ai instance {instance_id} launched."),
        bootstrap_command: None,
        provider_template: None,
        provider_job_id: Some(instance_id.clone()),
        provider_job_metadata,
        auto_launched: true,
        requires_operator_action: false,
    })
}

pub(super) async fn revoke_vast_instance(
    api_key: &str,
    instance_id: &str,
    action: RemoteLaunchAction,
) -> Result<String, String> {
    let instance_id = validate_numeric_provider_id("Vast.ai revoke", instance_id)?;
    let client = http_client()?;
    let response = match action {
        RemoteLaunchAction::Cancel => client
            .delete(provider_url(
                VAST_API_BASE,
                &["api", "v0", "instances", &instance_id, ""],
            )?)
            .bearer_auth(api_key)
            .send()
            .await
            .map_err(|err| format!("Vast.ai destroy request failed: {err}"))?,
        RemoteLaunchAction::Pause | RemoteLaunchAction::Reissue => client
            .put(provider_url(
                VAST_API_BASE,
                &["api", "v0", "instances", &instance_id, ""],
            )?)
            .bearer_auth(api_key)
            .json(&serde_json::json!({ "state": "stopped" }))
            .send()
            .await
            .map_err(|err| format!("Vast.ai stop request failed: {err}"))?,
    };
    if !response.status().is_success() {
        return Err(response_error("Vast.ai revoke failed", response).await);
    }
    Ok(match action {
        RemoteLaunchAction::Cancel => format!("Vast.ai instance destroyed: {instance_id}"),
        RemoteLaunchAction::Pause | RemoteLaunchAction::Reissue => {
            format!("Vast.ai instance stopped: {instance_id}")
        }
    })
}
