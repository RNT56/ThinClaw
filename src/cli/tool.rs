//! Tool management CLI commands.
//!
//! Commands for installing, listing, removing, and authenticating WASM tools.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Subcommand;
use tokio::fs;

use crate::config::Config;
#[allow(unused_imports)]
use crate::db::Database;
#[cfg(feature = "postgres")]
use crate::secrets::PostgresSecretsStore;
use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::terminal_branding::TerminalBranding;
use crate::tools::wasm::{
    AuthCapabilitySchema, CapabilitiesFile, WasmToolAuthStatus, WasmToolOAuthFlow,
    compute_binary_hash,
};

const MAX_TOOL_WASM_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOOL_CAPABILITIES_BYTES: u64 = 1024 * 1024;
const MAX_TOOL_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_TOOL_DIRECTORY_ENTRIES: usize = 4096;

fn validate_tool_name(name: &str, allow_protected: bool) -> anyhow::Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    if !valid {
        anyhow::bail!(
            "tool name must contain only lowercase ASCII letters, digits, hyphens, or underscores"
        );
    }
    if !allow_protected && thinclaw_tools::registry::PROTECTED_TOOL_NAMES.contains(&name) {
        anyhow::bail!("tool name conflicts with a protected built-in tool");
    }
    Ok(())
}

async fn read_bounded_file(path: &Path, max_bytes: u64, label: &str) -> anyhow::Result<Vec<u8>> {
    thinclaw_platform::read_regular_file_bounded_single_link_async(path.to_path_buf(), max_bytes)
        .await
        .map_err(|error| anyhow::anyhow!("failed to read {label}: {error}"))
}

/// Default tools directory.
fn default_tools_dir() -> PathBuf {
    crate::platform::state_paths().tools_dir
}

#[derive(Subcommand, Debug, Clone)]
pub enum ToolCommand {
    /// Install a WASM tool from source directory or .wasm file
    Install {
        /// Path to tool source directory (with Cargo.toml) or .wasm file
        path: PathBuf,

        /// Tool name (defaults to directory/file name)
        #[arg(short, long)]
        name: Option<String>,

        /// Path to capabilities JSON file (auto-detected if not specified)
        #[arg(long)]
        capabilities: Option<PathBuf>,

        /// Target directory for installation (default: ~/.thinclaw/tools/)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Build in release mode (default: true)
        #[arg(long, default_value = "true")]
        release: bool,

        /// Skip compilation (use existing .wasm file)
        #[arg(long)]
        skip_build: bool,

        /// Force overwrite if tool already exists
        #[arg(short, long)]
        force: bool,
    },

    /// List installed tools
    List {
        /// Directory to list tools from (default: ~/.thinclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// Show detailed information
        #[arg(short, long)]
        verbose: bool,
    },

    /// Remove an installed tool
    Remove {
        /// Name of the tool to remove
        name: String,

        /// Directory to remove tool from (default: ~/.thinclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Show information about a tool
    Info {
        /// Name of the tool or path to .wasm file
        name_or_path: String,

        /// Directory to look for tool (default: ~/.thinclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Configure authentication for a tool
    Auth {
        /// Name of the tool
        name: String,

        /// Directory to look for tool (default: ~/.thinclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// User ID for storing the secret (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },
}

/// Run a tool command.
pub async fn run_tool_command(cmd: ToolCommand) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    match cmd {
        ToolCommand::Install {
            path,
            name,
            capabilities,
            target,
            release,
            skip_build,
            force,
        } => {
            branding.print_banner("Tools", Some("Install a WebAssembly tool"));
            install_tool(path, name, capabilities, target, release, skip_build, force).await
        }
        ToolCommand::List { dir, verbose } => {
            branding.print_banner("Tools", Some("Inspect installed WebAssembly tools"));
            list_tools(dir, verbose).await
        }
        ToolCommand::Remove { name, dir } => {
            branding.print_banner("Tools", Some("Remove a WebAssembly tool"));
            remove_tool(name, dir).await
        }
        ToolCommand::Info { name_or_path, dir } => {
            branding.print_banner("Tools", Some("Inspect a WebAssembly tool"));
            show_tool_info(name_or_path, dir).await
        }
        ToolCommand::Auth { name, dir, user } => auth_tool(name, dir, user).await,
    }
}

/// Install a WASM tool.
async fn install_tool(
    path: PathBuf,
    name: Option<String>,
    capabilities: Option<PathBuf>,
    target: Option<PathBuf>,
    release: bool,
    skip_build: bool,
    force: bool,
) -> anyhow::Result<()> {
    let target_dir = target.unwrap_or_else(default_tools_dir);

    // Determine if path is a directory (source) or .wasm file
    let metadata = fs::symlink_metadata(&path).await?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("tool source must not be a symlink");
    }

    let (wasm_path, tool_name, caps_path) = if metadata.is_dir() {
        // Source directory, need to build
        let cargo_toml = path.join("Cargo.toml");
        if !cargo_toml.exists() {
            anyhow::bail!(
                "No Cargo.toml found in {}. Expected a Rust WASM tool source directory.",
                path.display()
            );
        }

        // Extract tool name from Cargo.toml or use provided name
        let tool_name = if let Some(n) = name {
            n
        } else {
            extract_crate_name(&cargo_toml).await?
        };

        // Build the WASM component if not skipping
        let profile = if release { "release" } else { "debug" };
        let wasm_path = if skip_build {
            // Look for existing wasm file
            crate::registry::artifacts::find_wasm_artifact(&path, &tool_name, profile)
                .or_else(|| crate::registry::artifacts::find_any_wasm_artifact(&path, profile))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No .wasm artifact found. Run without --skip-build to build first."
                    )
                })?
        } else {
            crate::registry::artifacts::build_wasm_component(&path, &tool_name, release).await?
        };

        // Look for capabilities file
        let caps_path = capabilities.or_else(|| {
            let candidates = [
                path.join(format!("{}.capabilities.json", tool_name)),
                path.join("capabilities.json"),
            ];
            candidates.into_iter().find(|p| p.exists())
        });

        (wasm_path, tool_name, caps_path)
    } else if path.extension().map(|e| e == "wasm").unwrap_or(false) {
        // Direct .wasm file
        let tool_name = name.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

        // Look for capabilities file next to wasm
        let caps_path = capabilities.or_else(|| {
            let candidates = [
                path.with_extension("capabilities.json"),
                path.parent()
                    .map(|p| p.join(format!("{}.capabilities.json", tool_name)))
                    .unwrap_or_default(),
            ];
            candidates.into_iter().find(|p| p.exists())
        });

        (path, tool_name, caps_path)
    } else {
        anyhow::bail!(
            "Expected a directory with Cargo.toml or a .wasm file, got: {}",
            path.display()
        );
    };
    validate_tool_name(&tool_name, false)?;

    // Ensure target directory exists
    fs::create_dir_all(&target_dir).await?;
    let target_metadata = fs::symlink_metadata(&target_dir).await?;
    if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
        anyhow::bail!("tool installation target is not a real directory");
    }

    // Target paths
    let target_wasm = target_dir.join(format!("{}.wasm", tool_name));
    let target_caps = target_dir.join(format!("{}.capabilities.json", tool_name));

    let wasm_bytes = read_bounded_file(&wasm_path, MAX_TOOL_WASM_BYTES, "WASM tool").await?;
    if wasm_bytes.len() < 8 || !wasm_bytes.starts_with(b"\0asm") {
        anyhow::bail!("tool input is not a valid-looking WebAssembly module");
    }
    let caps_bytes = if let Some(ref caps) = caps_path {
        let bytes =
            read_bounded_file(caps, MAX_TOOL_CAPABILITIES_BYTES, "tool capabilities").await?;
        CapabilitiesFile::from_bytes(&bytes)
            .map_err(|error| anyhow::anyhow!("invalid tool capabilities: {error}"))?;
        Some(bytes)
    } else {
        None
    };

    println!("Installing {} to {}", tool_name, target_wasm.display());
    thinclaw_platform::publish_file_pair(
        target_wasm.clone(),
        target_caps.clone(),
        wasm_bytes.clone(),
        caps_bytes.clone(),
        if force {
            thinclaw_platform::ExistingPairPolicy::Replace
        } else {
            thinclaw_platform::ExistingPairPolicy::Refuse
        },
    )
    .await
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!("tool already exists; use --force to replace it")
        } else {
            anyhow::anyhow!("failed to publish tool package: {error}")
        }
    })?;

    if caps_bytes.is_some() {
        println!("  Installed validated capabilities");
    } else {
        println!("  Warning: No capabilities file found. Tool will have no permissions.");
    }

    // Calculate and display hash
    let hash = compute_binary_hash(&wasm_bytes);
    let hash_hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();

    println!("\nInstalled successfully:");
    println!("  Name: {}", tool_name);
    println!("  WASM: {}", target_wasm.display());
    println!("  Size: {} bytes", wasm_bytes.len());
    println!("  Hash: {}", &hash_hex[..16]); // Show first 16 chars

    if caps_bytes.is_some() {
        println!("  Caps: {}", target_caps.display());
    }

    Ok(())
}

/// Extract crate name from Cargo.toml.
async fn extract_crate_name(cargo_toml: &Path) -> anyhow::Result<String> {
    let content = read_bounded_file(cargo_toml, MAX_TOOL_MANIFEST_BYTES, "Cargo manifest").await?;
    let manifest: toml::Value = toml::from_str(std::str::from_utf8(&content)?)?;
    manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("Cargo manifest has no package.name"))
}

/// List installed tools.
async fn list_tools(dir: Option<PathBuf>, verbose: bool) -> anyhow::Result<()> {
    let tools_dir = dir.unwrap_or_else(default_tools_dir);

    if !tools_dir.exists() {
        println!("No tools directory found at {}", tools_dir.display());
        println!("Install a tool with: thinclaw tool install <path>");
        return Ok(());
    }
    let directory_metadata = fs::symlink_metadata(&tools_dir).await?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        anyhow::bail!("tools path is not a real directory");
    }

    let mut entries = fs::read_dir(&tools_dir).await?;
    let mut tools = Vec::new();
    let mut entries_seen = 0_usize;

    while let Some(entry) = entries.next_entry().await? {
        entries_seen = entries_seen.saturating_add(1);
        if entries_seen > MAX_TOOL_DIRECTORY_ENTRIES {
            anyhow::bail!("tools directory contains too many entries");
        }
        let path = entry.path();
        let file_type = entry.file_type().await?;
        if file_type.is_file() && path.extension().map(|e| e == "wasm").unwrap_or(false) {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            let caps_path = path.with_extension("capabilities.json");
            let has_caps = fs::symlink_metadata(&caps_path)
                .await
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());

            let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);

            tools.push((name, path, has_caps, size));
        }
    }

    if tools.is_empty() {
        println!("No tools installed in {}", tools_dir.display());
        return Ok(());
    }

    tools.sort_by(|a, b| a.0.cmp(&b.0));

    println!("Installed tools in {}:", tools_dir.display());
    println!();

    for (name, path, has_caps, size) in tools {
        if verbose {
            let wasm_bytes =
                read_bounded_file(&path, MAX_TOOL_WASM_BYTES, "installed tool").await?;
            let hash = compute_binary_hash(&wasm_bytes);
            let hash_hex: String = hash.iter().take(8).map(|b| format!("{:02x}", b)).collect();

            println!("  {} ({})", name, format_size(size));
            println!("    Path: {}", path.display());
            println!("    Hash: {}", hash_hex);
            println!("    Caps: {}", if has_caps { "yes" } else { "no" });

            if has_caps {
                let caps_path = path.with_extension("capabilities.json");
                if let Ok(content) =
                    read_bounded_file(&caps_path, MAX_TOOL_CAPABILITIES_BYTES, "tool capabilities")
                        .await
                    && let Ok(caps) = CapabilitiesFile::from_bytes(&content)
                {
                    print_capabilities_summary(&caps);
                }
            }
            println!();
        } else {
            let caps_indicator = if has_caps { "✓" } else { "✗" };
            println!(
                "  {} ({}, caps: {})",
                name,
                format_size(size),
                caps_indicator
            );
        }
    }

    Ok(())
}

/// Remove an installed tool.
async fn remove_tool(name: String, dir: Option<PathBuf>) -> anyhow::Result<()> {
    validate_tool_name(&name, true)?;
    let tools_dir = dir.unwrap_or_else(default_tools_dir);

    let wasm_path = tools_dir.join(format!("{}.wasm", name));
    let caps_path = tools_dir.join(format!("{}.capabilities.json", name));

    if !fs::symlink_metadata(&wasm_path)
        .await
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        anyhow::bail!("Tool '{}' not found in {}", name, tools_dir.display());
    }

    thinclaw_platform::remove_file_pair(wasm_path.clone(), caps_path.clone()).await?;
    println!("Removed {}", wasm_path.display());
    println!("Removed associated capabilities when present");

    println!("\nTool '{}' removed.", name);
    Ok(())
}

/// Show information about a tool.
async fn show_tool_info(name_or_path: String, dir: Option<PathBuf>) -> anyhow::Result<()> {
    let wasm_path = if name_or_path.ends_with(".wasm") {
        PathBuf::from(&name_or_path)
    } else {
        validate_tool_name(&name_or_path, true)?;
        let tools_dir = dir.unwrap_or_else(default_tools_dir);
        tools_dir.join(format!("{}.wasm", name_or_path))
    };

    let wasm_bytes = read_bounded_file(&wasm_path, MAX_TOOL_WASM_BYTES, "WASM tool").await?;
    let hash = compute_binary_hash(&wasm_bytes);
    let hash_hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();

    let name = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    println!("Tool: {}", name);
    println!("Path: {}", wasm_path.display());
    println!(
        "Size: {} bytes ({})",
        wasm_bytes.len(),
        format_size(wasm_bytes.len() as u64)
    );
    println!("Hash: {}", hash_hex);

    let caps_path = wasm_path.with_extension("capabilities.json");
    if fs::symlink_metadata(&caps_path)
        .await
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        println!("\nCapabilities ({}):", caps_path.display());
        let content =
            read_bounded_file(&caps_path, MAX_TOOL_CAPABILITIES_BYTES, "tool capabilities").await?;
        match CapabilitiesFile::from_bytes(&content) {
            Ok(caps) => print_capabilities_detail(&caps),
            Err(e) => println!("  Error parsing: {}", e),
        }
    } else {
        println!("\nNo capabilities file found.");
        println!("Tool will have no permissions (default deny).");
    }

    Ok(())
}

/// Format bytes as human-readable size.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Print a brief capabilities summary.
fn print_capabilities_summary(caps: &CapabilitiesFile) {
    let mut parts = Vec::new();

    if let Some(ref http) = caps.http {
        let hosts: Vec<_> = http.allowlist.iter().map(|e| e.host.as_str()).collect();
        if !hosts.is_empty() {
            parts.push(format!("http: {}", hosts.join(", ")));
        }
    }

    if let Some(ref secrets) = caps.secrets
        && !secrets.allowed_names.is_empty()
    {
        parts.push(format!("secrets: {}", secrets.allowed_names.len()));
    }

    if let Some(ref ws) = caps.workspace
        && !ws.allowed_prefixes.is_empty()
    {
        parts.push("workspace: read".to_string());
    }

    if !parts.is_empty() {
        println!("    Perms: {}", parts.join(", "));
    }
}

/// Print detailed capabilities.
fn print_capabilities_detail(caps: &CapabilitiesFile) {
    if let Some(ref http) = caps.http {
        println!("  HTTP:");
        for endpoint in &http.allowlist {
            let methods = if endpoint.methods.is_empty() {
                "*".to_string()
            } else {
                endpoint.methods.join(", ")
            };
            let path = endpoint.path_prefix.as_deref().unwrap_or("/*");
            println!("    {} {} {}", methods, endpoint.host, path);
        }

        if !http.credentials.is_empty() {
            println!("  Credentials:");
            for (key, cred) in &http.credentials {
                println!("    {}: {} -> {:?}", key, cred.secret_name, cred.location);
            }
        }

        if let Some(ref rate) = http.rate_limit {
            println!(
                "  Rate limit: {}/min, {}/hour",
                rate.requests_per_minute, rate.requests_per_hour
            );
        }
    }

    if let Some(ref secrets) = caps.secrets
        && !secrets.allowed_names.is_empty()
    {
        println!("  Secrets (existence check only):");
        for name in &secrets.allowed_names {
            println!("    {}", name);
        }
    }

    if let Some(ref tool_invoke) = caps.tool_invoke
        && !tool_invoke.aliases.is_empty()
    {
        println!("  Tool aliases:");
        for (alias, real_name) in &tool_invoke.aliases {
            println!("    {} -> {}", alias, real_name);
        }
    }

    if let Some(ref ws) = caps.workspace
        && !ws.allowed_prefixes.is_empty()
    {
        println!("  Workspace read prefixes:");
        for prefix in &ws.allowed_prefixes {
            println!("    {}", prefix);
        }
    }
}

/// Configure authentication for a tool.
async fn auth_tool(name: String, dir: Option<PathBuf>, user_id: String) -> anyhow::Result<()> {
    validate_tool_name(&name, true)?;
    let branding = TerminalBranding::current();
    let tools_dir = dir.unwrap_or_else(default_tools_dir);
    let caps_path = tools_dir.join(format!("{}.capabilities.json", name));

    if !fs::symlink_metadata(&caps_path)
        .await
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        anyhow::bail!(
            "Tool '{}' not found or has no capabilities file at {}",
            name,
            caps_path.display()
        );
    }

    // Parse capabilities
    let content =
        read_bounded_file(&caps_path, MAX_TOOL_CAPABILITIES_BYTES, "tool capabilities").await?;
    let caps = CapabilitiesFile::from_bytes(&content)
        .map_err(|e| anyhow::anyhow!("Invalid capabilities file: {}", e))?;

    // Check for auth section
    let auth = caps.auth.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "Tool '{}' has no auth configuration.\n\
             The tool may not require authentication, or auth setup is not defined.",
            name
        )
    })?;

    let display_name = auth.display_name.as_deref().unwrap_or(&name);

    branding.print_banner("Tools", Some(&format!("Authenticate {}", display_name)));

    // Initialize secrets store
    let config = Config::from_env().await?;
    let master_key = config.secrets.master_key().ok_or_else(|| {
        anyhow::anyhow!(
            "SECRETS_MASTER_KEY not set. Run 'thinclaw onboard' first or set it in .env"
        )
    })?;

    let crypto = SecretsCrypto::new(master_key.clone())?;

    let secrets_store: Arc<dyn SecretsStore + Send + Sync> = {
        #[cfg(feature = "postgres")]
        {
            let store = crate::db::postgres::PgBackend::new(&config.database).await?;
            store.run_migrations().await?;
            Arc::new(PostgresSecretsStore::new(store.pool(), Arc::new(crypto)))
        }
        #[cfg(all(feature = "libsql", not(feature = "postgres")))]
        {
            use crate::db::Database as _;
            use crate::db::libsql::LibSqlBackend;
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config
                .database
                .libsql_path
                .as_deref()
                .unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.database.libsql_url {
                let token = config.database.libsql_auth_token.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set")
                })?;
                LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?
            } else {
                LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?
            };
            backend
                .run_migrations()
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            Arc::new(crate::secrets::LibSqlSecretsStore::new(
                backend.shared_db(),
                Arc::new(crypto),
            ))
        }
        #[cfg(not(any(feature = "postgres", feature = "libsql")))]
        {
            let _ = crypto;
            anyhow::bail!(
                "No database backend available for secrets. Enable 'postgres' or 'libsql' feature."
            );
        }
    };

    let oauth_flow = WasmToolOAuthFlow::new(secrets_store.as_ref(), &user_id, &tools_dir);
    let already_configured = oauth_flow
        .check_auth_status(&auth)
        .await
        .map(|status| {
            matches!(
                status.auth_status,
                WasmToolAuthStatus::Authenticated
                    | WasmToolAuthStatus::NeedsReauth
                    | WasmToolAuthStatus::InsufficientScope
            )
        })
        .unwrap_or(false);

    if already_configured {
        println!("  {} is already configured.", display_name);
        println!();
        print!("  Replace existing credentials? [y/N]: ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!();
            println!("  Keeping existing credentials.");
            return Ok(());
        }
        println!();
    }

    // Check for environment variable
    if let Some(ref env_var) = auth.env_var
        && let Ok(token) = std::env::var(env_var)
        && !token.is_empty()
    {
        println!("  Found {} in environment.", env_var);
        println!();

        // Validate if endpoint is provided
        if let Some(ref validation) = auth.validation_endpoint {
            print!("  Validating token...");
            std::io::stdout().flush()?;

            match validate_token(&token, validation, &auth.secret_name, &caps).await {
                Ok(()) => {
                    println!(" ✓");
                }
                Err(e) => {
                    println!(" ✗");
                    println!("  Validation failed: {}", e);
                    println!();
                    println!("  Falling back to manual entry...");
                    return auth_tool_manual(secrets_store.as_ref(), &user_id, &auth, &caps).await;
                }
            }
        }

        oauth_flow.store_manual_token(&auth, &token).await?;
        print_success(display_name);
        return Ok(());
    }

    // Check for OAuth configuration
    if auth.oauth.is_some() {
        if let Some(resolved) = oauth_flow.combined_oauth_config(&auth).await? {
            let base_scope_count = auth
                .oauth
                .as_ref()
                .map(|oauth| oauth.scopes.len())
                .unwrap_or_default();
            if resolved.required_scopes.len() > base_scope_count {
                let extra = resolved.required_scopes.len() - base_scope_count;
                println!(
                    "  Including scopes from {} other installed tool(s) sharing this credential.",
                    extra
                );
                println!();
            }
        }
        return auth_tool_oauth(&oauth_flow, &auth).await;
    }

    // Fall back to manual entry
    auth_tool_manual(secrets_store.as_ref(), &user_id, &auth, &caps).await
}

/// OAuth browser-based login flow.
async fn auth_tool_oauth(
    flow: &WasmToolOAuthFlow<'_>,
    auth: &AuthCapabilitySchema,
) -> anyhow::Result<()> {
    use crate::cli::oauth_defaults::{self, OAUTH_CALLBACK_PORT};

    let display_name = auth.display_name.as_deref().unwrap_or(&auth.secret_name);
    let resolved = flow
        .combined_oauth_config(auth)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Tool does not define OAuth configuration"))?;

    println!("  Starting OAuth authentication...");
    println!();

    let listener = oauth_defaults::bind_callback_listener().await?;
    let redirect_uri = format!("http://localhost:{}/callback", OAUTH_CALLBACK_PORT);
    // CSRF defense: generate a state nonce, send it on the authorization request,
    // and require the loopback callback to echo it back unchanged.
    let oauth_state = oauth_defaults::generate_oauth_state();
    let auth_request = flow
        .prepare_authorization(auth, &redirect_uri, "local", Some(&oauth_state))
        .await?;

    println!("  Opening browser for {} login...", display_name);
    println!();
    if oauth_defaults::ssh_or_headless_detected() {
        oauth_defaults::print_ssh_callback_hint();
        println!();
    }

    if let Err(e) = open::that(&auth_request.auth_url) {
        println!("  Could not open browser: {}", e);
        oauth_defaults::print_ssh_callback_hint();
        println!("  Please open this URL manually:");
        println!("  {}", auth_request.auth_url);
    }

    println!("  Waiting for authorization...");

    let code = oauth_defaults::wait_for_callback_with_state(
        listener,
        "/callback",
        "code",
        display_name,
        Some(&oauth_state),
    )
    .await?;

    println!();
    println!("  Exchanging code for token...");

    let token = flow
        .exchange_code(
            auth,
            &redirect_uri,
            &code,
            auth_request.code_verifier.as_deref(),
        )
        .await?;
    flow.store_token_exchange(auth, &token).await?;

    // Extract any additional info for display
    let workspace_name = token
        .raw
        .get("workspace_name")
        .and_then(|v| v.as_str())
        .or_else(|| token.raw.get("team_name").and_then(|v| v.as_str()));

    println!();
    println!("  ✓ {} connected!", display_name);
    if let Some(workspace) = workspace_name {
        println!("    Workspace: {}", workspace);
    }
    if resolved.required_scopes.len()
        > auth
            .oauth
            .as_ref()
            .map(|oauth| oauth.scopes.len())
            .unwrap_or_default()
    {
        println!("    Shared credential scopes were updated for other installed Google tools.");
    }
    println!();
    println!("  The tool can now access the API.");
    println!();

    Ok(())
}

/// Manual token entry flow.
async fn auth_tool_manual(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    auth: &crate::tools::wasm::AuthCapabilitySchema,
    capabilities: &CapabilitiesFile,
) -> anyhow::Result<()> {
    let display_name = auth.display_name.as_deref().unwrap_or(&auth.secret_name);

    // Show instructions
    if let Some(ref instructions) = auth.instructions {
        println!("  Setup instructions:");
        println!();
        for line in instructions.lines() {
            println!("    {}", line);
        }
        println!();
    }

    // Offer to open setup URL
    if let Some(ref url) = auth.setup_url {
        print!("  Press Enter to open setup page (or 's' to skip): ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("s") {
            if let Err(e) = open::that(url) {
                println!("  Could not open browser: {}", e);
                println!("  Please open manually: {}", url);
            } else {
                println!("  Opening browser...");
            }
        }
        println!();
    }

    // Show token hint
    if let Some(ref hint) = auth.token_hint {
        println!("  Token format: {}", hint);
        println!();
    }

    // Prompt for token
    print!("  Paste your token: ");
    std::io::stdout().flush()?;

    let token = read_hidden_input()?;
    println!();

    if token.is_empty() {
        println!("  No token provided. Aborting.");
        return Ok(());
    }

    // Validate if endpoint is provided
    if let Some(ref validation) = auth.validation_endpoint {
        print!("  Validating token...");
        std::io::stdout().flush()?;

        match validate_token(&token, validation, &auth.secret_name, capabilities).await {
            Ok(()) => {
                println!(" ✓");
            }
            Err(e) => {
                println!(" ✗");
                println!("  Validation failed: {}", e);
                println!();
                print!("  Save anyway? [y/N]: ");
                std::io::stdout().flush()?;

                let mut confirm = String::new();
                std::io::stdin().read_line(&mut confirm)?;

                if !confirm.trim().eq_ignore_ascii_case("y") {
                    println!("  Aborting.");
                    return Ok(());
                }
            }
        }
    }

    // Save the token (manual path: no refresh token or expiry)
    let flow = WasmToolOAuthFlow::new(store, user_id, Path::new("."));
    flow.store_manual_token(auth, &token).await?;
    print_success(display_name);
    Ok(())
}

/// Read input with hidden characters.
fn read_hidden_input() -> anyhow::Result<String> {
    use crossterm::{
        event::{self, Event, KeyCode, KeyModifiers},
        terminal,
    };

    let mut input = String::new();

    terminal::enable_raw_mode()?;

    loop {
        if let Event::Key(key_event) = event::read()? {
            match key_event.code {
                KeyCode::Enter => {
                    break;
                }
                KeyCode::Backspace if !input.is_empty() => {
                    input.pop();
                    print!("\x08 \x08");
                    std::io::stdout().flush()?;
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    terminal::disable_raw_mode()?;
                    return Err(anyhow::anyhow!("Interrupted"));
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    print!("*");
                    std::io::stdout().flush()?;
                }
                _ => {}
            }
        }
    }

    terminal::disable_raw_mode()?;

    Ok(input)
}

/// Validate a token against the validation endpoint.
async fn validate_token(
    token: &str,
    validation: &crate::tools::wasm::ValidationEndpointSchema,
    secret_name: &str,
    capabilities: &CapabilitiesFile,
) -> anyhow::Result<()> {
    use crate::setup::validation::{
        ValidationCredential, ValidationEndpointGrant, ensure_endpoint_is_granted,
        validate_extension_credential,
    };

    let runtime = capabilities.to_capabilities();
    let http = runtime
        .http
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("validation endpoint has no HTTP capability"))?;
    let grants = http
        .allowlist
        .iter()
        .map(|grant| ValidationEndpointGrant {
            host: grant.host.clone(),
            path_prefix: grant.path_prefix.clone(),
            methods: grant.methods.clone(),
        })
        .collect::<Vec<_>>();
    let credential = if let Some(mapping) = http.credentials.get(secret_name) {
        if !mapping.host_patterns.is_empty() {
            let host_grants = mapping
                .host_patterns
                .iter()
                .map(|host| ValidationEndpointGrant {
                    host: host.clone(),
                    path_prefix: None,
                    methods: vec![validation.method.clone()],
                })
                .collect::<Vec<_>>();
            ensure_endpoint_is_granted(&validation.url, &validation.method, &host_grants)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        match &mapping.location {
            crate::secrets::CredentialLocation::AuthorizationBearer => {
                ValidationCredential::Bearer(token)
            }
            crate::secrets::CredentialLocation::AuthorizationBasic { username } => {
                ValidationCredential::Basic {
                    username,
                    password: token,
                }
            }
            crate::secrets::CredentialLocation::Header { name, prefix } => {
                ValidationCredential::Header {
                    name,
                    prefix: prefix.as_deref(),
                    value: token,
                }
            }
            crate::secrets::CredentialLocation::QueryParam { .. }
            | crate::secrets::CredentialLocation::UrlPath { .. }
            | crate::secrets::CredentialLocation::UrlBase { .. }
            | crate::secrets::CredentialLocation::Body { .. } => {
                anyhow::bail!("validation credentials may not be placed in URLs")
            }
        }
    } else {
        ValidationCredential::Bearer(token)
    };
    validate_extension_credential(
        &validation.url,
        &validation.method,
        validation.success_status,
        &grants,
        credential,
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

/// Print success message.
fn print_success(display_name: &str) {
    println!();
    println!("  ✓ {} connected!", display_name);
    println!();
    println!("  The tool can now access the API.");
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(2621440), "2.5 MB");
    }

    #[test]
    fn test_default_tools_dir() {
        let dir = default_tools_dir();
        assert!(dir.to_string_lossy().contains(".thinclaw"));
        assert!(dir.to_string_lossy().contains("tools"));
    }

    #[test]
    fn tool_names_cannot_escape_install_directory() {
        assert!(validate_tool_name("../../outside", true).is_err());
        assert!(validate_tool_name("safe_tool-1", true).is_ok());
        assert!(validate_tool_name("shell", false).is_err());
    }

    #[tokio::test]
    async fn force_install_without_capabilities_removes_stale_sidecar() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let wasm = source.path().join("demo.wasm");
        std::fs::write(&wasm, b"\0asm\x01\0\0\0").unwrap();
        std::fs::write(
            target.path().join("demo.capabilities.json"),
            br#"{"workspace":{"allowed_prefixes":[""]}}"#,
        )
        .unwrap();

        install_tool(
            wasm,
            Some("demo".to_string()),
            None,
            Some(target.path().to_path_buf()),
            true,
            true,
            true,
        )
        .await
        .unwrap();

        assert!(target.path().join("demo.wasm").is_file());
        assert!(!target.path().join("demo.capabilities.json").exists());
    }
}
