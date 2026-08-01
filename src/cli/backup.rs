//! `thinclaw export` / `thinclaw import` — whole-agent encrypted backup.
//!
//! An export bundles the ThinClaw home directory (config, `SOUL.md`, skills,
//! channels, …) as a file tree plus a database payload, sealed with a
//! passphrase (scrypt + XChaCha20-Poly1305) via [`thinclaw_portability`]. The
//! bundle is portable: it decrypts with the passphrase alone, on any machine.
//!
//! Volatile and secret paths (`logs/`, `.env`, pid files, capture dirs, the live
//! database file) are excluded from the file tree. Secrets are **not** exported;
//! they live in the OS keychain / secrets store and must be re-provisioned.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Subcommand;
use secrecy::{ExposeSecret, SecretString};

use crate::config::{Config, DatabaseBackend};
use crate::platform::state_paths;
use crate::terminal_branding::TerminalBranding;
use thinclaw_portability::{BundleWriter, MAX_SEALED_BUNDLE_BYTES, OpenBundle, SectionKind};

const PASSPHRASE_ENV: &str = "THINCLAW_BACKUP_PASSPHRASE";
const WORKSPACE_SECTION: &str = "workspace";
const DATABASE_SECTION: &str = "database";
const MAX_DATABASE_DUMP_BYTES: u64 = 512 * 1024 * 1024;
const MAX_BACKUP_PASSPHRASE_BYTES: usize = 4 * 1024;
const MIN_EXPORT_PASSPHRASE_BYTES: usize = 12;

#[derive(Subcommand, Debug, Clone)]
pub enum BackupCommand {
    /// Export whole-agent state to an encrypted bundle.
    Export {
        /// Output bundle path (default: ./thinclaw-backup-<timestamp>.tclaw).
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Read the passphrase from a private regular file.
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
        /// Replace an existing output bundle.
        #[arg(long)]
        force: bool,
        /// Skip the database section (config + workspace files only).
        #[arg(long, conflicts_with = "require_database")]
        no_database: bool,
        /// Fail instead of producing a partial bundle when database export is unavailable.
        #[arg(long)]
        require_database: bool,
    },
    /// Restore an encrypted bundle. Overwrites config + workspace files in the
    /// ThinClaw home; requires `--yes`.
    Import {
        /// Bundle path to restore from.
        input: PathBuf,
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
        /// Confirm overwriting config + workspace files.
        #[arg(long)]
        yes: bool,
        /// Inspect the restore plan without mutation (the default without --yes).
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
        /// Also restore the database in place. For the local (libSQL) backend
        /// this overwrites the database file — ThinClaw must be stopped. For
        /// Postgres the restore command is printed instead of run.
        #[arg(long)]
        restore_database: bool,
    },
    /// Show a bundle's manifest without restoring anything.
    Inspect {
        /// Bundle path to inspect.
        input: PathBuf,
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },
}

pub async fn run_backup_command(cmd: BackupCommand) -> anyhow::Result<()> {
    match cmd {
        BackupCommand::Export {
            out,
            passphrase_file,
            force,
            no_database,
            require_database,
        } => export(out, passphrase_file, force, no_database, require_database).await,
        BackupCommand::Import {
            input,
            passphrase_file,
            yes,
            dry_run: _,
            restore_database,
        } => import(input, passphrase_file, yes, restore_database).await,
        BackupCommand::Inspect {
            input,
            passphrase_file,
        } => inspect(input, passphrase_file).await,
    }
}

async fn export(
    output: Option<PathBuf>,
    passphrase_file: Option<PathBuf>,
    force: bool,
    no_database: bool,
    require_database: bool,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    branding.print_banner("ThinClaw Export", Some("Encrypted whole-agent backup"));
    let passphrase = resolve_passphrase(passphrase_file.as_deref(), true)?;

    let home = state_paths().home;
    if !home.exists() {
        anyhow::bail!("no ThinClaw home directory at {}", home.display());
    }

    let created_at = chrono::Utc::now();
    let mut writer = BundleWriter::new(producer_version()).created_at(created_at.to_rfc3339());

    let file_count = writer.add_dir(WORKSPACE_SECTION, WORKSPACE_SECTION, &home, &is_excluded)?;
    println!("{}", branding.key_value("workspace files", file_count));

    if no_database {
        println!(
            "{}",
            branding.muted("database section skipped (--no-database)")
        );
    } else {
        match export_database().await {
            Ok(Some((bytes, note))) => {
                let len = bytes.len();
                writer.add_blob(
                    DATABASE_SECTION,
                    SectionKind::Database,
                    "database/dump.bin",
                    &bytes,
                    Some(note.clone()),
                )?;
                println!(
                    "{}",
                    branding.key_value("database", format!("{len} bytes ({note})"))
                );
            }
            Ok(None) => {
                if require_database {
                    anyhow::bail!(
                        "database export is unavailable and --require-database was requested"
                    );
                }
                println!(
                    "{}",
                    branding.warn(
                        "database export unavailable; bundle contains config + workspace only"
                    )
                );
            }
            Err(error) => {
                if require_database {
                    anyhow::bail!(
                        "database export failed and --require-database was requested: {error}"
                    );
                }
                println!(
                    "{}",
                    branding.warn(format!(
                        "database export failed: {error}; bundle contains config + workspace only"
                    ))
                );
            }
        }
    }

    let sealed = writer.finish(passphrase.expose_secret())?;
    let out_path = output.unwrap_or_else(|| default_output_name(&created_at));
    if out_path.exists() && !force {
        anyhow::bail!(
            "refusing to overwrite existing backup {}; pass --force",
            out_path.display()
        );
    }
    thinclaw_platform::write_private_file_atomic(&out_path, &sealed, true)?;

    println!(
        "{}",
        branding.good(format!(
            "wrote encrypted backup to {} ({} bytes)",
            out_path.display(),
            sealed.len()
        ))
    );
    println!(
        "{}",
        branding.muted("secrets/.env are NOT included; re-provision credentials after restore")
    );
    Ok(())
}

async fn import(
    input: PathBuf,
    passphrase_file: Option<PathBuf>,
    yes: bool,
    restore_database: bool,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    branding.print_banner("ThinClaw Import", Some("Restore from encrypted backup"));
    let passphrase = resolve_passphrase(passphrase_file.as_deref(), false)?;

    let sealed =
        thinclaw_platform::read_regular_file_bounded_async(input.clone(), MAX_SEALED_BUNDLE_BYTES)
            .await
            .map_err(|error| anyhow::anyhow!("cannot read bundle {}: {error}", input.display()))?;
    let bundle = OpenBundle::open(&sealed, passphrase.expose_secret())?;
    print_manifest(&branding, bundle.manifest());

    if !yes {
        println!(
            "{}",
            branding.warn("dry run: re-run with --yes to overwrite config + workspace files")
        );
        return Ok(());
    }

    let home = state_paths().home;
    std::fs::create_dir_all(&home)?;
    let restored = bundle.extract_files(WORKSPACE_SECTION, &home)?;
    println!(
        "{}",
        branding.good(format!(
            "restored {restored} workspace files to {}",
            home.display()
        ))
    );

    if bundle.manifest().section(DATABASE_SECTION).is_some() {
        restore_database_section(&branding, &input, &bundle, restore_database).await?;
    }

    println!(
        "{}",
        branding.muted("re-provision secrets/credentials; they are not part of the backup")
    );
    Ok(())
}

async fn inspect(input: PathBuf, passphrase_file: Option<PathBuf>) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    branding.print_banner("ThinClaw Bundle", Some("Manifest (no changes made)"));
    let passphrase = resolve_passphrase(passphrase_file.as_deref(), false)?;
    let sealed =
        thinclaw_platform::read_regular_file_bounded_async(input.clone(), MAX_SEALED_BUNDLE_BYTES)
            .await
            .map_err(|error| anyhow::anyhow!("cannot read bundle {}: {error}", input.display()))?;
    let bundle = OpenBundle::open(&sealed, passphrase.expose_secret())?;
    print_manifest(&branding, bundle.manifest());
    Ok(())
}

/// Export the database as backend-appropriate bytes: a WAL-checkpointed libSQL
/// snapshot, or a `pg_dump` custom-format archive for Postgres. Returns `None`
/// when neither is possible (the caller then writes a config+workspace bundle).
async fn export_database() -> anyhow::Result<Option<(Vec<u8>, String)>> {
    let config = Config::from_env()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    if config.database.backend == DatabaseBackend::Postgres {
        return pg_dump_export(&config).await;
    }

    // Snapshot-capable backend (libSQL): copy the WAL-checkpointed file.
    let db = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let temp_dir = tempfile::Builder::new()
        .prefix("thinclaw-export-")
        .tempdir()?;
    let tmp = temp_dir.path().join("snapshot.db");
    match db.snapshot(&tmp).await {
        Ok(_) => {
            let bytes =
                thinclaw_platform::read_regular_file_bounded(&tmp, MAX_DATABASE_DUMP_BYTES)?;
            Ok(Some((
                bytes,
                "libsql snapshot (wal-checkpointed)".to_string(),
            )))
        }
        Err(error) => Err(anyhow::anyhow!("snapshot failed: {error}")),
    }
}

/// Run `pg_dump --format=custom` against the configured URL. Returns `None` if
/// `pg_dump` is not installed so export can still produce a partial bundle.
async fn pg_dump_export(config: &Config) -> anyhow::Result<Option<(Vec<u8>, String)>> {
    let temp_dir = tempfile::Builder::new()
        .prefix("thinclaw-pgdump-")
        .tempdir()?;
    let tmp = temp_dir.path().join("dump.bin");
    let mut url = url::Url::parse(config.database.url.expose_secret())
        .map_err(|error| anyhow::anyhow!("invalid Postgres database URL: {error}"))?;
    let password = url
        .password()
        .map(urlencoding::decode)
        .transpose()
        .map_err(|error| anyhow::anyhow!("invalid percent-encoding in database password: {error}"))?
        .map(|password| SecretString::from(password.into_owned()));
    url.set_password(None)
        .map_err(|()| anyhow::anyhow!("could not sanitize Postgres database URL"))?;
    let pgpass_path = if let Some(password) = password.as_ref() {
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("Postgres database URL has no host"))?;
        let port = url.port().unwrap_or(5432);
        let database = url.path().trim_start_matches('/');
        let user = url.username();
        if database.is_empty() || user.is_empty() {
            anyhow::bail!("Postgres database URL must include a database and user");
        }
        let escape = |value: &str| {
            if value.contains(['\n', '\r']) {
                anyhow::bail!("Postgres credential fields cannot contain newlines");
            }
            Ok::<String, anyhow::Error>(value.replace('\\', "\\\\").replace(':', "\\:"))
        };
        let line = format!(
            "{}:{}:{}:{}:{}\n",
            escape(host)?,
            port,
            escape(database)?,
            escape(user)?,
            escape(password.expose_secret())?
        );
        let path = temp_dir.path().join("pgpass");
        thinclaw_platform::write_private_file_atomic(&path, line.as_bytes(), false)?;
        Some(path)
    } else {
        None
    };
    let mut command = tokio::process::Command::new("pg_dump");
    command
        .arg("--format=custom")
        .arg("--no-owner")
        .arg("--no-privileges")
        .arg("--file")
        .arg(&tmp)
        .arg(url.as_str())
        .env("PGCONNECT_TIMEOUT", "15");
    if let Some(path) = pgpass_path.as_ref() {
        command.env("PGPASSFILE", path);
    }
    let result = thinclaw_platform::bounded_command_output(
        &mut command,
        Duration::from_secs(30 * 60),
        64 * 1024,
        64 * 1024,
    )
    .await;

    match result {
        Ok(output) if output.status.success() => {
            let bytes =
                thinclaw_platform::read_regular_file_bounded(&tmp, MAX_DATABASE_DUMP_BYTES)?;
            Ok(Some((bytes, "pg_dump --format=custom".to_string())))
        }
        Ok(output) => Err(anyhow::anyhow!(
            "pg_dump exited with status {}",
            output.status
        )),
        Err(thinclaw_platform::BoundedProcessError::Spawn(error)) => {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(None) // pg_dump not installed
            } else {
                Err(anyhow::anyhow!("failed to run pg_dump: {error}"))
            }
        }
        Err(error) => Err(anyhow::anyhow!("failed to run pg_dump: {error}")),
    }
}

/// Restore the database section: always write the extracted dump next to the
/// bundle for auditability; then either restore in place (libSQL, on
/// `--restore-database`) or print the exact Postgres restore command.
async fn restore_database_section(
    branding: &TerminalBranding,
    input: &Path,
    bundle: &OpenBundle,
    restore_database: bool,
) -> anyhow::Result<()> {
    let db_bytes = bundle.section_bytes(DATABASE_SECTION)?;
    let dump_path = input.with_extension("database-dump");
    thinclaw_platform::write_private_file_atomic(&dump_path, db_bytes, true)?;
    println!(
        "{}",
        branding.key_value("database dump written", dump_path.display())
    );

    let config = Config::from_env()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    if config.database.backend == DatabaseBackend::Postgres {
        println!(
            "{}",
            branding.warn(
                "Postgres database is NOT restored automatically. With ThinClaw stopped, run:"
            )
        );
        println!(
            "    pg_restore --clean --if-exists --no-owner --dbname \"$DATABASE_URL\" {}",
            dump_path.display()
        );
        return Ok(());
    }

    // libSQL backend.
    let target = config
        .database
        .libsql_path
        .clone()
        .unwrap_or_else(crate::config::default_libsql_path);
    if restore_database {
        println!(
            "{}",
            branding.warn(format!(
                "overwriting local database at {} — ThinClaw must be stopped",
                target.display()
            ))
        );
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        validate_libsql_sidecar(&target, "-wal")?;
        validate_libsql_sidecar(&target, "-shm")?;
        thinclaw_platform::write_private_file_atomic(&target, db_bytes, true)?;
        remove_libsql_sidecar_if_present(&target, "-wal")?;
        remove_libsql_sidecar_if_present(&target, "-shm")?;
        println!("{}", branding.good("local database restored"));
    } else {
        println!(
            "{}",
            branding.muted(format!(
                "pass --restore-database to overwrite the local database at {} (ThinClaw must be stopped), or copy the dump there manually",
                target.display()
            ))
        );
    }
    Ok(())
}

fn print_manifest(branding: &TerminalBranding, manifest: &thinclaw_portability::BundleManifest) {
    println!(
        "{}",
        branding.key_value("produced by", &manifest.producer_version)
    );
    if let Some(created) = &manifest.created_at {
        println!("{}", branding.key_value("created at", created));
    }
    for section in &manifest.sections {
        let detail = match &section.note {
            Some(note) => format!("{:?} — {note}", section.kind),
            None => format!("{:?}", section.kind),
        };
        println!("{}", branding.key_value(&section.name, detail));
    }
}

/// Resolve passphrase precedence: private file, environment, masked TTY.
fn resolve_passphrase(
    private_file: Option<&Path>,
    require_strong: bool,
) -> anyhow::Result<SecretString> {
    if let Some(path) = private_file {
        if !path.is_absolute() {
            anyhow::bail!("--passphrase-file must be an absolute path");
        }
        validate_private_passphrase_permissions(path)?;
        let mut bytes = thinclaw_platform::read_regular_file_bounded_single_link(
            path,
            MAX_BACKUP_PASSPHRASE_BYTES as u64,
        )?;
        if bytes.ends_with(b"\n") {
            bytes.pop();
            if bytes.ends_with(b"\r") {
                bytes.pop();
            }
        }
        let passphrase = String::from_utf8(bytes)
            .map_err(|_| anyhow::anyhow!("backup passphrase file is not UTF-8"))?;
        return validate_backup_passphrase(passphrase, require_strong);
    }
    match std::env::var(PASSPHRASE_ENV) {
        Ok(pass) if !pass.is_empty() => validate_backup_passphrase(pass, require_strong),
        _ if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => {
            let passphrase = crate::setup::prompts::secret_input("Backup passphrase")?;
            validate_backup_passphrase(passphrase.expose_secret().to_string(), require_strong)
        }
        _ => anyhow::bail!(
            "no passphrase available: use --passphrase-file, set {PASSPHRASE_ENV}, or run in a TTY"
        ),
    }
}

fn validate_private_passphrase_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            anyhow::bail!(
                "backup passphrase file must be owned by the effective user with no group/other permissions"
            );
        }
    }
    Ok(())
}

fn validate_backup_passphrase(
    passphrase: String,
    require_strong: bool,
) -> anyhow::Result<SecretString> {
    if passphrase.len() > MAX_BACKUP_PASSPHRASE_BYTES {
        anyhow::bail!("backup passphrase exceeds {MAX_BACKUP_PASSPHRASE_BYTES} bytes");
    }
    if require_strong && passphrase.len() < MIN_EXPORT_PASSPHRASE_BYTES {
        anyhow::bail!(
            "backup passphrase must be at least {MIN_EXPORT_PASSPHRASE_BYTES} bytes for a new export"
        );
    }
    Ok(SecretString::from(passphrase))
}

fn remove_libsql_sidecar_if_present(target: &Path, suffix: &str) -> anyhow::Result<()> {
    let sidecar = libsql_sidecar_path(target, suffix);
    match std::fs::symlink_metadata(&sidecar) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            anyhow::bail!(
                "refusing to remove non-regular libSQL sidecar {}",
                sidecar.display()
            );
        }
        Ok(_) => std::fs::remove_file(&sidecar)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn validate_libsql_sidecar(target: &Path, suffix: &str) -> anyhow::Result<()> {
    let sidecar = libsql_sidecar_path(target, suffix);
    match std::fs::symlink_metadata(&sidecar) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            anyhow::bail!(
                "libSQL sidecar is not a regular file: {}",
                sidecar.display()
            );
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn libsql_sidecar_path(target: &Path, suffix: &str) -> PathBuf {
    let mut sidecar_name = target.as_os_str().to_os_string();
    sidecar_name.push(suffix);
    PathBuf::from(sidecar_name)
}

/// Volatile or secret paths (relative to the ThinClaw home) excluded from the
/// exported file tree.
fn is_excluded(rel: &Path) -> bool {
    let first = rel.components().next().and_then(|c| c.as_os_str().to_str());
    if matches!(
        first,
        Some("logs" | "bin" | "screenshots" | "camera" | "audio")
    ) {
        return true;
    }
    if let Some(name) = rel.file_name().and_then(|n| n.to_str()) {
        if name == ".env" || name == "gateway.pid" {
            return true;
        }
        // Live libSQL database files (exported separately, WAL-checkpointed).
        if name.ends_with(".db") || name.ends_with(".db-wal") || name.ends_with(".db-shm") {
            return true;
        }
    }
    false
}

fn producer_version() -> String {
    format!("thinclaw {}", env!("CARGO_PKG_VERSION"))
}

fn default_output_name(created_at: &chrono::DateTime<chrono::Utc>) -> PathBuf {
    PathBuf::from(format!(
        "thinclaw-backup-{}.tclaw",
        created_at.format("%Y%m%d-%H%M%S")
    ))
}
