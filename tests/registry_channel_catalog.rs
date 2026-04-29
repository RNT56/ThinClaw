use std::collections::HashSet;
use std::path::Path;

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
    }
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
