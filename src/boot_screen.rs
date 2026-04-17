//! Boot screen displayed after all initialization completes.
//!
//! Shows a polished ANSI-styled status panel summarizing the agent's runtime
//! state: model, database, tool count, enabled features, active channels,
//! and the gateway URL.

use crate::terminal_branding::TerminalBranding;
use crate::tui::skin::CliSkin;

/// All displayable fields for the boot screen.
pub struct BootInfo {
    pub version: String,
    pub agent_name: String,
    pub llm_backend: String,
    pub llm_model: String,
    pub cheap_model: Option<String>,
    pub db_backend: String,
    pub db_connected: bool,
    pub tool_count: usize,
    pub gateway_url: Option<String>,
    pub embeddings_enabled: bool,
    pub embeddings_provider: Option<String>,
    pub heartbeat_enabled: bool,
    pub heartbeat_interval_secs: u64,
    pub sandbox_enabled: bool,
    pub docker_status: crate::sandbox::detect::DockerStatus,
    pub claude_code_enabled: bool,
    pub codex_code_enabled: bool,
    pub routines_enabled: bool,
    pub skills_enabled: bool,
    pub channels: Vec<String>,
    /// Public URL from a managed tunnel (e.g., "https://abc.ngrok.io").
    pub tunnel_url: Option<String>,
    /// Provider name for the managed tunnel (e.g., "ngrok").
    pub tunnel_provider: Option<String>,
    /// Local CLI skin name to use for the boot palette.
    pub cli_skin: String,
}

/// Print the boot screen to stdout.
pub fn print_boot_screen(info: &BootInfo) {
    let skin = CliSkin::load(&info.cli_skin);
    let branding = TerminalBranding::from_skin(skin.clone());
    let bold = "\x1b[1m";
    let accent = skin.ansi_fg(skin.accent);
    let muted = skin.ansi_fg(skin.muted);
    let warn = skin.ansi_fg(skin.warn);
    let reset = skin.ansi_reset();

    let border = format!("  {muted}{}{reset}", "\u{2576}".repeat(58));
    let mission = if info.db_connected {
        "Cockpit online. ThinClaw is ready for the next request."
    } else {
        "Cockpit online, with storage still warming up."
    };
    let readiness = if info.sandbox_enabled {
        if info.docker_status.is_ok() {
            "Sandboxing is available for work that needs a safer boundary."
        } else {
            "Sandboxing will activate when Docker becomes available."
        }
    } else {
        "Sandboxing is currently disabled."
    };
    let mut readiness_notes = Vec::new();
    readiness_notes.push(if info.db_connected {
        format!("{accent}database connected{reset}")
    } else {
        format!("{warn}database not connected{reset}")
    });
    readiness_notes.push(match info.docker_status {
        crate::sandbox::detect::DockerStatus::Available => {
            format!("{accent}sandbox host ready{reset}")
        }
        crate::sandbox::detect::DockerStatus::NotInstalled => {
            format!("{warn}sandbox host missing docker{reset}")
        }
        crate::sandbox::detect::DockerStatus::NotRunning => {
            format!("{warn}sandbox pending (start docker to activate){reset}")
        }
        crate::sandbox::detect::DockerStatus::Disabled => "sandbox disabled".to_string(),
    });
    if info.heartbeat_enabled {
        readiness_notes.push(format!(
            "{accent}heartbeat every {}m{reset}",
            info.heartbeat_interval_secs / 60
        ));
    }
    if info.embeddings_enabled {
        readiness_notes.push(match info.embeddings_provider.as_deref() {
            Some(provider) => format!("{accent}embeddings via {provider}{reset}"),
            None => format!("{accent}embeddings enabled{reset}"),
        });
    }

    for line in branding.banner_lines(&format!("{} v{}", info.agent_name, info.version), None) {
        println!("{line}");
    }
    println!("{border}");
    println!();
    println!("  {bold}{}{reset} v{}", info.agent_name, info.version);
    println!("  {muted}mission{reset}    {accent}{mission}{reset}");
    println!();

    println!("  {bold}Runtime{reset}");
    let model_display = if let Some(ref cheap) = info.cheap_model {
        format!(
            "{accent}{}{reset}  {muted}cheap{reset} {accent}{}{reset}",
            info.llm_model, cheap
        )
    } else {
        format!("{accent}{}{reset}", info.llm_model)
    };
    println!(
        "    {muted}model{reset}     {model_display}  {muted}via {}{reset}",
        info.llm_backend
    );

    let db_status = if info.db_connected {
        "connected"
    } else {
        "none"
    };
    println!(
        "    {muted}database{reset}  {accent}{}{reset} {muted}({db_status}){reset}",
        info.db_backend
    );
    println!(
        "    {muted}tools{reset}     {accent}{}{reset} {muted}registered{reset}",
        info.tool_count
    );
    if info.routines_enabled
        || info.skills_enabled
        || info.claude_code_enabled
        || info.codex_code_enabled
    {
        let mut feature_tags = Vec::new();
        if info.claude_code_enabled {
            feature_tags.push("claude-code");
        }
        if info.codex_code_enabled {
            feature_tags.push("codex-code");
        }
        if info.routines_enabled {
            feature_tags.push("routines");
        }
        if info.skills_enabled {
            feature_tags.push("skills");
        }
        println!(
            "    {muted}features{reset}  {accent}{}{reset}",
            feature_tags.join("  ")
        );
    }

    println!();
    println!("  {bold}Readiness{reset}");
    println!("    {muted}status{reset}    {accent}{readiness}{reset}");

    let mut features = Vec::new();
    if info.embeddings_enabled {
        if let Some(ref provider) = info.embeddings_provider {
            features.push(format!("embeddings ({provider})"));
        } else {
            features.push("embeddings".to_string());
        }
    }
    if info.heartbeat_enabled {
        let mins = info.heartbeat_interval_secs / 60;
        features.push(format!("heartbeat ({mins}m)"));
    }
    match info.docker_status {
        crate::sandbox::detect::DockerStatus::Available => {
            features.push("sandbox".to_string());
        }
        crate::sandbox::detect::DockerStatus::NotInstalled => {
            features.push(format!("{warn}sandbox (docker not installed){reset}"));
        }
        crate::sandbox::detect::DockerStatus::NotRunning => {
            features.push(format!("{warn}sandbox (pending docker){reset}"));
        }
        crate::sandbox::detect::DockerStatus::Disabled => {
            // Don't show sandbox when disabled
        }
    }
    if !features.is_empty() {
        println!(
            "    {muted}capabilities{reset}  {accent}{}{reset}",
            features.join("  ")
        );
    }
    if !info.channels.is_empty() {
        println!(
            "    {muted}channels{reset}  {accent}{}{reset}",
            info.channels.join("  ")
        );
    }
    println!("    {muted}note{reset}      {mission}");
    println!("    {muted}note{reset}      {readiness}");
    if !readiness_notes.is_empty() {
        println!(
            "    {muted}health{reset}     {accent}{}{reset}",
            readiness_notes.join("  ")
        );
    }

    println!();
    println!("  {bold}Access{reset}");
    if let Some(ref url) = info.gateway_url {
        // Show the full tokenized URL so the user can copy-paste it directly.
        // This is a local terminal on the operator's own machine — safe to display.
        println!(
            "    {muted}gateway{reset}   {warn}{url}{reset}",
        );
        if url.contains("token=") {
            println!(
                "    {muted}open{reset}      Copy the URL above and paste it into your browser."
            );
        }
    }
    if let Some(ref url) = info.tunnel_url {
        let provider_tag = info
            .tunnel_provider
            .as_deref()
            .map(|p| format!(" {muted}({p}){reset}"))
            .unwrap_or_default();
        println!("    {muted}tunnel{reset}    {warn}{url}{reset}{provider_tag}");
    }
    if info.gateway_url.is_none() && info.tunnel_url.is_none() {
        println!(
            "    {muted}direct cue{reset}  Connect a gateway or tunnel when you want remote access."
        );
    }

    println!();
    println!("{border}");
    println!();
    println!(
        "  {bold}/help{reset} for commands  {muted}•{reset}  {bold}/quit{reset} to exit  {muted}•{reset}  send a message to begin"
    );
    println!();
}

#[allow(dead_code)]
fn redact_gateway_url(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.to_string();
    };

    let mut pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(key, value)| {
            if key.eq_ignore_ascii_case("token") {
                (key.to_string(), "****".to_string())
            } else {
                (key.to_string(), value.to_string())
            }
        })
        .collect();

    if pairs.is_empty() {
        return parsed.to_string();
    }

    parsed.query_pairs_mut().clear().extend_pairs(pairs.drain(..));
    parsed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::detect::DockerStatus;

    #[test]
    fn test_print_boot_screen_full() {
        let info = BootInfo {
            version: "0.2.0".to_string(),
            agent_name: "thinclaw".to_string(),
            llm_backend: "nearai".to_string(),
            llm_model: "claude-3-5-sonnet-20241022".to_string(),
            cheap_model: Some("gpt-4o-mini".to_string()),
            db_backend: "libsql".to_string(),
            db_connected: true,
            tool_count: 24,
            gateway_url: Some("http://127.0.0.1:3001/?token=abc123".to_string()),
            embeddings_enabled: true,
            embeddings_provider: Some("openai".to_string()),
            heartbeat_enabled: true,
            heartbeat_interval_secs: 1800,
            sandbox_enabled: true,
            docker_status: DockerStatus::Available,
            claude_code_enabled: false,
            codex_code_enabled: false,
            routines_enabled: true,
            skills_enabled: true,
            channels: vec![
                "repl".to_string(),
                "gateway".to_string(),
                "telegram".to_string(),
            ],
            tunnel_url: Some("https://abc123.ngrok.io".to_string()),
            tunnel_provider: Some("ngrok".to_string()),
            cli_skin: "cockpit".to_string(),
        };
        // Should not panic
        print_boot_screen(&info);
    }

    #[test]
    fn test_print_boot_screen_minimal() {
        let info = BootInfo {
            version: "0.2.0".to_string(),
            agent_name: "thinclaw".to_string(),
            llm_backend: "nearai".to_string(),
            llm_model: "gpt-4o".to_string(),
            cheap_model: None,
            db_backend: "none".to_string(),
            db_connected: false,
            tool_count: 5,
            gateway_url: None,
            embeddings_enabled: false,
            embeddings_provider: None,
            heartbeat_enabled: false,
            heartbeat_interval_secs: 0,
            sandbox_enabled: false,
            docker_status: DockerStatus::Disabled,
            claude_code_enabled: false,
            codex_code_enabled: false,
            routines_enabled: false,
            skills_enabled: false,
            channels: vec![],
            tunnel_url: None,
            tunnel_provider: None,
            cli_skin: "cockpit".to_string(),
        };
        // Should not panic
        print_boot_screen(&info);
    }

    #[test]
    fn test_print_boot_screen_no_features() {
        let info = BootInfo {
            version: "0.1.0".to_string(),
            agent_name: "test".to_string(),
            llm_backend: "openai".to_string(),
            llm_model: "gpt-4o".to_string(),
            cheap_model: None,
            db_backend: "postgres".to_string(),
            db_connected: true,
            tool_count: 10,
            gateway_url: None,
            embeddings_enabled: false,
            embeddings_provider: None,
            heartbeat_enabled: false,
            heartbeat_interval_secs: 0,
            sandbox_enabled: false,
            docker_status: DockerStatus::Disabled,
            claude_code_enabled: false,
            codex_code_enabled: false,
            routines_enabled: false,
            skills_enabled: false,
            channels: vec!["repl".to_string()],
            tunnel_url: None,
            tunnel_provider: None,
            cli_skin: "cockpit".to_string(),
        };
        // Should not panic
        print_boot_screen(&info);
    }
}
