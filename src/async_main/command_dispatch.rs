//! Dispatch for terminal CLI commands that do not enter the agent runtime.

use super::*;

pub(super) async fn run_terminal_command(
    cli: &Cli,
    context: &thinclaw::cli::CliContext,
) -> Result<thinclaw::cli::CliDispatch, thinclaw::cli::CliError> {
    match cli.command.as_ref() {
        None
        | Some(
            Command::Run(_)
            | Command::Tui(_)
            | Command::Ask { .. }
            | Command::Onboard { .. }
            | Command::AutonomyShadowCanary { .. },
        ) => return Ok(thinclaw::cli::CliDispatch::Runtime),
        _ => {}
    }

    debug_assert_eq!(context.debug(), cli.debug);

    let result = match &cli.command {
        Some(Command::Tool(tool_cmd)) => {
            init_cli_tracing(cli.debug);
            run_tool_command(tool_cmd.clone()).await
        }
        Some(Command::Config(config_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_config_command(config_cmd.clone()).await
        }
        Some(Command::Registry(registry_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_registry_command(registry_cmd.clone()).await
        }
        Some(Command::RepoProjects(rp_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_repo_projects_command(rp_cmd.clone()).await
        }
        Some(Command::Backup(backup_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_backup_command(backup_cmd.clone()).await
        }
        Some(Command::Mcp(mcp_cmd)) => {
            init_cli_tracing(cli.debug);
            run_mcp_command(mcp_cmd.clone()).await
        }
        Some(Command::Memory(mem_cmd)) => {
            init_cli_tracing(cli.debug);
            run_memory_command(mem_cmd).await
        }
        Some(Command::Pairing(pairing_cmd)) => {
            init_cli_tracing(cli.debug);
            run_pairing_command(pairing_cmd.clone())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        Some(Command::Devices(device_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_devices_command(device_cmd.clone()).await
        }
        #[cfg(feature = "repl")]
        Some(Command::Service(service_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_service_command(service_cmd)
        }
        #[cfg(all(feature = "repl", target_os = "windows"))]
        Some(Command::WindowsServiceRuntime { home }) => {
            thinclaw::service::run_windows_service_dispatcher(home.clone())
        }
        Some(Command::Doctor { profile }) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_doctor_command((*profile).into()).await
        }
        Some(Command::Status { profile }) => {
            init_cli_tracing(cli.debug);
            run_status_command((*profile).into()).await
        }
        Some(Command::Reset(reset_cmd)) => {
            init_cli_tracing(cli.debug);
            run_reset_command(reset_cmd.clone()).await
        }
        Some(Command::Secrets(secrets_cmd)) => {
            init_cli_tracing(cli.debug);
            run_secrets_command(secrets_cmd.clone()).await
        }
        Some(Command::Cron(cron_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_cron_command(cron_cmd.clone()).await
        }
        Some(Command::Experiments(experiments_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_experiments_command(experiments_cmd.clone()).await
        }
        Some(Command::Gateway(gw_cmd)) => {
            init_cli_tracing(cli.debug);
            run_gateway_command(gw_cmd.clone()).await
        }
        Some(Command::Identity(identity_cmd)) => {
            init_cli_tracing(cli.debug);
            run_identity_command(identity_cmd.clone()).await
        }
        Some(Command::Channels(ch_cmd)) => {
            init_cli_tracing(cli.debug);
            run_channels_command(ch_cmd.clone()).await
        }
        Some(Command::Comfy(comfy_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_comfy_command(comfy_cmd.clone()).await
        }
        Some(Command::Message(msg_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_message_command(msg_cmd.clone(), context).await
        }
        Some(Command::Send {
            text,
            user_id,
            gateway_url,
        }) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_message_command(
                thinclaw::cli::MessageCommand::Send {
                    text: text.clone(),
                    user_id: user_id.clone(),
                    gateway_url: gateway_url.clone(),
                },
                context,
            )
            .await
        }
        Some(Command::Models(model_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_model_command(model_cmd.clone()).await
        }
        Some(Command::Completion(completion)) => {
            init_cli_tracing(cli.debug);
            completion.run()
        }
        #[cfg(feature = "docker-sandbox")]
        Some(Command::Worker {
            job_id,
            orchestrator_url,
            max_iterations,
        }) => {
            init_worker_tracing();
            run_worker(*job_id, orchestrator_url, *max_iterations).await
        }
        #[cfg(feature = "docker-sandbox")]
        Some(Command::ClaudeBridge {
            job_id,
            orchestrator_url,
            max_turns,
            model,
        }) => {
            init_worker_tracing();
            run_claude_bridge(*job_id, orchestrator_url, *max_turns, model).await
        }
        #[cfg(feature = "docker-sandbox")]
        Some(Command::CodexBridge {
            job_id,
            orchestrator_url,
            model,
        }) => {
            init_worker_tracing();
            run_codex_bridge(*job_id, orchestrator_url, model).await
        }
        #[cfg(feature = "docker-sandbox")]
        Some(Command::NetworkRelay { forwards }) => {
            init_worker_tracing();
            run_network_relay(forwards).await
        }
        Some(Command::Agents(agent_cmd)) => {
            init_cli_tracing(cli.debug);
            // In standalone CLI mode, create a fresh router.
            // Runtime agent routing state is in-memory only.
            let router = thinclaw::agent::AgentRouter::new();
            thinclaw::cli::run_agents_command(agent_cmd.clone(), &router).await;
            Ok(())
        }
        Some(Command::Sessions(session_cmd)) => {
            init_cli_tracing(cli.debug);
            // In standalone CLI mode, create a fresh session manager.
            // Runtime session state is in-memory only.
            let mgr = std::sync::Arc::new(thinclaw::agent::SessionManager::new());
            thinclaw::cli::run_sessions_command(session_cmd.clone(), &mgr).await;
            Ok(())
        }
        Some(Command::Logs(log_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_log_command(log_cmd.clone()).await
        }
        Some(Command::Browser(browser_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_browser_command(browser_cmd.clone()).await
        }
        Some(Command::Trajectory(trajectory_cmd)) => {
            init_cli_tracing(cli.debug);
            run_trajectory_command(trajectory_cmd.clone()).await
        }
        Some(Command::ExperimentRunner {
            gateway_url,
            auth_stdin,
            auth_file,
            workspace_root,
        }) => {
            init_cli_tracing(cli.debug);
            let auth = thinclaw::experiments::runner_auth::read_runner_auth(
                *auth_stdin,
                auth_file.as_deref(),
            )?;
            let workspace_root =
                thinclaw::experiments::runner_auth::resolve_workspace_root(workspace_root.clone())?;
            thinclaw::experiments::runner::run_remote_runner(
                gateway_url,
                auth.lease_id,
                secrecy::ExposeSecret::expose_secret(&auth.token),
                workspace_root,
            )
            .await
        }
        Some(Command::Update(update_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_update_command(update_cmd.clone()).await
        }
        _ => unreachable!("runtime command must be handled by the caller"),
    };

    result.map_err(thinclaw::cli::CliError::from)?;
    Ok(thinclaw::cli::CliDispatch::Handled(
        thinclaw::cli::CliOutcome::Success,
    ))
}
