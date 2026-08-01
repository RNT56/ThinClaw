//! Compact, typed capability and readiness snapshot.

use thinclaw_app::{ActivityState, CapabilityFact, CapabilitySnapshot, FactState, ReadinessState};

use crate::cli::{CliContext, CliError, CliOutcome};
use crate::settings::Settings;

pub async fn run_status_command(
    linux_profile: crate::platform::LinuxReadinessProfile,
    context: &CliContext,
) -> Result<CliOutcome, CliError> {
    let settings = Settings::load();
    let linux = crate::platform::linux_readiness_report(linux_profile).await;
    let mut snapshot = CapabilitySnapshot {
        schema_version: 1,
        revision: String::new(),
        profile: linux_profile.as_str().to_string(),
        runtime_active: FactState::Unknown,
        healthy: linux.failed() == 0,
        facts: vec![
            database_fact(),
            llm_fact(&settings),
            gateway_fact(&settings),
            embeddings_fact(&settings),
            wasm_fact(&settings),
            sandbox_fact(&settings),
            heartbeat_fact(&settings),
            linux_fact(&linux),
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
