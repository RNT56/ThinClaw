//! Compact, typed capability and readiness snapshots.

use clap::{Args, Subcommand, ValueEnum};
use thinclaw_app::{
    ActivityState, CapabilityFact, CapabilitySnapshot, DependencyState, FactState, HealthState,
    ReadinessProfile, ReadinessState, ToolCapabilityFact, ToolCapabilitySnapshot,
};

use crate::cli::{CliContext, CliError, CliOutcome, GatewayClient, ReadinessProfileArg};
use crate::registry::{ManifestKind, RegistryCatalog};
use crate::settings::Settings;

#[derive(Args, Debug, Clone)]
pub struct StatusArgs {
    #[command(flatten)]
    pub scope: StatusScopeArgs,

    #[command(subcommand)]
    pub area: Option<StatusArea>,
}

#[derive(Args, Debug, Clone)]
pub struct StatusScopeArgs {
    /// Readiness profile used to classify required capabilities
    #[arg(
        long = "readiness-profile",
        alias = "profile",
        value_enum,
        default_value_t = ReadinessProfileArg::Server,
        global = true
    )]
    pub readiness_profile: ReadinessProfileArg,

    /// Run bounded, non-billable dependency probes
    #[arg(long, global = true)]
    pub live: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum StatusArea {
    /// Inspect the tool capability catalog
    Tools(ToolStatusArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct ToolStatusArgs {
    /// Exact case-sensitive stable tool ID
    #[arg(conflicts_with = "match_pattern")]
    pub name: Option<String>,

    /// Include the complete static catalog
    #[arg(long)]
    pub all: bool,

    /// Match stable IDs with literal characters plus `*` and `?`
    #[arg(long = "match")]
    pub match_pattern: Option<String>,

    #[arg(long, value_enum)]
    pub origin: Vec<ToolOriginArg>,
    #[arg(long, value_enum)]
    pub compiled: Vec<FactStateArg>,
    #[arg(long, value_enum)]
    pub configured: Vec<FactStateArg>,
    #[arg(long, value_enum)]
    pub registered: Vec<FactStateArg>,
    #[arg(long, value_enum)]
    pub dependency: Vec<DependencyStateArg>,
    #[arg(long, value_enum)]
    pub exposed: Vec<FactStateArg>,
    #[arg(long, value_enum)]
    pub approval: Vec<ApprovalPolicyArg>,
    #[arg(long, value_enum)]
    pub health: Vec<HealthStateArg>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOriginArg {
    Core,
    Memory,
    Dev,
    Job,
    ExtensionAdmin,
    Skill,
    Learning,
    RepoProject,
    Media,
    Desktop,
    HardwareBridge,
    Channel,
    Subagent,
    Llm,
    Agent,
    Routine,
    Wasm,
    Mcp,
    UserTool,
    NativePlugin,
    Registry,
}

impl ToolOriginArg {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Memory => "memory",
            Self::Dev => "dev",
            Self::Job => "job",
            Self::ExtensionAdmin => "extension-admin",
            Self::Skill => "skill",
            Self::Learning => "learning",
            Self::RepoProject => "repo-project",
            Self::Media => "media",
            Self::Desktop => "desktop",
            Self::HardwareBridge => "hardware-bridge",
            Self::Channel => "channel",
            Self::Subagent => "subagent",
            Self::Llm => "llm",
            Self::Agent => "agent",
            Self::Routine => "routine",
            Self::Wasm => "wasm",
            Self::Mcp => "mcp",
            Self::UserTool => "user-tool",
            Self::NativePlugin => "native-plugin",
            Self::Registry => "registry",
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactStateArg {
    Yes,
    No,
    Unknown,
    NotApplicable,
}

impl From<FactStateArg> for FactState {
    fn from(value: FactStateArg) -> Self {
        match value {
            FactStateArg::Yes => Self::Yes,
            FactStateArg::No => Self::No,
            FactStateArg::Unknown => Self::Unknown,
            FactStateArg::NotApplicable => Self::NotApplicable,
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyStateArg {
    Available,
    Missing,
    Unknown,
    NotApplicable,
}

impl From<DependencyStateArg> for DependencyState {
    fn from(value: DependencyStateArg) -> Self {
        match value {
            DependencyStateArg::Available => Self::Available,
            DependencyStateArg::Missing => Self::Missing,
            DependencyStateArg::Unknown => Self::Unknown,
            DependencyStateArg::NotApplicable => Self::NotApplicable,
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPolicyArg {
    Never,
    Conditional,
    Always,
}

impl ApprovalPolicyArg {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Conditional => "conditional",
            Self::Always => "always",
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStateArg {
    Healthy,
    Unhealthy,
    Unknown,
    NotProbed,
    NotSupported,
}

impl From<HealthStateArg> for HealthState {
    fn from(value: HealthStateArg) -> Self {
        match value {
            HealthStateArg::Healthy => Self::Healthy,
            HealthStateArg::Unhealthy => Self::Unhealthy,
            HealthStateArg::Unknown => Self::Unknown,
            HealthStateArg::NotProbed => Self::NotProbed,
            HealthStateArg::NotSupported => Self::NotSupported,
        }
    }
}

pub async fn run_status_command(
    args: StatusArgs,
    context: &CliContext,
) -> Result<CliOutcome, CliError> {
    if let Some(StatusArea::Tools(tool_args)) = &args.area {
        return run_tool_status(&args.scope, tool_args, context).await;
    }

    let settings = Settings::load();
    let profile: ReadinessProfile = args.scope.readiness_profile.into();
    let readiness = if args.scope.live {
        Some(crate::platform::linux_readiness_report(args.scope.readiness_profile.into()).await)
    } else {
        None
    };
    let mut snapshot = CapabilitySnapshot {
        schema_version: 1,
        revision: String::new(),
        profile: profile.as_str().to_string(),
        runtime_active: FactState::Unknown,
        healthy: readiness.as_ref().is_none_or(|report| report.failed() == 0),
        facts: vec![
            database_fact(),
            llm_fact(&settings),
            gateway_fact(&settings),
            embeddings_fact(&settings),
            wasm_fact(&settings),
            sandbox_fact(&settings),
            heartbeat_fact(&settings),
            readiness
                .as_ref()
                .map_or_else(platform_static_fact, linux_fact),
        ],
    };
    snapshot.sort_facts();
    let encoded = serde_json::to_vec(&snapshot.facts).map_err(|error| {
        CliError::operational(format!("failed to encode capability snapshot: {error}"))
    })?;
    snapshot.revision = blake3::hash(&encoded).to_hex().to_string();

    context
        .output()
        .write_record("status", &snapshot, render_human)?;
    Ok(if snapshot.healthy {
        CliOutcome::Success
    } else {
        CliOutcome::Unhealthy
    })
}

async fn run_tool_status(
    scope: &StatusScopeArgs,
    args: &ToolStatusArgs,
    context: &CliContext,
) -> Result<CliOutcome, CliError> {
    validate_selector(args.name.as_deref(), "NAME")?;
    validate_pattern(args.match_pattern.as_deref())?;

    let live_snapshot = if scope.live {
        let config = context.config().await?;
        let client = GatewayClient::resolve_from_config(None, None, config)
            .map_err(|error| CliError::operational(error.to_string()))?;
        Some(
            client
                .get_json::<_, crate::tools::RegistrySnapshot>(
                    "/api/capabilities/tools",
                    &Vec::<(String, String)>::new(),
                )
                .await
                .map_err(|error| CliError::operational(error.to_string()))?,
        )
    } else {
        None
    };

    let live_by_name = live_snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .identities
                .iter()
                .map(|identity| (identity.name.as_str(), identity))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut tools = thinclaw_tools::STATIC_TOOL_CATALOG
        .iter()
        .map(|descriptor| static_tool_fact(descriptor, live_by_name.get(descriptor.name).copied()))
        .collect::<Vec<_>>();

    for identity in live_snapshot
        .as_ref()
        .into_iter()
        .flat_map(|snapshot| snapshot.identities.iter())
        .filter(|identity| thinclaw_tools::static_tool_descriptor(&identity.name).is_none())
    {
        tools.push(live_dynamic_tool_fact(identity));
    }

    let catalog = RegistryCatalog::load_or_embedded()
        .map_err(|error| CliError::operational(format!("failed to load tool catalog: {error}")))?;
    for manifest in catalog.list(Some(ManifestKind::Tool), None) {
        if !tools.iter().any(|tool| tool.name == manifest.name) {
            tools.push(registry_tool_fact(manifest, scope.live));
        }
    }

    if let Some(name) = args.name.as_deref()
        && !tools.iter().any(|tool| tool.name == name)
    {
        return Err(CliError::operational(format!(
            "tool '{name}' was not found in the static or installed catalog"
        )));
    }

    let explicit_population = args.name.is_some() || args.match_pattern.is_some() || args.all;
    tools.retain(|tool| {
        (explicit_population || tool.registered == FactState::Yes)
            && args.name.as_ref().is_none_or(|name| tool.name == *name)
            && args
                .match_pattern
                .as_ref()
                .is_none_or(|pattern| wildcard_matches(pattern, &tool.name))
            && matches_string_filter(&args.origin, &tool.origin, ToolOriginArg::as_str)
            && matches_fact_filter(&args.compiled, tool.compiled)
            && matches_fact_filter(&args.configured, tool.configured)
            && matches_fact_filter(&args.registered, tool.registered)
            && matches_dependency_filter(&args.dependency, tool.dependency)
            && matches_fact_filter(&args.exposed, tool.exposed)
            && matches_string_filter(&args.approval, &tool.approval, ApprovalPolicyArg::as_str)
            && matches_health_filter(&args.health, tool.health)
    });
    tools.sort_by(|left, right| {
        (&left.origin, &left.name, &left.source_id).cmp(&(
            &right.origin,
            &right.name,
            &right.source_id,
        ))
    });

    let revision = if let Some(snapshot) = live_snapshot {
        snapshot.revision.to_string()
    } else {
        let revision_bytes = serde_json::to_vec(&tools).map_err(|error| {
            CliError::operational(format!("failed to encode tool status: {error}"))
        })?;
        blake3::hash(&revision_bytes).to_hex().to_string()
    };
    let snapshot = ToolCapabilitySnapshot {
        schema_version: 1,
        revision,
        readiness_profile: scope.readiness_profile.into(),
        live: scope.live,
        tools,
    };
    context
        .output()
        .write_record("status_tools", &snapshot, render_tools_human)?;
    Ok(CliOutcome::Success)
}

fn static_tool_fact(
    descriptor: &thinclaw_tools::StaticToolDescriptor,
    live: Option<&thinclaw_tools::RegistryIdentity>,
) -> ToolCapabilityFact {
    let compiled = match descriptor.name {
        "extract_document" if !cfg!(feature = "document-extraction") => FactState::No,
        "browser" if !cfg!(feature = "browser") => FactState::No,
        "nostr_actions" if !cfg!(feature = "nostr") => FactState::No,
        "apple_mail" if !cfg!(target_os = "macos") => FactState::No,
        _ => FactState::Yes,
    };
    let registered = yes_no(live.is_some());
    ToolCapabilityFact {
        name: descriptor.name.to_string(),
        source_id: live
            .map(|identity| identity.source_id.clone())
            .unwrap_or_else(|| format!("builtin/{}", descriptor.name)),
        label: descriptor.name.replace('_', " "),
        origin: live
            .map(|identity| identity.origin.to_string())
            .unwrap_or_else(|| descriptor.origin.to_string()),
        compiled,
        configured: live
            .and_then(|identity| identity.configured)
            .map_or(FactState::Unknown, yes_no),
        registered: live.map_or(registered, |identity| yes_no(identity.registered)),
        dependency: if compiled == FactState::No {
            DependencyState::Missing
        } else if let Some(identity) = live {
            match identity.dependency.as_str() {
                "available" => DependencyState::Available,
                "missing" => DependencyState::Missing,
                "not_applicable" => DependencyState::NotApplicable,
                _ => DependencyState::Unknown,
            }
        } else {
            DependencyState::Unknown
        },
        exposed: live.map_or_else(
            || {
                if HIDDEN_TOOL_NAMES.contains(&descriptor.name) {
                    FactState::No
                } else {
                    registered
                }
            },
            |identity| yes_no(identity.exposed),
        ),
        approval: live
            .map(|identity| identity.approval.clone())
            .unwrap_or_else(|| "conditional".to_string()),
        health: if let Some(identity) = live {
            match identity.health.as_str() {
                "healthy" => HealthState::Healthy,
                "unhealthy" => HealthState::Unhealthy,
                "not_probed" => HealthState::NotProbed,
                "not_supported" => HealthState::NotSupported,
                _ => HealthState::Unknown,
            }
        } else {
            HealthState::NotProbed
        },
        reasons: if let Some(identity) = live {
            identity.reasons.clone()
        } else if compiled == FactState::No {
            vec!["capability is not compiled for this build or platform".to_string()]
        } else if live.is_none() {
            vec!["capability is catalogued but absent from the live registry".to_string()]
        } else {
            Vec::new()
        },
    }
}

fn live_dynamic_tool_fact(identity: &thinclaw_tools::RegistryIdentity) -> ToolCapabilityFact {
    ToolCapabilityFact {
        name: identity.name.clone(),
        source_id: identity.source_id.clone(),
        label: identity.name.replace('_', " "),
        origin: identity.origin.to_string(),
        compiled: yes_no(identity.compiled),
        configured: identity.configured.map_or(FactState::Unknown, yes_no),
        registered: yes_no(identity.registered),
        dependency: match identity.dependency.as_str() {
            "available" => DependencyState::Available,
            "missing" => DependencyState::Missing,
            "not_applicable" => DependencyState::NotApplicable,
            _ => DependencyState::Unknown,
        },
        exposed: yes_no(identity.exposed),
        approval: identity.approval.clone(),
        health: match identity.health.as_str() {
            "healthy" => HealthState::Healthy,
            "unhealthy" => HealthState::Unhealthy,
            "not_probed" => HealthState::NotProbed,
            "not_supported" => HealthState::NotSupported,
            _ => HealthState::Unknown,
        },
        reasons: identity.reasons.clone(),
    }
}

const HIDDEN_TOOL_NAMES: &[&str] = &[
    "external_memory_recall",
    "external_memory_export",
    "external_memory_setup",
    "external_memory_off",
    "external_memory_status",
];

fn registry_tool_fact(
    manifest: &crate::registry::ExtensionManifest,
    live: bool,
) -> ToolCapabilityFact {
    let installed = dirs::home_dir().is_some_and(|home| {
        let directory = home.join(".thinclaw/tools");
        directory.join(format!("{}.wasm", manifest.name)).is_file()
            || directory.join(&manifest.name).is_dir()
    });
    let compiled = if cfg!(feature = "wasm-runtime") {
        FactState::Yes
    } else {
        FactState::No
    };
    let registered = if installed && compiled == FactState::Yes {
        FactState::Yes
    } else {
        FactState::No
    };
    let auth_free = manifest
        .auth_summary
        .as_ref()
        .and_then(|summary| summary.method.as_deref())
        .is_none_or(|method| method == "none");
    ToolCapabilityFact {
        name: manifest.name.clone(),
        source_id: format!("registry/tools/{}", manifest.name),
        label: manifest.display_name.clone(),
        origin: "registry".to_string(),
        compiled,
        configured: if auth_free {
            FactState::Yes
        } else {
            FactState::Unknown
        },
        registered,
        dependency: if compiled == FactState::Yes {
            DependencyState::Available
        } else {
            DependencyState::Missing
        },
        exposed: registered,
        approval: "conditional".to_string(),
        health: if live {
            HealthState::NotSupported
        } else {
            HealthState::NotProbed
        },
        reasons: if installed && compiled == FactState::No {
            vec!["installed WASM tool cannot load because wasm-runtime is not compiled".to_string()]
        } else {
            Vec::new()
        },
    }
}

fn validate_selector(value: Option<&str>, label: &str) -> Result<(), CliError> {
    if let Some(value) = value
        && (value.len() > 256 || value.chars().any(char::is_control))
    {
        return Err(CliError::usage(format!(
            "{label} must be at most 256 bytes and contain no control characters"
        )));
    }
    Ok(())
}

fn validate_pattern(value: Option<&str>) -> Result<(), CliError> {
    validate_selector(value, "--match")?;
    if let Some(value) = value
        && value
            .chars()
            .any(|character| matches!(character, '[' | ']' | '\\'))
    {
        return Err(CliError::usage(
            "--match supports only literal characters plus '*' and '?'; brackets and escaping are not allowed",
        ));
    }
    Ok(())
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut row = vec![false; value.len() + 1];
    row[0] = true;
    for &token in pattern {
        let previous = row.clone();
        row[0] = token == b'*' && previous[0];
        for index in 1..=value.len() {
            row[index] = match token {
                b'*' => row[index - 1] || previous[index],
                b'?' => previous[index - 1],
                literal => previous[index - 1] && literal == value[index - 1],
            };
        }
    }
    row[value.len()]
}

fn matches_fact_filter(filters: &[FactStateArg], value: FactState) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|filter| FactState::from(*filter) == value)
}

fn matches_dependency_filter(filters: &[DependencyStateArg], value: DependencyState) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|filter| DependencyState::from(*filter) == value)
}

fn matches_health_filter(filters: &[HealthStateArg], value: HealthState) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|filter| HealthState::from(*filter) == value)
}

fn matches_string_filter<T: Copy>(
    filters: &[T],
    value: &str,
    render: impl Fn(T) -> &'static str,
) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .copied()
            .any(|filter| render(filter) == value)
}

fn database_fact() -> CapabilityFact {
    let backend = std::env::var("DATABASE_BACKEND").unwrap_or_else(|_| "postgres".to_string());
    let compiled = match backend.as_str() {
        "libsql" | "sqlite" | "turso" => {
            if cfg!(feature = "libsql") {
                FactState::Yes
            } else {
                FactState::No
            }
        }
        _ => {
            if cfg!(feature = "postgres") {
                FactState::Yes
            } else {
                FactState::No
            }
        }
    };
    let configured = match backend.as_str() {
        "libsql" | "sqlite" | "turso" => FactState::Yes,
        _ if std::env::var_os("DATABASE_URL").is_some() => FactState::Yes,
        _ => FactState::No,
    };
    CapabilityFact {
        id: "database".to_string(),
        label: format!("Database ({backend})"),
        compiled,
        configured,
        available: FactState::Unknown,
        active: ActivityState::Unknown,
        ready: if compiled == FactState::No || configured == FactState::No {
            ReadinessState::NotReady
        } else {
            ReadinessState::Unknown
        },
        reasons: vec!["static status does not open the database".to_string()],
        remediation: Vec::new(),
    }
}

fn llm_fact(_settings: &Settings) -> CapabilityFact {
    let configured = std::env::var_os("LLM_BASE_URL").is_some()
        || std::env::var_os("OPENAI_API_KEY").is_some()
        || std::env::var_os("ANTHROPIC_API_KEY").is_some()
        || std::env::var_os("LLM_BACKEND").is_some();
    fact(
        "llm",
        "Language model",
        FactState::Yes,
        yes_no(configured),
        FactState::Unknown,
        ActivityState::Inactive,
        if configured {
            ReadinessState::Unknown
        } else {
            ReadinessState::NotReady
        },
    )
}

fn gateway_fact(settings: &Settings) -> CapabilityFact {
    let access =
        crate::platform::gateway_access::GatewayAccessInfo::from_env_and_settings(Some(settings));
    let mut value = fact(
        "gateway",
        "Web gateway",
        FactState::Yes,
        yes_no(access.enabled),
        FactState::Unknown,
        ActivityState::Unknown,
        if access.enabled && access.auth_token.is_some() {
            ReadinessState::Unknown
        } else {
            ReadinessState::NotReady
        },
    );
    if access.auth_token.is_none() {
        value
            .reasons
            .push("gateway authentication token is unavailable".to_string());
        value
            .remediation
            .push("configure a gateway credential source".to_string());
    }
    value
}

fn embeddings_fact(settings: &Settings) -> CapabilityFact {
    fact(
        "embeddings",
        "Embeddings",
        FactState::Yes,
        yes_no(settings.embeddings.enabled),
        FactState::Unknown,
        ActivityState::Inactive,
        if settings.embeddings.enabled {
            ReadinessState::Unknown
        } else {
            ReadinessState::NotApplicable
        },
    )
}

fn wasm_fact(settings: &Settings) -> CapabilityFact {
    fact(
        "wasm_runtime",
        "WASM extensions",
        yes_no(cfg!(feature = "wasm-runtime")),
        yes_no(settings.wasm.enabled),
        FactState::Unknown,
        ActivityState::Inactive,
        if !cfg!(feature = "wasm-runtime") {
            ReadinessState::NotApplicable
        } else if settings.wasm.enabled {
            ReadinessState::Unknown
        } else {
            ReadinessState::NotApplicable
        },
    )
}

fn sandbox_fact(settings: &Settings) -> CapabilityFact {
    fact(
        "sandbox",
        "Sandbox jobs",
        yes_no(cfg!(feature = "docker-sandbox")),
        yes_no(settings.sandbox.enabled),
        FactState::Unknown,
        ActivityState::Inactive,
        if !cfg!(feature = "docker-sandbox") || !settings.sandbox.enabled {
            ReadinessState::NotApplicable
        } else {
            ReadinessState::Unknown
        },
    )
}

fn heartbeat_fact(settings: &Settings) -> CapabilityFact {
    fact(
        "heartbeat",
        "Heartbeat",
        FactState::Yes,
        yes_no(settings.heartbeat.enabled),
        FactState::NotApplicable,
        ActivityState::Inactive,
        if settings.heartbeat.enabled {
            ReadinessState::Unknown
        } else {
            ReadinessState::NotApplicable
        },
    )
}

fn linux_fact(report: &crate::platform::LinuxReadinessReport) -> CapabilityFact {
    let mut value = fact(
        "platform_readiness",
        "Platform readiness",
        FactState::Yes,
        FactState::Yes,
        if report.failed() == 0 {
            FactState::Yes
        } else {
            FactState::No
        },
        ActivityState::NotApplicable,
        if report.failed() == 0 {
            ReadinessState::Ready
        } else {
            ReadinessState::NotReady
        },
    );
    value.reasons = report
        .probes
        .iter()
        .filter(|probe| probe.status == crate::platform::LinuxProbeStatus::Fail)
        .map(|probe| format!("{}: {}", probe.label, probe.detail))
        .collect();
    value
}

fn platform_static_fact() -> CapabilityFact {
    let mut value = fact(
        "platform_readiness",
        "Platform readiness",
        FactState::Yes,
        FactState::Yes,
        FactState::Unknown,
        ActivityState::NotApplicable,
        ReadinessState::Unknown,
    );
    value
        .reasons
        .push("static status does not execute platform probes; use --live".to_string());
    value
}

#[allow(clippy::too_many_arguments)]
fn fact(
    id: &str,
    label: &str,
    compiled: FactState,
    configured: FactState,
    available: FactState,
    active: ActivityState,
    ready: ReadinessState,
) -> CapabilityFact {
    CapabilityFact {
        id: id.to_string(),
        label: label.to_string(),
        compiled,
        configured,
        available,
        active,
        ready,
        reasons: Vec::new(),
        remediation: Vec::new(),
    }
}

const fn yes_no(value: bool) -> FactState {
    if value { FactState::Yes } else { FactState::No }
}

fn render_human(snapshot: &CapabilitySnapshot) -> String {
    let mut lines = vec![format!(
        "ThinClaw {} — profile {} — revision {}",
        if snapshot.healthy {
            "ready"
        } else {
            "not ready"
        },
        snapshot.profile,
        &snapshot.revision[..12]
    )];
    for fact in &snapshot.facts {
        lines.push(format!(
            "{}: compiled={:?}, configured={:?}, available={:?}, active={:?}, ready={:?}",
            fact.label, fact.compiled, fact.configured, fact.available, fact.active, fact.ready
        ));
        for reason in &fact.reasons {
            lines.push(format!("  - {reason}"));
        }
    }
    lines.join("\n")
}

fn render_tools_human(snapshot: &ToolCapabilitySnapshot) -> String {
    let mut lines = vec![format!(
        "{} tools — profile {} — revision {}{}",
        snapshot.tools.len(),
        snapshot.readiness_profile.as_str(),
        &snapshot.revision[..12],
        if snapshot.live { " — live" } else { "" }
    )];
    for tool in &snapshot.tools {
        lines.push(format!(
            "{}: origin={}, compiled={:?}, configured={:?}, registered={:?}, dependency={:?}, exposed={:?}, approval={}, health={:?}",
            tool.name,
            tool.origin,
            tool.compiled,
            tool.configured,
            tool.registered,
            tool.dependency,
            tool.exposed,
            tool.approval,
            tool.health
        ));
        for reason in &tool.reasons {
            lines.push(format!("  - {reason}"));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_wildcards_are_literal_except_star_and_question_mark() {
        assert!(wildcard_matches("git*", "github"));
        assert!(wildcard_matches("g?t", "git"));
        assert!(!wildcard_matches("git", "github"));
        assert!(validate_pattern(Some("tool-[x]")).is_err());
    }
}
