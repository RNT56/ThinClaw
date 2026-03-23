//! WASM binaries embedded at compile time (`--features bundled-wasm`).
//!
//! When compiled with the `bundled-wasm` feature, all WASM extensions (tools and
//! channels) are embedded into the binary via `include_bytes!`. This module
//! provides functions to extract them to disk at install time, enabling
//! **air-gapped / zero-network deployments** on headless machines like Mac Minis.
//!
//! The default build (without this feature) does NOT include any embedded WASM
//! binaries — extensions are downloaded from GitHub Releases instead.
//!
//! # Usage
//!
//! ```bash
//! # Standard build — extensions downloaded at install time
//! cargo build --release
//!
//! # Air-gapped build — all extensions embedded in binary
//! cargo build --release --features bundled-wasm
//! ```

/// Whether bundled WASM support is compiled in.
pub fn is_available() -> bool {
    cfg!(feature = "bundled-wasm")
}

/// Auto-generated entries from `build.rs` when `bundled-wasm` feature is active.
///
/// Each entry is `(name, kind, wasm_bytes, caps_bytes)` where:
/// - `name`: extension name (e.g. "telegram", "github")
/// - `kind`: "tool" or "channel"
/// - `wasm_bytes`: the compiled WASM binary
/// - `caps_bytes`: optional capabilities JSON sidecar
#[cfg(feature = "bundled-wasm")]
include!(concat!(env!("OUT_DIR"), "/bundled_wasm_entries.rs"));

/// Stub constant when feature is disabled — empty array.
#[cfg(not(feature = "bundled-wasm"))]
pub const BUNDLED_ENTRIES: &[(&str, &str, &[u8], Option<&[u8]>)] = &[];

/// List names of all bundled extensions.
pub fn bundled_names() -> Vec<&'static str> {
    BUNDLED_ENTRIES
        .iter()
        .map(|(name, _, _, _)| *name)
        .collect()
}

/// List names of bundled tools only.
pub fn bundled_tool_names() -> Vec<&'static str> {
    BUNDLED_ENTRIES
        .iter()
        .filter(|(_, kind, _, _)| *kind == "tool")
        .map(|(name, _, _, _)| *name)
        .collect()
}

/// List names of bundled channels only.
pub fn bundled_channel_names() -> Vec<&'static str> {
    BUNDLED_ENTRIES
        .iter()
        .filter(|(_, kind, _, _)| *kind == "channel")
        .map(|(name, _, _, _)| *name)
        .collect()
}

/// Check if a specific extension is bundled.
pub fn is_bundled(name: &str) -> bool {
    BUNDLED_ENTRIES.iter().any(|(n, _, _, _)| *n == name)
}

/// Extract a bundled WASM extension to the target directory.
///
/// Writes `<name>.wasm` and optionally `<name>.capabilities.json` to `target_dir`.
/// Returns `Ok(())` on success. Overwrites any existing files.
pub async fn extract_bundled(name: &str, target_dir: &std::path::Path) -> Result<(), String> {
    let bundle = BUNDLED_ENTRIES
        .iter()
        .find(|(n, _, _, _)| *n == name)
        .ok_or_else(|| format!("No bundled WASM for '{}' in this build", name))?;

    tokio::fs::create_dir_all(target_dir)
        .await
        .map_err(|e| format!("Failed to create directory {}: {}", target_dir.display(), e))?;

    let wasm_path = target_dir.join(format!("{}.wasm", name));
    tokio::fs::write(&wasm_path, bundle.2)
        .await
        .map_err(|e| format!("Failed to write {}: {}", wasm_path.display(), e))?;

    if let Some(caps) = bundle.3 {
        let caps_path = target_dir.join(format!("{}.capabilities.json", name));
        tokio::fs::write(&caps_path, caps)
            .await
            .map_err(|e| format!("Failed to write {}: {}", caps_path.display(), e))?;
    }

    tracing::info!(
        "Extracted bundled WASM '{}' ({} bytes) to {}",
        name,
        bundle.2.len(),
        target_dir.display()
    );

    Ok(())
}

/// Extract ALL bundled extensions to the appropriate directories.
///
/// Tools go to `tools_dir`, channels go to `channels_dir`.
/// Skips extensions that are already installed (file exists) unless `force` is true.
pub async fn extract_all(
    tools_dir: &std::path::Path,
    channels_dir: &std::path::Path,
    force: bool,
) -> Result<ExtractSummary, String> {
    let mut summary = ExtractSummary::default();

    for (name, kind, _, _) in BUNDLED_ENTRIES {
        let target_dir = match *kind {
            "tool" => tools_dir,
            "channel" => channels_dir,
            _ => continue,
        };

        let wasm_path = target_dir.join(format!("{}.wasm", name));
        if wasm_path.exists() && !force {
            summary.skipped += 1;
            continue;
        }

        match extract_bundled(name, target_dir).await {
            Ok(()) => summary.extracted += 1,
            Err(e) => {
                tracing::warn!("Failed to extract bundled '{}': {}", name, e);
                summary.failed += 1;
            }
        }
    }

    Ok(summary)
}

/// Summary of a bulk extraction operation.
#[derive(Debug, Default)]
pub struct ExtractSummary {
    /// Extensions successfully extracted.
    pub extracted: usize,
    /// Extensions skipped because already installed.
    pub skipped: usize,
    /// Extensions that failed to extract.
    pub failed: usize,
}

impl std::fmt::Display for ExtractSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} extracted, {} skipped (already installed), {} failed",
            self.extracted, self.skipped, self.failed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_available() {
        // In default test builds (without bundled-wasm feature), should be false
        let expected = cfg!(feature = "bundled-wasm");
        assert_eq!(is_available(), expected);
    }

    #[test]
    fn test_bundled_entries_consistent() {
        // All entries should have non-empty names and valid kinds
        for (name, kind, wasm, _caps) in BUNDLED_ENTRIES {
            assert!(!name.is_empty(), "Entry name must not be empty");
            assert!(
                *kind == "tool" || *kind == "channel",
                "Invalid kind '{}' for '{}'",
                kind,
                name
            );
            assert!(
                !wasm.is_empty(),
                "WASM bytes must not be empty for '{}'",
                name
            );
        }
    }

    #[test]
    fn test_bundled_names_returns_all() {
        let all = bundled_names();
        let tools = bundled_tool_names();
        let channels = bundled_channel_names();
        assert_eq!(all.len(), tools.len() + channels.len());
    }

    #[test]
    fn test_is_bundled_with_nonexistent() {
        assert!(!is_bundled("nonexistent_extension_xyz"));
    }

    #[cfg(feature = "bundled-wasm")]
    #[tokio::test]
    async fn test_extract_bundled_writes_files() {
        let dir = tempfile::tempdir().unwrap();

        // Should have at least one bundled extension when feature is active
        if let Some((name, _, _, _)) = BUNDLED_ENTRIES.first() {
            let result = extract_bundled(name, dir.path()).await;
            assert!(result.is_ok(), "Extract failed: {:?}", result.err());

            let wasm_path = dir.path().join(format!("{}.wasm", name));
            assert!(
                wasm_path.exists(),
                "WASM file should exist after extraction"
            );
        }
    }
}
