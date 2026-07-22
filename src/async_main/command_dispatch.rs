//! Dispatch for terminal CLI commands that do not enter the agent runtime.

use super::*;

pub(super) async fn run_terminal_command(
    cli: &Cli,
    env_bootstrap_plan: RuntimeEnvBootstrapPlan,
) -> Option<anyhow::Result<()>> {
    match cli.command.as_ref() {
        None
        | Some(
            Command::Run
            | Command::Tui
            | Command::Onboard { .. }
            | Command::AutonomyShadowCanary { .. },
        ) => return None,
        _ => {}
    }

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
            execute_env_bootstrap_plan(env_bootstrap_plan);
            thinclaw::cli::run_doctor_command((*profile).into()).await
        }
        Some(Command::Status { profile }) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            run_status_command((*profile).into()).await
        }
        Some(Command::Reset(reset_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            run_reset_command(reset_cmd.clone()).await
        }
        Some(Command::Secrets(secrets_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            run_secrets_command(secrets_cmd.clone()).await
        }
        Some(Command::Cron(cron_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            thinclaw::cli::run_cron_command(cron_cmd.clone()).await
        }
        Some(Command::Experiments(experiments_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            thinclaw::cli::run_experiments_command(experiments_cmd.clone()).await
        }
        Some(Command::Gateway(gw_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            run_gateway_command(gw_cmd.clone()).await
        }
        Some(Command::Identity(identity_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            run_identity_command(identity_cmd.clone()).await
        }
        Some(Command::Channels(ch_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            run_channels_command(ch_cmd.clone()).await
        }
        Some(Command::Comfy(comfy_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_comfy_command(comfy_cmd.clone()).await
        }
        Some(Command::Message(msg_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            thinclaw::cli::run_message_command(msg_cmd.clone()).await
        }
        Some(Command::Models(model_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
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
            execute_env_bootstrap_plan(env_bootstrap_plan);
            // In standalone CLI mode, create a fresh router.
            // Runtime agent routing state is in-memory only.
            let router = thinclaw::agent::AgentRouter::new();
            thinclaw::cli::run_agents_command(agent_cmd.clone(), &router).await;
            Ok(())
        }
        Some(Command::Sessions(session_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
            // In standalone CLI mode, create a fresh session manager.
            // Runtime session state is in-memory only.
            let mgr = std::sync::Arc::new(thinclaw::agent::SessionManager::new());
            thinclaw::cli::run_sessions_command(session_cmd.clone(), &mgr).await;
            Ok(())
        }
        Some(Command::Logs(log_cmd)) => {
            init_cli_tracing(cli.debug);
            execute_env_bootstrap_plan(env_bootstrap_plan);
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
            lease_id,
            gateway_url,
            token,
            workspace_root,
        }) => {
            init_cli_tracing(cli.debug);
            thinclaw::experiments::runner::run_remote_runner(
                gateway_url,
                *lease_id,
                token,
                workspace_root.clone(),
            )
            .await
        }
        Some(Command::Update(update_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_update_command(update_cmd.clone()).await
        }
        _ => unreachable!("runtime command must be handled by the caller"),
    };

    Some(result)
}
