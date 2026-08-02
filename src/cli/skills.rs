//! Running-runtime administration for the complete skill lifecycle surface.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use super::{CliContext, CliError, GatewayClient};

#[derive(Subcommand, Debug, Clone)]
pub enum SkillCommand {
    /// List installed skills with source and trust metadata
    List,
    /// Search installed skills and the configured catalog
    Search { query: String },
    /// Inspect files, provenance, and scanner findings
    Inspect { name: String },
    /// Read the complete instruction content for one skill
    Read { name: String },
    /// Scan a candidate without installing it
    Check(SkillCheckArgs),
    /// Install a catalog skill or an explicitly supplied source
    Install(SkillInstallArgs),
    /// Update an installed skill from its catalog identity
    Update {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Audit one or every installed skill without mutation
    Audit { name: Option<String> },
    /// Write a deterministic installed-skill inventory artifact
    Snapshot {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Preview or execute publication through the running runtime
    Publish(SkillPublishArgs),
    /// Manage configured skill taps
    #[command(subcommand)]
    Taps(SkillTapsCommand),
    /// Remove an installed skill
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Reload one skill or rediscover every skill
    Reload {
        name: Option<String>,
        #[arg(long, conflicts_with = "name")]
        all: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Change an installed skill's trust ceiling
    Trust {
        name: String,
        #[arg(long, value_enum)]
        to: SkillTrustTarget,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args, Debug, Clone)]
pub struct SkillCheckArgs {
    #[arg(long, value_name = "FILE")]
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub stdin: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SkillInstallArgs {
    pub name: String,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long, value_name = "FILE", conflicts_with = "url")]
    pub from_file: Option<PathBuf>,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SkillPublishArgs {
    pub name: String,
    #[arg(long)]
    pub target_repo: String,
    /// Execute the remote write; the default is a dry-run preview
    #[arg(long)]
    pub execute: bool,
    #[arg(long, requires = "execute")]
    pub yes: bool,
    #[arg(long, requires = "execute")]
    pub approve_risky: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SkillTapsCommand {
    List,
    Add {
        repo: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long, value_enum, default_value_t = SkillTapTrust::Community)]
        trust: SkillTapTrust,
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        yes: bool,
    },
    Remove {
        repo: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    Refresh {
        repo: Option<String>,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTrustTarget {
    Installed,
    Trusted,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTapTrust {
    Builtin,
    Trusted,
    Community,
}

pub async fn run_skills_command(cmd: SkillCommand, context: &CliContext) -> Result<(), CliError> {
    if let SkillCommand::Check(args) = cmd {
        return check_candidate(args, context).await;
    }

    let config = context.config().await?;
    let client = GatewayClient::resolve_from_config(None, None, config)
        .map_err(|error| CliError::operational(error.to_string()))?;

    match cmd {
        SkillCommand::List => {
            let value: serde_json::Value = client
                .get_json("/api/skills", &EmptyQuery {})
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.list", value)
        }
        SkillCommand::Search { query } => {
            if query.trim().is_empty() || query.len() > 512 {
                return Err(CliError::usage(
                    "skill search query must contain 1 to 512 characters",
                ));
            }
            let value: serde_json::Value = client
                .post_json("/api/skills/search", &serde_json::json!({"query": query}))
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.search", value)
        }
        SkillCommand::Inspect { name } => {
            let path = skill_path(&name, "inspect")?;
            let value: serde_json::Value = client
                .post_json(
                    &path,
                    &serde_json::json!({
                        "include_content": false,
                        "include_files": true,
                        "audit": true
                    }),
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.inspect", value)
        }
        SkillCommand::Read { name } => {
            let path = skill_path(&name, "inspect")?;
            let value: serde_json::Value = client
                .post_json(
                    &path,
                    &serde_json::json!({
                        "include_content": true,
                        "include_files": false,
                        "audit": false
                    }),
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.read", value)
        }
        SkillCommand::Install(args) => {
            validate_skill_name(&args.name)?;
            confirm("install skill", &args.name, args.yes)?;
            let content = args
                .from_file
                .as_deref()
                .map(read_bounded_file)
                .transpose()?;
            let value: serde_json::Value = client
                .post_json_confirmed(
                    "/api/skills/install",
                    &serde_json::json!({
                        "name": args.name,
                        "url": args.url,
                        "content": content,
                        "force": false
                    }),
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.install", value)
        }
        SkillCommand::Update { name, yes } => {
            validate_skill_name(&name)?;
            confirm("update skill", &name, yes)?;
            let value: serde_json::Value = client
                .post_json_confirmed(
                    "/api/skills/install",
                    &serde_json::json!({
                        "name": name,
                        "url": null,
                        "content": null,
                        "force": true
                    }),
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.update", value)
        }
        SkillCommand::Audit { name } => {
            let names = match name {
                Some(name) => vec![name],
                None => installed_skill_names(&client).await?,
            };
            let mut reports = Vec::with_capacity(names.len());
            for name in names {
                let value: serde_json::Value = client
                    .post_json(
                        &skill_path(&name, "inspect")?,
                        &serde_json::json!({
                            "include_content": false,
                            "include_files": true,
                            "audit": true
                        }),
                    )
                    .await
                    .map_err(gateway_error)?;
                reports.push(value);
            }
            write_value(
                context,
                "extensions.skills.audit",
                serde_json::json!({"reports": reports}),
            )
        }
        SkillCommand::Snapshot { out, force } => {
            let mut value: serde_json::Value = client
                .get_json("/api/skills", &EmptyQuery {})
                .await
                .map_err(gateway_error)?;
            if let Some(skills) = value
                .get_mut("skills")
                .and_then(serde_json::Value::as_array_mut)
            {
                skills.sort_by(|left, right| {
                    left.get("name")
                        .and_then(serde_json::Value::as_str)
                        .cmp(&right.get("name").and_then(serde_json::Value::as_str))
                });
            }
            let inventory = serde_json::to_vec(&value)
                .map_err(|error| CliError::operational(error.to_string()))?;
            let artifact = serde_json::json!({
                "schema_version": 1,
                "sha256": sha256_hex(&inventory),
                "inventory": value
            });
            write_artifact(&out, &artifact, force).await?;
            write_value(
                context,
                "extensions.skills.snapshot",
                serde_json::json!({"path": out, "sha256": artifact["sha256"]}),
            )
        }
        SkillCommand::Publish(args) => {
            validate_skill_name(&args.name)?;
            if args.execute {
                confirm("publish skill remotely", &args.name, args.yes)?;
            }
            let value: serde_json::Value = if args.execute {
                client
                    .post_json_confirmed(
                        &skill_path(&args.name, "publish")?,
                        &serde_json::json!({
                            "target_repo": args.target_repo,
                            "dry_run": false,
                            "remote_write": true,
                            "confirm_remote_write": true,
                            "approve_risky": args.approve_risky
                        }),
                    )
                    .await
            } else {
                client
                    .post_json(
                        &skill_path(&args.name, "publish")?,
                        &serde_json::json!({
                            "target_repo": args.target_repo,
                            "dry_run": true,
                            "remote_write": false,
                            "confirm_remote_write": false,
                            "approve_risky": false
                        }),
                    )
                    .await
            }
            .map_err(gateway_error)?;
            write_value(context, "extensions.skills.publish", value)
        }
        SkillCommand::Taps(command) => run_taps(command, context, &client).await,
        SkillCommand::Remove { name, yes } => {
            confirm("remove skill", &name, yes)?;
            let value: serde_json::Value = client
                .delete_json_confirmed(&skill_base_path(&name)?)
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.remove", value)
        }
        SkillCommand::Reload { name, all, yes } => {
            if name.is_none() && !all {
                return Err(CliError::usage("skill reload requires NAME or --all"));
            }
            confirm(
                "reload skills",
                name.as_deref().unwrap_or("all installed skills"),
                yes,
            )?;
            let path = match name {
                Some(name) => skill_path(&name, "reload")?,
                None => "/api/skills/reload-all".to_string(),
            };
            let value: serde_json::Value = client
                .post_json_confirmed(&path, &serde_json::json!({}))
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.reload", value)
        }
        SkillCommand::Trust { name, to, yes } => {
            confirm("change skill trust", &name, yes)?;
            let value: serde_json::Value = client
                .put_json_confirmed(
                    &skill_path(&name, "trust")?,
                    &serde_json::json!({"trust": to}),
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "extensions.skills.trust", value)
        }
        SkillCommand::Check(_) => unreachable!(),
    }
}

async fn run_taps(
    command: SkillTapsCommand,
    context: &CliContext,
    client: &GatewayClient,
) -> Result<(), CliError> {
    let (command_name, value) = match command {
        SkillTapsCommand::List => (
            "list",
            client
                .get_json("/api/skills/taps", &EmptyQuery {})
                .await
                .map_err(gateway_error)?,
        ),
        SkillTapsCommand::Add {
            repo,
            path,
            branch,
            trust,
            replace,
            yes,
        } => {
            validate_repo(&repo)?;
            confirm("add skill tap", &repo, yes)?;
            (
                "add",
                client
                    .post_json_confirmed(
                        "/api/skills/taps",
                        &serde_json::json!({
                            "repo": repo,
                            "path": path,
                            "branch": branch,
                            "trust_level": trust,
                            "replace": replace
                        }),
                    )
                    .await
                    .map_err(gateway_error)?,
            )
        }
        SkillTapsCommand::Remove {
            repo,
            path,
            branch,
            yes,
        } => {
            validate_repo(&repo)?;
            confirm("remove skill tap", &repo, yes)?;
            (
                "remove",
                client
                    .post_json_confirmed(
                        "/api/skills/taps/remove",
                        &serde_json::json!({"repo": repo, "path": path, "branch": branch}),
                    )
                    .await
                    .map_err(gateway_error)?,
            )
        }
        SkillTapsCommand::Refresh { repo, path, yes } => {
            if let Some(repo) = repo.as_deref() {
                validate_repo(repo)?;
            }
            confirm(
                "refresh skill taps",
                repo.as_deref().unwrap_or("all configured taps"),
                yes,
            )?;
            (
                "refresh",
                client
                    .post_json_confirmed(
                        "/api/skills/taps/refresh",
                        &serde_json::json!({"repo": repo, "path": path}),
                    )
                    .await
                    .map_err(gateway_error)?,
            )
        }
    };
    write_value(
        context,
        match command_name {
            "list" => "extensions.skills.taps.list",
            "add" => "extensions.skills.taps.add",
            "remove" => "extensions.skills.taps.remove",
            _ => "extensions.skills.taps.refresh",
        },
        value,
    )
}

async fn check_candidate(args: SkillCheckArgs, context: &CliContext) -> Result<(), CliError> {
    let selected = usize::from(args.path.is_some())
        + usize::from(args.url.is_some())
        + usize::from(args.stdin);
    if selected != 1 {
        return Err(CliError::usage(
            "skill check requires exactly one of --path, --url, or --stdin",
        ));
    }
    let (content, source_kind, source_ref) = if let Some(path) = args.path {
        let content = read_bounded_file(&path)?;
        (content, "file", path.display().to_string())
    } else if let Some(url) = args.url {
        let content = crate::tools::builtin::skill_tools::fetch_skill_content(&url)
            .await
            .map_err(|error| CliError::operational(error.to_string()))?;
        (content, "url", url)
    } else {
        let mut content = String::new();
        std::io::stdin()
            .take(crate::skills::MAX_PROMPT_FILE_SIZE + 1)
            .read_to_string(&mut content)
            .map_err(|error| CliError::operational(error.to_string()))?;
        if content.len() as u64 > crate::skills::MAX_PROMPT_FILE_SIZE {
            return Err(CliError::usage("skill input exceeds the 64 KiB limit"));
        }
        (content, "stdin", "stdin".to_string())
    };
    let normalized = crate::skills::normalize_line_endings(&content);
    let parsed = crate::skills::parser::parse_skill_md(&normalized)
        .map_err(|error| CliError::usage(format!("invalid skill: {error}")))?;
    let quarantine = crate::skills::QuarantineManager::new(std::env::temp_dir());
    let report = quarantine.scan_report(&crate::skills::quarantine::QuarantinedSkill {
        skill_name: parsed.manifest.name,
        dir: PathBuf::new(),
        content: crate::skills::quarantine::SkillContent {
            raw_content: normalized,
            source_kind: source_kind.to_string(),
            source_adapter: "cli_check".to_string(),
            source_ref,
            source_repo: None,
            source_url: None,
            manifest_url: None,
            manifest_digest: None,
            path: None,
            branch: None,
            commit_sha: None,
            trust_level: crate::settings::SkillTapTrustLevel::Community,
        },
        package_files: Vec::new(),
    });
    context
        .output()
        .write_record("extensions.skills.check", &report, |report| {
            format!(
                "Scanner: {}\nDigest: {}\nFindings: {} (critical {}, warnings {})",
                report.scanner_version,
                report.content_sha256,
                report.summary.total,
                report.summary.critical,
                report.summary.warnings
            )
        })
}

async fn installed_skill_names(client: &GatewayClient) -> Result<Vec<String>, CliError> {
    let value: serde_json::Value = client
        .get_json("/api/skills", &EmptyQuery {})
        .await
        .map_err(gateway_error)?;
    value
        .get("skills")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::operational("gateway skill list response is malformed"))?
        .iter()
        .map(|skill| {
            skill
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| CliError::operational("gateway returned a nameless skill"))
        })
        .collect()
}

fn validate_skill_name(name: &str) -> Result<(), CliError> {
    if crate::skills::validate_skill_name(name) {
        Ok(())
    } else {
        Err(CliError::usage("invalid skill name"))
    }
}

fn skill_base_path(name: &str) -> Result<String, CliError> {
    validate_skill_name(name)?;
    Ok(format!("/api/skills/{name}"))
}

fn skill_path(name: &str, action: &str) -> Result<String, CliError> {
    Ok(format!("{}/{}", skill_base_path(name)?, action))
}

fn validate_repo(repo: &str) -> Result<(), CliError> {
    let mut parts = repo.split('/');
    let valid = matches!((parts.next(), parts.next(), parts.next()), (Some(owner), Some(name), None)
        if !owner.is_empty() && !name.is_empty()
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')));
    if valid {
        Ok(())
    } else {
        Err(CliError::usage("skill tap repository must be OWNER/REPO"))
    }
}

fn read_bounded_file(path: &std::path::Path) -> Result<String, CliError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        CliError::operational(format!("cannot inspect {}: {error}", path.display()))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CliError::usage(
            "skill source must be a regular non-symlink file",
        ));
    }
    if metadata.len() > crate::skills::MAX_PROMPT_FILE_SIZE {
        return Err(CliError::usage("skill source exceeds the 64 KiB limit"));
    }
    std::fs::read_to_string(path).map_err(|error| {
        CliError::operational(format!("failed to read {}: {error}", path.display()))
    })
}

async fn write_artifact(
    path: &std::path::Path,
    value: &serde_json::Value,
    force: bool,
) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| CliError::operational(error.to_string()))?;
    thinclaw_platform::write_private_file_atomic_async(path.to_path_buf(), bytes, force)
        .await
        .map_err(|error| {
            CliError::operational(format!("failed to write {}: {error}", path.display()))
        })
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn confirm(action: &str, target: &str, yes: bool) -> Result<(), CliError> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::usage(format!(
            "{action} requires --yes in noninteractive mode"
        )));
    }
    eprint!("{action} '{target}'? [y/N] ");
    std::io::stderr()
        .flush()
        .map_err(|error| CliError::operational(error.to_string()))?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| CliError::operational(error.to_string()))?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::operational(format!("{action} cancelled")))
    }
}

fn gateway_error(error: super::GatewayClientError) -> CliError {
    CliError::operational(error.to_string())
}

fn write_value(
    context: &CliContext,
    command: &'static str,
    value: serde_json::Value,
) -> Result<(), CliError> {
    context.output().write_record(command, &value, |value| {
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
    })
}

#[derive(Serialize)]
struct EmptyQuery {}
