use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;
use thinclaw::extensions::{AuthHint, ExtensionKind, ExtensionSource};
use thinclaw::registry::{ManifestKind, RegistryCatalog};

const WAVE6_CHANNELS: &[&str] = &[
    "mattermost",
    "twilio_sms",
    "dingtalk",
    "feishu_lark",
    "wecom",
    "weixin",
    "qq",
    "line",
    "matrix",
    "google_chat",
    "ms_teams",
    "twitch",
];

#[test]
fn wave6_channel_packages_are_in_real_registry_catalog() {
    let catalog = RegistryCatalog::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("registry")
            .as_path(),
    )
    .expect("real registry should load");

    let channel_names: HashSet<_> = catalog
        .list(Some(ManifestKind::Channel), None)
        .into_iter()
        .map(|manifest| manifest.name.as_str())
        .collect();

    for name in WAVE6_CHANNELS {
        assert!(
            channel_names.contains(name),
            "missing WASM channel registry package: {name}"
        );

        let manifest = catalog
            .get(&format!("channels/{name}"))
            .expect("channel manifest should be addressable by qualified key");
        assert_eq!(manifest.kind, ManifestKind::Channel);
        assert_eq!(
            manifest.source.capabilities,
            format!("{name}.capabilities.json"),
            "capability filename should follow channel package naming"
        );
        assert!(
            manifest.source.dir.starts_with("channels-src/"),
            "channel source dir should live under channels-src/"
        );
        assert!(
            manifest.artifacts.contains_key("wasm32-wasip2"),
            "channel manifest should declare the WASI component target"
        );
        assert!(
            manifest
                .auth_summary
                .as_ref()
                .is_some_and(
                    |auth| auth.method.as_deref() == Some("manual") && !auth.secrets.is_empty(),
                ),
            "channel manifest should summarize manual credentials"
        );
        assert!(
            manifest.tags.iter().any(|tag| tag == "messaging"),
            "channel manifest should be discoverable with the messaging tag"
        );

        let entry = manifest.to_registry_entry();
        assert_eq!(entry.kind, ExtensionKind::WasmChannel);
        assert!(
            matches!(entry.auth_hint, AuthHint::CapabilitiesAuth),
            "manual-auth channel packages should advertise capabilities auth"
        );
        assert!(
            matches!(entry.source, ExtensionSource::WasmBuildable { .. }),
            "null-artifact channel packages should resolve to buildable sources"
        );

        let flat_wasm = root_channel_dir(name).join(format!("{name}.wasm"));
        assert!(
            flat_wasm.exists(),
            "bundled channel {name} should include a flat packaged wasm artifact at {}",
            flat_wasm.display()
        );
    }
}

#[test]
fn wave6_channel_setup_descriptors_match_auth_summary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let catalog =
        RegistryCatalog::load(root.join("registry").as_path()).expect("real registry should load");

    for name in WAVE6_CHANNELS {
        let manifest = catalog
            .get(&format!("channels/{name}"))
            .expect("channel manifest should be addressable by qualified key");
        let manifest_secrets: HashSet<_> = manifest
            .auth_summary
            .as_ref()
            .expect("channel manifest should include auth summary")
            .secrets
            .iter()
            .map(|secret| secret.as_str())
            .collect();

        let caps_path = root
            .join("channels-src")
            .join(name)
            .join(format!("{name}.capabilities.json"));
        let raw =
            std::fs::read_to_string(&caps_path).expect("capabilities file should be readable");
        let caps = thinclaw::channels::wasm::ChannelCapabilitiesFile::from_json(&raw)
            .expect("capabilities file should parse through channel schema");

        assert!(
            !caps.setup.required_secrets.is_empty(),
            "{name} should declare setup secrets consumed by setup/auth surfaces"
        );
        for secret in &caps.setup.required_secrets {
            assert!(
                !secret.prompt.trim().is_empty(),
                "{name} secret {} should include an operator prompt",
                secret.name
            );
            assert!(
                manifest_secrets.contains(secret.name.as_str()),
                "{name} setup secret {} should be summarized in registry auth metadata",
                secret.name
            );
        }

        if let Some(endpoint) = &caps.setup.validation_endpoint {
            assert!(
                endpoint.starts_with("http://") || endpoint.starts_with("https://"),
                "{name} validation endpoint should be an HTTP(S) URL template"
            );
        }
    }
}

#[test]
fn provider_channel_response_shapes_cover_nested_api_payloads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let line = load_channel_config(root, "line");
    assert_eq!(
        line.pointer("/response/body/messages/0/type")
            .and_then(Value::as_str),
        Some("text"),
        "LINE responses must render a messages array"
    );
    assert_eq!(
        line.pointer("/response/body/messages/0/text")
            .and_then(Value::as_str),
        Some("{content}")
    );
    assert_eq!(
        line.pointer("/events_path").and_then(Value::as_str),
        Some("events"),
        "LINE inbound events are batched"
    );

    let dingtalk = load_channel_config(root, "dingtalk");
    assert_eq!(
        dingtalk
            .pointer("/response/body/text/content")
            .and_then(Value::as_str),
        Some("{content}"),
        "DingTalk requires a nested text.content response object"
    );

    let twilio = load_channel_config(root, "twilio_sms");
    assert_eq!(
        twilio
            .pointer("/response/body/From")
            .and_then(Value::as_str),
        Some("{TWILIO_FROM_NUMBER}"),
        "Twilio sender number must come from an injected secret placeholder"
    );
    let twilio_url = twilio
        .pointer("/response/url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(twilio_url.contains("{TWILIO_ACCOUNT_SID}"));
    assert!(twilio_url.contains("{TWILIO_AUTH_TOKEN}"));
    assert!(
        !twilio_url.contains("ACxxxxxxxx") && !twilio_url.contains("from_number"),
        "Twilio response URL must not contain fake credential defaults"
    );
}

#[test]
fn provider_channel_inbound_hardening_is_declared() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let line = load_caps(root, "line");
    assert_eq!(
        line.pointer("/capabilities/channel/webhook/secret_validation")
            .and_then(Value::as_str),
        Some("hmac_sha256_base64_body")
    );

    let twitch = load_caps(root, "twitch");
    assert_eq!(
        twitch
            .pointer("/capabilities/channel/webhook/secret_validation")
            .and_then(Value::as_str),
        Some("twitch_eventsub_hmac_sha256")
    );
    assert_eq!(
        twitch
            .pointer("/config/challenge/response_format")
            .and_then(Value::as_str),
        Some("text")
    );

    let twilio = load_caps(root, "twilio_sms");
    assert_eq!(
        twilio
            .pointer("/capabilities/channel/webhook/secret_validation")
            .and_then(Value::as_str),
        Some("twilio_request_signature")
    );

    for name in ["wecom", "weixin", "feishu_lark"] {
        let caps = load_caps(root, name);
        assert!(
            caps.pointer("/config/challenge").is_some(),
            "{name} should declare its platform challenge handshake"
        );
    }
}

fn root_channel_dir(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("channels-src")
        .join(name)
}

fn load_caps(root: &Path, name: &str) -> Value {
    let path = root
        .join("channels-src")
        .join(name)
        .join(format!("{name}.capabilities.json"));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn load_channel_config(root: &Path, name: &str) -> Value {
    load_caps(root, name)
        .pointer("/config")
        .cloned()
        .unwrap_or_else(|| panic!("{name} capabilities should include config"))
}

#[test]
fn messaging_bundle_includes_wave6_channel_packages() {
    let catalog = RegistryCatalog::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("registry")
            .as_path(),
    )
    .expect("real registry should load");

    let bundle = catalog
        .get_bundle("messaging")
        .expect("messaging bundle should exist");

    for name in WAVE6_CHANNELS {
        let key = format!("channels/{name}");
        assert!(
            bundle.extensions.contains(&key),
            "messaging bundle should include {key}"
        );
    }

    let (_, missing) = catalog
        .resolve_bundle("messaging")
        .expect("messaging bundle should resolve");
    assert!(
        missing.is_empty(),
        "messaging bundle has unresolved entries: {missing:?}"
    );
}

#[test]
fn wave6_channel_configs_cover_provider_payload_and_auth_shapes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = [
        (
            "twilio_sms",
            "twilio_request_signature",
            "X-Twilio-Signature",
            vec![
                "TWILIO_ACCOUNT_SID",
                "TWILIO_AUTH_TOKEN",
                "TWILIO_FROM_NUMBER",
            ],
        ),
        (
            "line",
            "hmac_sha256_base64_body",
            "X-Line-Signature",
            vec!["messages", "replyToken"],
        ),
        (
            "dingtalk",
            "equals",
            "X-Webhook-Secret",
            vec!["msgtype", "text"],
        ),
        (
            "feishu_lark",
            "equals",
            "X-Webhook-Secret",
            vec!["receive_id", "$json_string"],
        ),
        (
            "twitch",
            "twitch_eventsub_hmac_sha256",
            "Twitch-Eventsub-Message-Signature",
            vec!["message"],
        ),
    ];

    for (name, validation, header, response_fragments) in cases {
        let path = root
            .join("channels-src")
            .join(name)
            .join(format!("{name}.capabilities.json"));
        let raw = std::fs::read_to_string(&path).expect("capabilities file should be readable");
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("capabilities file should be JSON");

        let webhook = &parsed["capabilities"]["channel"]["webhook"];
        assert_eq!(
            webhook["secret_validation"].as_str(),
            Some(validation),
            "{name} should declare provider-appropriate webhook validation"
        );
        assert_eq!(
            webhook["secret_header"].as_str(),
            Some(header),
            "{name} should validate the provider-native signature/header"
        );

        let response = parsed["config"]["response"].to_string();
        for fragment in response_fragments {
            assert!(
                response.contains(fragment),
                "{name} response template should include provider payload fragment {fragment}"
            );
        }
    }
}
