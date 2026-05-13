use std::path::{Path, PathBuf};

const EXTRACTED_FACADES: &[(&str, &str)] = &[
    (
        "src/channels/channel.rs",
        "crates/thinclaw-channels-core/src/channel.rs",
    ),
    (
        "src/channels/manager.rs",
        "crates/thinclaw-channels/src/manager.rs",
    ),
    (
        "src/channels/gmail.rs",
        "crates/thinclaw-channels/src/gmail.rs",
    ),
    (
        "src/channels/wasm/capabilities.rs",
        "crates/thinclaw-channels/src/wasm/capabilities.rs",
    ),
    (
        "src/channels/wasm/host.rs",
        "crates/thinclaw-channels/src/wasm/host.rs",
    ),
    (
        "src/channels/wasm/loader.rs",
        "crates/thinclaw-channels/src/wasm/loader.rs",
    ),
    (
        "src/channels/wasm/wrapper.rs",
        "crates/thinclaw-channels/src/wasm/wrapper.rs",
    ),
    (
        "src/document_extraction/mod.rs",
        "crates/thinclaw-media/src/document_extraction/mod.rs",
    ),
    ("src/media/cache.rs", "crates/thinclaw-media/src/cache.rs"),
    ("src/media/limits.rs", "crates/thinclaw-media/src/limits.rs"),
    (
        "src/tools/wasm/allowlist.rs",
        "crates/thinclaw-tools/src/wasm/allowlist.rs",
    ),
    (
        "src/tools/wasm/capabilities.rs",
        "crates/thinclaw-tools/src/wasm/capabilities.rs",
    ),
    (
        "src/tools/wasm/capabilities_schema.rs",
        "crates/thinclaw-tools/src/wasm/capabilities_schema.rs",
    ),
    (
        "src/tools/wasm/credential_injector.rs",
        "crates/thinclaw-tools/src/wasm/credential_injector.rs",
    ),
    (
        "src/tools/wasm/error.rs",
        "crates/thinclaw-tools/src/wasm/error.rs",
    ),
    (
        "src/tools/wasm/host.rs",
        "crates/thinclaw-tools/src/wasm/host.rs",
    ),
    (
        "src/tools/wasm/limits.rs",
        "crates/thinclaw-tools/src/wasm/limits.rs",
    ),
    (
        "src/tools/wasm/rate_limiter.rs",
        "crates/thinclaw-tools/src/wasm/rate_limiter.rs",
    ),
    (
        "src/tools/wasm/runtime.rs",
        "crates/thinclaw-tools/src/wasm/runtime.rs",
    ),
    (
        "src/tools/wasm/storage.rs",
        "crates/thinclaw-tools/src/wasm/storage.rs",
    ),
    (
        "src/tools/wasm/watcher.rs",
        "crates/thinclaw-tools/src/wasm/watcher.rs",
    ),
    (
        "src/tools/wasm/wrapper.rs",
        "crates/thinclaw-tools/src/wasm/wrapper.rs",
    ),
    (
        "src/workspace/workspace_core.rs",
        "crates/thinclaw-workspace/src/workspace_core.rs",
    ),
];

#[test]
fn extracted_root_modules_remain_compatibility_facades() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for (facade, canonical) in EXTRACTED_FACADES {
        let facade_path = root.join(facade);
        let canonical_path = root.join(canonical);
        assert!(
            canonical_path.exists(),
            "{} should exist as canonical crate-owned implementation for {}",
            canonical_path.display(),
            facade
        );
        assert_facade_marker(&facade_path);
    }
}

fn assert_facade_marker(path: &Path) {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} should be readable: {error}", path.display()));
    let header = content.lines().take(3).collect::<Vec<_>>().join("\n");
    assert!(
        header.contains("Compatibility facade") || header.contains("compatibility facade"),
        "{} must remain a root compatibility facade, not a new implementation home",
        path.display()
    );
}
