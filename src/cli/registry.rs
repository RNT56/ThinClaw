//! Registry CLI commands for discovering and installing extensions.

use clap::Subcommand;

use crate::registry::catalog::RegistryCatalog;
use crate::registry::installer::RegistryInstaller;
use crate::registry::manifest::ManifestKind;
use crate::terminal_branding::TerminalBranding;

#[derive(Subcommand, Debug, Clone)]
pub enum RegistryCommand {
    /// List available extensions in the registry
    List {
        /// Filter by kind: "tool" or "channel"
        #[arg(short, long)]
        kind: Option<String>,

        /// Filter by tag (e.g. "default", "google", "messaging")
        #[arg(short, long)]
        tag: Option<String>,

        /// Show detailed information
        #[arg(short, long)]
        verbose: bool,
    },

    /// Search extensions by query (matches name, description, keywords)
    Search {
        /// Search query (e.g. "messaging", "slack", "email")
        query: String,
    },

    /// Show detailed information about an extension or bundle
    Info {
        /// Extension or bundle name (e.g. "slack", "google", "tools/gmail")
        name: String,
    },

    /// Install an extension or bundle from the registry
    Install {
        /// Extension or bundle name (e.g. "slack", "google", "default")
        name: String,

        /// Force overwrite if already installed
        #[arg(short, long)]
        force: bool,

        /// Build from source instead of downloading pre-built artifact
        #[arg(long)]
        build: bool,
    },

    /// Install the default bundle of recommended extensions
    InstallDefaults {
        /// Force overwrite if already installed
        #[arg(short, long)]
        force: bool,

        /// Build from source instead of downloading pre-built artifact
        #[arg(long)]
        build: bool,
    },

    /// Remove an installed extension
    Remove {
        /// Extension name (e.g. "slack", "telegram")
        name: String,
    },
}

/// Run a registry command.
pub async fn run_registry_command(cmd: RegistryCommand) -> anyhow::Result<()> {
    // For install commands that need to build from source, a disk registry is required.
    // For list/info, embedded manifests suffice.
    let registry_dir = RegistryCatalog::find_dir();
    let catalog = if let Some(ref dir) = registry_dir {
        RegistryCatalog::load(dir)?
    } else {
        RegistryCatalog::load_or_embedded()?
    };

    // Resolve repo root for installer (empty path when running from binary)
    let repo_root = registry_dir
        .as_ref()
        .and_then(|d| d.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();

    match cmd {
        RegistryCommand::List { kind, tag, verbose } => {
            cmd_list(&catalog, kind.as_deref(), tag.as_deref(), verbose)
        }
        RegistryCommand::Search { query } => cmd_search(&catalog, &query),
        RegistryCommand::Info { name } => cmd_info(&catalog, &name),
        RegistryCommand::Install { name, force, build } => {
            cmd_install(&catalog, &repo_root, &name, force, build).await
        }
        RegistryCommand::InstallDefaults { force, build } => {
            cmd_install(&catalog, &repo_root, "default", force, build).await
        }
        RegistryCommand::Remove { name } => cmd_remove(&name).await,
    }
}

fn cmd_list(
    catalog: &RegistryCatalog,
    kind: Option<&str>,
    tag: Option<&str>,
    verbose: bool,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let kind_filter = match kind {
        Some("tool" | "tools") => Some(ManifestKind::Tool),
        Some("channel" | "channels") => Some(ManifestKind::Channel),
        Some(other) => anyhow::bail!("Unknown kind '{}'. Use 'tool' or 'channel'.", other),
        None => None,
    };

    let manifests = catalog.list(kind_filter, tag);

    if manifests.is_empty() {
        branding.print_banner("Registry", Some("Browse installable extensions"));
        println!(
            "{}",
            branding.warn("No extensions found matching the criteria.")
        );
        return Ok(());
    }

    // Print header
    branding.print_banner("Registry", Some("Browse installable extensions"));
    if verbose {
        println!(
            "{}",
            branding.body_bold(format!(
                "{:<20} {:<8} {:<8} {:<10} DESCRIPTION",
                "NAME", "KIND", "VERSION", "AUTH"
            ))
        );
        println!("{}", branding.separator(80));
    } else {
        println!(
            "{}",
            branding.body_bold(format!("{:<20} {:<8} DESCRIPTION", "NAME", "KIND"))
        );
        println!("{}", branding.separator(60));
    }

    for m in &manifests {
        if verbose {
            let auth = m
                .auth_summary
                .as_ref()
                .and_then(|a| a.method.as_deref())
                .unwrap_or("none");
            println!(
                "{:<20} {:<8} {:<8} {:<10} {}",
                m.name, m.kind, m.version, auth, m.description
            );
        } else {
            println!("{:<20} {:<8} {}", m.name, m.kind, m.description);
        }
    }

    println!();
    println!(
        "{}",
        branding.muted(format!("{} extension(s) found.", manifests.len()))
    );

    // Show bundles hint
    let bundle_names = catalog.bundle_names();
    if !bundle_names.is_empty() {
        println!();
        println!("{}", branding.key_value("Bundles", bundle_names.join(", ")));
        println!(
            "{}",
            branding.muted("Use `thinclaw registry info <bundle>` for details.")
        );
    }

    Ok(())
}

fn cmd_info(catalog: &RegistryCatalog, name: &str) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    // Check if it's a bundle
    if let Some(bundle) = catalog.get_bundle(name) {
        branding.print_banner("Registry", Some("Inspect a bundle"));
        println!("{}", branding.key_value("Bundle", &bundle.display_name));
        if let Some(desc) = &bundle.description {
            println!("{}", branding.body(desc));
        }
        println!("\nExtensions:");
        for ext_key in &bundle.extensions {
            if let Some(m) = catalog.get(ext_key) {
                println!("  {} - {} ({})", ext_key, m.description, m.kind);
            } else {
                println!("  {} (not found in registry)", ext_key);
            }
        }
        if let Some(shared) = &bundle.shared_auth {
            println!("\nShared auth: {}", shared);
        }
        return Ok(());
    }

    // Single extension (use get_strict to surface ambiguous bare names)
    let manifest = catalog
        .get_strict(name)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    branding.print_banner("Registry", Some("Inspect an extension"));
    println!(
        "{}",
        branding.key_value(
            "Extension",
            format!("{} ({})", manifest.display_name, manifest.kind)
        )
    );
    println!("{}", branding.key_value("Version", &manifest.version));
    println!("{}", branding.body(&manifest.description));

    if !manifest.keywords.is_empty() {
        println!("  Keywords: {}", manifest.keywords.join(", "));
    }

    println!("\nSource:");
    println!("  Directory: {}", manifest.source.dir);
    println!("  Crate: {}", manifest.source.crate_name);
    println!("  Capabilities: {}", manifest.source.capabilities);

    if let Some(artifact) = manifest.artifacts.get("wasm32-wasip2") {
        println!("\nArtifact (wasm32-wasip2):");
        match &artifact.url {
            Some(url) => println!("  URL: {}", url),
            None => println!("  URL: (not yet published)"),
        }
        match &artifact.sha256 {
            Some(sha) => println!("  SHA256: {}", sha),
            None => println!("  SHA256: (not yet computed)"),
        }
    }

    if let Some(auth) = &manifest.auth_summary {
        println!("\nAuthentication:");
        if let Some(method) = &auth.method {
            println!("  Method: {}", method);
        }
        if let Some(provider) = &auth.provider {
            println!("  Provider: {}", provider);
        }
        if !auth.secrets.is_empty() {
            println!("  Secrets: {}", auth.secrets.join(", "));
        }
        if let Some(shared) = &auth.shared_auth {
            println!("  Shared with: {}", shared);
        }
        if let Some(url) = &auth.setup_url {
            println!("  Setup: {}", url);
        }
    }

    if !manifest.tags.is_empty() {
        println!("\nTags: {}", manifest.tags.join(", "));
    }

    Ok(())
}

async fn cmd_install(
    catalog: &RegistryCatalog,
    repo_root: &std::path::Path,
    name: &str,
    force: bool,
    prefer_build: bool,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let installer = RegistryInstaller::with_defaults(repo_root.to_path_buf());

    let (manifests, bundle) = catalog.resolve(name)?;

    if manifests.is_empty() {
        anyhow::bail!("No extensions found for '{}'.", name);
    }

    if let Some(bundle_def) = bundle {
        // Bundle install
        branding.print_banner("Registry", Some("Install a bundle"));
        println!(
            "{}",
            branding.accent(format!(
                "Installing bundle '{}' ({} extensions)...",
                bundle_def.display_name,
                manifests.len()
            ))
        );
        println!();

        let (outcomes, hints) = installer
            .install_bundle(&manifests, bundle_def, force, prefer_build)
            .await;

        println!();
        println!("{}", branding.body_bold("Results"));
        for outcome in &outcomes {
            let caps_status = if outcome.has_capabilities { "+" } else { "-" };
            println!(
                "  [{}] {} ({}) -> {}",
                caps_status,
                outcome.name,
                outcome.kind,
                outcome.wasm_path.display()
            );
            for w in &outcome.warnings {
                println!("      Warning: {}", w);
            }
        }

        if !hints.is_empty() {
            println!();
            println!("{}", branding.body_bold("Auth setup"));
            for hint in &hints {
                println!("{}", hint);
            }
        }

        println!();
        println!(
            "{}",
            branding.good(format!(
                "Installed {}/{} extensions.",
                outcomes.len(),
                manifests.len()
            ))
        );
    } else {
        // Single extension
        let manifest = manifests[0];
        let outcome = installer.install(manifest, force, prefer_build).await?;

        branding.print_banner("Registry", Some("Install an extension"));
        println!("{}", branding.good("Installed successfully:"));
        println!("  Name: {}", outcome.name);
        println!("  Kind: {}", outcome.kind);
        println!("  WASM: {}", outcome.wasm_path.display());
        println!("  Capabilities: {}", outcome.has_capabilities);

        if let Some(auth) = &manifest.auth_summary
            && auth.method.as_deref() != Some("none")
        {
            println!(
                "\nNext step: authenticate with `thinclaw tool auth {}`",
                manifest.name
            );
            if let Some(url) = &auth.setup_url {
                println!("  Setup credentials at: {}", url);
            }
        }
    }

    Ok(())
}

fn cmd_search(catalog: &RegistryCatalog, query: &str) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let query_lower = query.to_lowercase();
    let manifests = catalog.list(None, None);

    let matches: Vec<_> = manifests
        .iter()
        .filter(|m| {
            m.name.to_lowercase().contains(&query_lower)
                || m.description.to_lowercase().contains(&query_lower)
                || m.keywords
                    .iter()
                    .any(|k| k.to_lowercase().contains(&query_lower))
        })
        .collect();

    if matches.is_empty() {
        branding.print_banner("Registry", Some("Search the catalog"));
        println!(
            "{}",
            branding.warn(format!("No extensions matching '{}'.", query))
        );
        return Ok(());
    }

    branding.print_banner("Registry", Some("Search the catalog"));
    println!(
        "{}",
        branding.body_bold(format!("{:<20} {:<8} DESCRIPTION", "NAME", "KIND"))
    );
    println!("{}", branding.separator(60));

    for m in &matches {
        println!("{:<20} {:<8} {}", m.name, m.kind, m.description);
    }

    println!();
    println!(
        "{}",
        branding.muted(format!("{} result(s) for '{}'.", matches.len(), query))
    );
    Ok(())
}

async fn cmd_remove(name: &str) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    // Check channels dir
    let channels_dir = home.join(".thinclaw/channels");
    let wasm_path = channels_dir.join(format!("{}.wasm", name));
    let caps_path = channels_dir.join(format!("{}.capabilities.json", name));

    if wasm_path.exists() {
        tokio::fs::remove_file(&wasm_path).await?;
        let _ = tokio::fs::remove_file(&caps_path).await;
        branding.print_banner("Registry", Some("Remove an installed extension"));
        println!(
            "  {}",
            branding.good(format!("Removed channel '{}'.", name))
        );
        return Ok(());
    }

    // Check tools dir
    let tools_dir = home.join(".thinclaw/tools");
    let tool_wasm = tools_dir.join(format!("{}.wasm", name));
    let tool_caps = tools_dir.join(format!("{}.capabilities.json", name));

    if tool_wasm.exists() {
        tokio::fs::remove_file(&tool_wasm).await?;
        let _ = tokio::fs::remove_file(&tool_caps).await;
        branding.print_banner("Registry", Some("Remove an installed extension"));
        println!("  {}", branding.good(format!("Removed tool '{}'.", name)));
        return Ok(());
    }

    anyhow::bail!(
        "Extension '{}' not found in ~/.thinclaw/channels/ or ~/.thinclaw/tools/.",
        name
    );
}
