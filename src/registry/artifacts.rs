//! Unified WASM artifact resolution: find, build, and install WASM components.
//!
//! This module consolidates all WASM artifact logic that was previously duplicated
//! across `cli/tool.rs`, `registry/installer.rs`, `extensions/manager.rs`,
//! `channels/wasm/bundled.rs`, and `tools/wasm/loader.rs`.
//!
//! # Functions
//!
//! - [`resolve_target_dir`] — resolve the cargo target directory for a crate
//! - [`find_wasm_artifact`] — find a compiled `.wasm` by crate name across all triples
//! - [`find_any_wasm_artifact`] — find any `.wasm` file (fallback when name is unknown)
//! - [`build_wasm_component`] — async build via `cargo component build`
//! - [`build_wasm_component_sync`] — sync build for CLI use
//! - [`install_wasm_files`] — copy `.wasm` + optional `.capabilities.json` to install dir

use std::path::{Path, PathBuf};

/// Failure modes while compiling a source extension into a WASM component.
#[derive(Debug, thiserror::Error)]
pub enum WasmBuildError {
    #[error("cargo-component is not available")]
    ToolchainUnavailable,

    #[error("failed to probe cargo-component: {0}")]
    ToolchainProbe(String),

    #[error("WASM component build failed: {0}")]
    Build(String),

    #[error("built WASM artifact was not found: {0}")]
    ArtifactNotFound(String),

    #[error("WASM component build worker panicked")]
    WorkerPanicked,

    #[error("failed to initialize the WASM build runtime: {0}")]
    Runtime(#[from] std::io::Error),
}

/// WASM target triples to search, in priority order.
const WASM_TRIPLES: &[&str] = &[
    "wasm32-wasip1",
    "wasm32-wasip2",
    "wasm32-wasi",
    "wasm32-unknown-unknown",
];

/// Resolve the cargo target directory for a crate.
///
/// Checks (in order):
/// 1. `CARGO_TARGET_DIR` env var (shared target dir)
/// 2. `<crate_dir>/target/` (default per-crate layout)
pub fn resolve_target_dir(crate_dir: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let p = PathBuf::from(dir);
        // Resolve relative CARGO_TARGET_DIR against crate_dir
        if p.is_relative() {
            return crate_dir.join(p);
        }
        return p;
    }
    crate_dir.join("target")
}

/// Find a compiled WASM artifact by searching across all target triples.
///
/// Tries exact name match first (with hyphen-to-underscore normalization),
/// then falls back to searching in whichever target directory exists.
/// `profile` is `"release"` or `"debug"`.
pub fn find_wasm_artifact(crate_dir: &Path, crate_name: &str, profile: &str) -> Option<PathBuf> {
    let target_base = resolve_target_dir(crate_dir);
    let snake_name = crate_name.replace('-', "_");

    // Try exact name match in each target triple directory
    for triple in WASM_TRIPLES {
        let dir = target_base.join(triple).join(profile);
        let candidates = [
            dir.join(format!("{}.wasm", crate_name)),
            dir.join(format!("{}.wasm", snake_name)),
        ];
        for candidate in &candidates {
            if std::fs::symlink_metadata(candidate)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                return Some(candidate.clone());
            }
        }
    }

    None
}

/// Find any `.wasm` file in the target dirs (fallback when crate name is unknown).
///
/// Returns the artifact only when exactly one regular, non-symlink `.wasm`
/// exists in the first target triple containing any candidates. Ambiguous
/// build output is rejected rather than installing an arbitrary workspace
/// member based on filesystem iteration order.
pub fn find_any_wasm_artifact(crate_dir: &Path, profile: &str) -> Option<PathBuf> {
    let target_base = resolve_target_dir(crate_dir);

    for triple in WASM_TRIPLES {
        let dir = target_base.join(triple).join(profile);
        if !dir.is_dir() {
            continue;
        }
        let mut candidates = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let file_type = entry.file_type().ok()?;
                (path.extension().is_some_and(|ext| ext == "wasm")
                    && file_type.is_file()
                    && !file_type.is_symlink())
                .then_some(path)
            })
            .collect::<Vec<_>>();
        candidates.sort();
        match candidates.as_slice() {
            [] => {}
            [only] => return Some(only.clone()),
            _ => return None,
        }
    }

    None
}

/// Build a WASM component using `cargo-component` (async).
///
/// Runs in an owned descendant process group with bounded output and a hard
/// deadline. Returns the path to the built artifact.
pub async fn build_wasm_component(
    source_dir: &Path,
    crate_name: &str,
    release: bool,
) -> Result<PathBuf, WasmBuildError> {
    run_component_build(source_dir, release).await?;

    let profile = if release { "release" } else { "debug" };
    let wasm_filename = format!("{}.wasm", crate_name.replace('-', "_"));

    // Look for the specific crate's WASM file across target triples
    find_wasm_artifact(source_dir, wasm_filename.trim_end_matches(".wasm"), profile)
        .or_else(|| {
            // Fall back: search by crate_name directly
            find_wasm_artifact(source_dir, crate_name, profile)
        })
        .ok_or_else(|| {
            WasmBuildError::ArtifactNotFound(format!(
                "could not find {} in {}/target/*/{}/ after build",
                wasm_filename,
                source_dir.display(),
                profile,
            ))
        })
}

/// Build a WASM component using `cargo-component` (sync, for CLI use).
///
/// Returns the path to the built artifact.
pub fn build_wasm_component_sync(
    source_dir: &Path,
    release: bool,
) -> Result<PathBuf, WasmBuildError> {
    println!("Building WASM component in {}...", source_dir.display());

    println!(
        "  Running: cargo component build{}",
        if release { " --release" } else { "" }
    );

    // Run the async owned-process implementation on a dedicated thread. This
    // remains safe even if a legacy caller invokes the sync API from a Tokio
    // worker, and it never blocks that runtime's reactor with child I/O.
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(WasmBuildError::Runtime)?;
                runtime.block_on(run_component_build(source_dir, release))
            })
            .join()
            .map_err(|_| WasmBuildError::WorkerPanicked)?
    })?;

    let profile = if release { "release" } else { "debug" };

    // Find the built artifact
    find_any_wasm_artifact(source_dir, profile).ok_or_else(|| {
        WasmBuildError::ArtifactNotFound(format!(
            "no unique .wasm file found after build in {}/target/*/{}",
            source_dir.display(),
            profile,
        ))
    })
}

async fn run_component_build(source_dir: &Path, release: bool) -> Result<(), WasmBuildError> {
    use tokio::process::Command;

    const TOOLCHAIN_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
    const BUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
    const PROBE_OUTPUT_LIMIT: usize = 64 * 1024;
    const BUILD_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

    let mut check_command = Command::new("cargo");
    check_command.args(["component", "--version"]);
    let check = thinclaw_platform::bounded_command_output(
        &mut check_command,
        TOOLCHAIN_PROBE_TIMEOUT,
        PROBE_OUTPUT_LIMIT,
        PROBE_OUTPUT_LIMIT,
    )
    .await
    .map_err(|error| match error {
        thinclaw_platform::BoundedProcessError::Spawn(_) => WasmBuildError::ToolchainUnavailable,
        other => WasmBuildError::ToolchainProbe(other.to_string()),
    })?;
    if !check.status.success() {
        return Err(WasmBuildError::ToolchainUnavailable);
    }

    let mut command = Command::new("cargo");
    command.current_dir(source_dir).args(["component", "build"]);
    if release {
        command.arg("--release");
    }
    let output = thinclaw_platform::bounded_command_output(
        &mut command,
        BUILD_TIMEOUT,
        BUILD_OUTPUT_LIMIT,
        BUILD_OUTPUT_LIMIT,
    )
    .await
    .map_err(|error| WasmBuildError::Build(error.to_string()))?;
    if !output.status.success() {
        let stderr = bounded_output_preview(&output.stderr, 16 * 1024);
        return Err(WasmBuildError::Build(format!(
            "process exited with {}: {}",
            output.status, stderr
        )));
    }
    Ok(())
}

fn bounded_output_preview(bytes: &[u8], limit: usize) -> String {
    if bytes.len() <= limit {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let retained = &bytes[..limit];
    format!(
        "{}\n[build output truncated after {limit} bytes]",
        String::from_utf8_lossy(retained)
    )
}

/// Copy WASM binary + optional `capabilities.json` sidecar to an install directory.
///
/// Looks for capabilities files in `source_dir` matching several naming conventions.
/// Returns the destination wasm path.
pub async fn install_wasm_files(
    wasm_src: &Path,
    source_dir: &Path,
    name: &str,
    target_dir: &Path,
    kind: crate::registry::manifest::ManifestKind,
    force: bool,
) -> anyhow::Result<PathBuf> {
    const MAX_WASM_BYTES: usize = 50 * 1024 * 1024;
    const MAX_CAPABILITIES_BYTES: usize = 1024 * 1024;
    if name.is_empty()
        || name.len() > 128
        || matches!(name, "." | "..")
        || name.contains("..")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        anyhow::bail!("invalid WASM extension name");
    }

    let wasm_dst = target_dir.join(format!("{}.wasm", name));
    let caps_dst = target_dir.join(format!("{}.capabilities.json", name));

    match tokio::fs::symlink_metadata(&wasm_dst).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            anyhow::bail!("existing WASM destination is not a regular file");
        }
        Ok(_) if !force => {
            anyhow::bail!(
                "Tool '{}' already exists at {}. Use --force to overwrite.",
                name,
                wasm_dst.display()
            );
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let wasm = super::installer::read_regular_file_bounded(wasm_src.to_path_buf(), MAX_WASM_BYTES)
        .await?
        .ok_or_else(|| anyhow::anyhow!("WASM build artifact does not exist"))?;
    super::installer::validate_wasm_payload(&wasm, &wasm_src.display().to_string())?;

    // Look for capabilities.json sidecar in the source directory.
    // Prefer manifest-name variants first, then fall back to wasm artifact stem
    // in case manifest name and built artifact basename diverge.
    let wasm_stem = wasm_src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string();
    let mut caps_candidates = vec![
        source_dir.join(format!("{}.capabilities.json", name)),
        source_dir.join(format!("{}-tool.capabilities.json", name)),
    ];
    if wasm_stem != name {
        caps_candidates.push(source_dir.join(format!("{}.capabilities.json", wasm_stem)));
        caps_candidates.push(source_dir.join(format!("{}-tool.capabilities.json", wasm_stem)));
    }
    caps_candidates.push(source_dir.join("capabilities.json"));
    let mut capabilities = None;
    for caps_src in caps_candidates {
        if caps_src == caps_dst {
            continue;
        }
        match super::installer::read_regular_file_bounded(caps_src.clone(), MAX_CAPABILITIES_BYTES)
            .await
        {
            Ok(Some(bytes)) => {
                super::installer::validate_capabilities_payload(
                    kind,
                    name,
                    &bytes,
                    &caps_src.display().to_string(),
                )?;
                capabilities = Some(bytes);
                break;
            }
            Ok(None) => {}
            Err(error) => return Err(error.into()),
        }
    }

    super::installer::publish_extension_files(
        wasm_dst.clone(),
        caps_dst,
        wasm,
        capabilities,
        force,
    )
    .await?;

    Ok(wasm_dst)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn with_default_target_dir<T>(f: impl FnOnce() -> T) -> T {
        let _env_guard = crate::config::helpers::lock_env();
        // SAFETY: guarded by crate-wide ENV_MUTEX.
        unsafe {
            std::env::remove_var("CARGO_TARGET_DIR");
        }
        f()
    }

    #[test]
    fn test_resolve_target_dir_default() {
        with_default_target_dir(|| {
            // When CARGO_TARGET_DIR is not set, should return <crate_dir>/target
            let dir = Path::new("/some/crate");
            let result = resolve_target_dir(dir);
            assert!(result.ends_with("target"));
        });
    }

    #[test]
    fn test_find_wasm_artifact_not_found() {
        with_default_target_dir(|| {
            let dir = TempDir::new().unwrap();
            assert!(find_wasm_artifact(dir.path(), "nonexistent", "release").is_none());
        });
    }

    #[test]
    fn test_find_wasm_artifact_found() {
        with_default_target_dir(|| {
            let dir = TempDir::new().unwrap();
            let target_base = resolve_target_dir(dir.path());
            let wasm_dir = target_base.join("wasm32-wasip2/release");
            std::fs::create_dir_all(&wasm_dir).unwrap();
            std::fs::File::create(wasm_dir.join("my_tool.wasm")).unwrap();

            let result = find_wasm_artifact(dir.path(), "my_tool", "release");
            assert!(result.is_some());
            assert!(result.unwrap().ends_with("my_tool.wasm"));
        });
    }

    #[test]
    fn test_find_wasm_artifact_hyphen_to_underscore() {
        with_default_target_dir(|| {
            let dir = TempDir::new().unwrap();
            let target_base = resolve_target_dir(dir.path());
            let wasm_dir = target_base.join("wasm32-wasip1/release");
            std::fs::create_dir_all(&wasm_dir).unwrap();
            std::fs::File::create(wasm_dir.join("my_tool.wasm")).unwrap();

            // Search with hyphens, should find underscore version
            let result = find_wasm_artifact(dir.path(), "my-tool", "release");
            assert!(result.is_some());
        });
    }

    #[test]
    fn test_find_any_wasm_artifact_found() {
        with_default_target_dir(|| {
            let dir = TempDir::new().unwrap();
            let target_base = resolve_target_dir(dir.path());
            let wasm_dir = target_base.join("wasm32-wasip2/release");
            std::fs::create_dir_all(&wasm_dir).unwrap();
            std::fs::File::create(wasm_dir.join("something.wasm")).unwrap();

            let result = find_any_wasm_artifact(dir.path(), "release");
            assert!(result.is_some());
        });
    }

    #[test]
    fn test_find_any_wasm_artifact_not_found() {
        with_default_target_dir(|| {
            let dir = TempDir::new().unwrap();
            assert!(find_any_wasm_artifact(dir.path(), "release").is_none());
        });
    }

    #[test]
    fn test_find_any_wasm_artifact_rejects_ambiguous_output() {
        with_default_target_dir(|| {
            let dir = TempDir::new().unwrap();
            let wasm_dir = resolve_target_dir(dir.path()).join("wasm32-wasip2/release");
            std::fs::create_dir_all(&wasm_dir).unwrap();
            std::fs::write(wasm_dir.join("first.wasm"), b"wasm").unwrap();
            std::fs::write(wasm_dir.join("second.wasm"), b"wasm").unwrap();

            assert!(find_any_wasm_artifact(dir.path(), "release").is_none());
        });
    }

    #[cfg(unix)]
    #[test]
    fn test_artifact_discovery_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        with_default_target_dir(|| {
            let dir = TempDir::new().unwrap();
            let wasm_dir = resolve_target_dir(dir.path()).join("wasm32-wasip2/release");
            std::fs::create_dir_all(&wasm_dir).unwrap();
            let outside = dir.path().join("outside.wasm");
            std::fs::write(&outside, b"wasm").unwrap();
            symlink(&outside, wasm_dir.join("linked.wasm")).unwrap();

            assert!(find_wasm_artifact(dir.path(), "linked", "release").is_none());
            assert!(find_any_wasm_artifact(dir.path(), "release").is_none());
        });
    }

    #[tokio::test]
    async fn test_install_wasm_files_copies() {
        let src_dir = TempDir::new().unwrap();
        let target_dir = TempDir::new().unwrap();

        let wasm_src = src_dir.path().join("test.wasm");
        tokio::fs::write(&wasm_src, b"\0asm\x01\x00\x00\x00")
            .await
            .unwrap();

        // Create a capabilities file
        let caps_src = src_dir.path().join("mytool.capabilities.json");
        tokio::fs::write(&caps_src, b"{}").await.unwrap();

        let result = install_wasm_files(
            &wasm_src,
            src_dir.path(),
            "mytool",
            target_dir.path(),
            crate::registry::manifest::ManifestKind::Tool,
            false,
        )
        .await;

        assert!(result.is_ok());
        let wasm_dst = result.unwrap();
        assert!(wasm_dst.exists());
        assert!(target_dir.path().join("mytool.capabilities.json").exists());
    }

    #[tokio::test]
    async fn test_install_wasm_files_refuses_overwrite() {
        let src_dir = TempDir::new().unwrap();
        let target_dir = TempDir::new().unwrap();

        let wasm_src = src_dir.path().join("test.wasm");
        tokio::fs::write(&wasm_src, b"\0asm").await.unwrap();

        // Pre-create the target
        let existing = target_dir.path().join("mytool.wasm");
        tokio::fs::write(&existing, b"existing").await.unwrap();

        let result = install_wasm_files(
            &wasm_src,
            src_dir.path(),
            "mytool",
            target_dir.path(),
            crate::registry::manifest::ManifestKind::Tool,
            false,
        )
        .await;

        assert!(result.is_err());
    }

    #[test]
    fn test_wasm_triples_order() {
        // Verify the order is as documented
        assert_eq!(WASM_TRIPLES[0], "wasm32-wasip1");
        assert_eq!(WASM_TRIPLES[1], "wasm32-wasip2");
        assert_eq!(WASM_TRIPLES[2], "wasm32-wasi");
        assert_eq!(WASM_TRIPLES[3], "wasm32-unknown-unknown");
    }
}
