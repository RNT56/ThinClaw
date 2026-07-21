use super::*;

const DEFAULT_SOURCE_REPOSITORY: &str = "https://github.com/RNT56/ThinClaw.git";
const MAX_SOURCE_SPEC_BYTES: usize = 16 * 1024;
const MAX_SOURCE_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;

struct ManagedSourceSpec {
    origin: String,
    revision: String,
}

impl DesktopAutonomyManager {
    pub(super) async fn sync_managed_source_clone(&self) -> Result<PathBuf, String> {
        let source = resolve_managed_source_spec().await?;
        let managed_source = self.state_root.join("agent-src");
        match tokio::fs::symlink_metadata(&managed_source).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("managed autonomy source is not a real directory".to_string());
            }
            Ok(_) => {
                let actual_origin = run_cmd(
                    Command::new("git")
                        .arg("-C")
                        .arg(&managed_source)
                        .arg("remote")
                        .arg("get-url")
                        .arg("origin"),
                )
                .await?;
                if actual_origin.trim() != source.origin {
                    return Err(format!(
                        "managed autonomy source origin changed (expected {}, found {})",
                        source.origin,
                        actual_origin.trim()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let scratch = tempfile::Builder::new()
                    .prefix(".agent-source-")
                    .tempdir_in(&self.state_root)
                    .map_err(|error| {
                        format!("failed to create source-clone staging dir: {error}")
                    })?;
                let staged_source = scratch.path().join("repository");
                run_cmd(
                    Command::new("git")
                        .arg("clone")
                        .arg("--no-hardlinks")
                        .arg("--")
                        .arg(&source.origin)
                        .arg(&staged_source),
                )
                .await?;
                verify_thinclaw_source_checkout(&staged_source)?;
                thinclaw_platform::rename_no_replace(&staged_source, &managed_source)
                    .map_err(|error| format!("failed to publish managed source clone: {error}"))?;
            }
            Err(error) => return Err(format!("failed to inspect managed source clone: {error}")),
        }

        verify_thinclaw_source_checkout(&managed_source)?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("fetch")
                .arg("--force")
                .arg("--prune")
                .arg("origin")
                .arg(&source.revision),
        )
        .await?;
        let target = run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("rev-parse")
                .arg("--verify")
                .arg("FETCH_HEAD^{commit}"),
        )
        .await?;
        let target = target.trim();
        validate_source_revision(target)?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("reset")
                .arg("--hard")
                .arg(target),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("clean")
                .arg("-fdx"),
        )
        .await?;
        verify_thinclaw_source_checkout(&managed_source)?;
        Ok(managed_source)
    }
}

async fn resolve_managed_source_spec() -> Result<ManagedSourceSpec, String> {
    if let Ok(origin) = std::env::var("THINCLAW_AUTONOMY_SOURCE") {
        let origin = normalize_source_origin(&origin)?;
        let revision =
            std::env::var("THINCLAW_AUTONOMY_SOURCE_REF").unwrap_or_else(|_| "HEAD".to_string());
        validate_source_revision(&revision)?;
        return Ok(ManagedSourceSpec { origin, revision });
    }

    let mut candidates = Vec::new();
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current);
    }
    let compile_time_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if !candidates.contains(&compile_time_root) {
        candidates.push(compile_time_root);
    }
    for candidate in candidates {
        let root = match run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&candidate)
                .arg("rev-parse")
                .arg("--show-toplevel"),
        )
        .await
        {
            Ok(root) => PathBuf::from(root.trim()),
            Err(_) => continue,
        };
        let root = match root.canonicalize() {
            Ok(root) => root,
            Err(_) => continue,
        };
        if verify_thinclaw_source_checkout(&root).is_err() {
            continue;
        }
        let revision = run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .arg("rev-parse")
                .arg("--verify")
                .arg("HEAD^{commit}"),
        )
        .await?
        .trim()
        .to_string();
        validate_source_revision(&revision)?;
        return Ok(ManagedSourceSpec {
            origin: root.to_string_lossy().to_string(),
            revision,
        });
    }

    Ok(ManagedSourceSpec {
        origin: DEFAULT_SOURCE_REPOSITORY.to_string(),
        revision: format!("v{}", env!("CARGO_PKG_VERSION")),
    })
}

pub(crate) async fn resolve_thinclaw_source_for_learning() -> Result<(String, String), String> {
    let source = resolve_managed_source_spec().await?;
    Ok((source.origin, source.revision))
}

fn normalize_source_origin(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_SOURCE_SPEC_BYTES
        || value.chars().any(char::is_control)
        || value.starts_with('-')
    {
        return Err("THINCLAW_AUTONOMY_SOURCE is malformed or oversized".to_string());
    }
    if value.contains("://") {
        let parsed = url::Url::parse(value)
            .map_err(|error| format!("invalid autonomy source URL: {error}"))?;
        if parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(
                "remote autonomy sources must use HTTPS without credentials, query, or fragment"
                    .to_string(),
            );
        }
        return Ok(parsed.to_string());
    }
    let path = PathBuf::from(value)
        .canonicalize()
        .map_err(|error| format!("failed to resolve autonomy source path: {error}"))?;
    verify_thinclaw_source_checkout(&path)?;
    Ok(path.to_string_lossy().to_string())
}

fn validate_source_revision(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value.starts_with('-')
        || value.contains("..")
        || value.contains("@{")
        || value.ends_with('.')
        || value.ends_with('/')
        || value.chars().any(char::is_control)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err("autonomy source revision is malformed".to_string());
    }
    Ok(())
}

fn verify_thinclaw_source_checkout(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect source checkout: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("ThinClaw source checkout is not a real directory".to_string());
    }
    let manifest_path = path.join("Cargo.toml");
    let manifest_metadata = std::fs::symlink_metadata(&manifest_path)
        .map_err(|error| format!("failed to inspect source Cargo.toml: {error}"))?;
    if manifest_metadata.file_type().is_symlink()
        || !manifest_metadata.is_file()
        || manifest_metadata.len() > MAX_SOURCE_MANIFEST_BYTES
    {
        return Err("source Cargo.toml is not a bounded regular file".to_string());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut manifest_file = options
        .open(&manifest_path)
        .map_err(|error| format!("failed to open source Cargo.toml: {error}"))?;
    let opened_metadata = manifest_file
        .metadata()
        .map_err(|error| format!("failed to re-inspect source Cargo.toml: {error}"))?;
    if !opened_metadata.is_file()
        || opened_metadata.len() != manifest_metadata.len()
        || opened_metadata.len() > MAX_SOURCE_MANIFEST_BYTES
    {
        return Err("source Cargo.toml changed while opening".to_string());
    }
    let mut raw = Vec::with_capacity(
        usize::try_from(opened_metadata.len())
            .map_err(|_| "source Cargo.toml is too large for this platform".to_string())?,
    );
    let mut limited_manifest =
        std::io::Read::take(&mut manifest_file, MAX_SOURCE_MANIFEST_BYTES + 1);
    std::io::Read::read_to_end(&mut limited_manifest, &mut raw)
        .map_err(|error| format!("failed to read source Cargo.toml: {error}"))?;
    if u64::try_from(raw.len()).ok() != Some(opened_metadata.len()) {
        return Err("source Cargo.toml changed while reading".to_string());
    }
    let raw =
        String::from_utf8(raw).map_err(|_| "source Cargo.toml is not valid UTF-8".to_string())?;
    let manifest: toml::Value =
        toml::from_str(&raw).map_err(|error| format!("invalid source Cargo.toml: {error}"))?;
    if manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        != Some("thinclaw")
    {
        return Err("source checkout is not a ThinClaw repository".to_string());
    }
    Ok(())
}
