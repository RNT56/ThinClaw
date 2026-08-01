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
            | Command::Setup(thinclaw::cli::SetupCommand {
                action: None | Some(thinclaw::cli::SetupAction::Edit { .. }),
                ..
            })
            | Command::Onboard { .. }
            | Command::AutonomyShadowCanary { .. },
        ) => return Ok(thinclaw::cli::CliDispatch::Runtime),
        _ => {}
    }

    debug_assert_eq!(context.debug(), cli.debug);

    if let Some(Command::Setup(thinclaw::cli::SetupCommand {
        action: Some(thinclaw::cli::SetupAction::Reset(command)),
        ..
    })) = &cli.command
    {
        init_cli_tracing(cli.debug);
        run_reset_command(command.clone())
            .await
            .map_err(thinclaw::cli::CliError::from)?;
        return Ok(thinclaw::cli::CliDispatch::Handled(
            thinclaw::cli::CliOutcome::Success,
        ));
    }

    if let Some(Command::Doctor { profile }) = &cli.command {
        init_cli_tracing(cli.debug);
        thinclaw::cli::run_doctor_command((*profile).into(), context).await?;
        return Ok(thinclaw::cli::CliDispatch::Handled(
            thinclaw::cli::CliOutcome::Success,
        ));
    }

    if let Some(Command::Status { profile }) = &cli.command {
        init_cli_tracing(cli.debug);
        let outcome = thinclaw::cli::run_status_command((*profile).into(), context).await?;
        return Ok(thinclaw::cli::CliDispatch::Handled(outcome));
    }

    let result = match &cli.command {
        Some(Command::Tool(tool_cmd)) => {
            init_cli_tracing(cli.debug);
            run_tool_command(tool_cmd.clone()).await
        }
        Some(Command::Config(config_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_config_command(config_cmd.clone(), context).await
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
        Some(Command::Doctor { .. }) => unreachable!("doctor handled before generic dispatch"),
        Some(Command::Status { .. }) => unreachable!("status handled before generic dispatch"),
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
            thinclaw::cli::run_cron_command(cron_cmd.clone(), context).await
        }
        Some(Command::Automation(automation_cmd)) => {
            init_cli_tracing(cli.debug);
            match automation_cmd {
                thinclaw::cli::AutomationCommand::Routines(command) => {
                    thinclaw::cli::run_cron_command(command.clone(), context).await
                }
                thinclaw::cli::AutomationCommand::Jobs(command) => {
                    thinclaw::cli::run_jobs_command(command.clone(), context)
                        .await
                        .map_err(anyhow::Error::from)
                }
                thinclaw::cli::AutomationCommand::Projects(command) => {
                    thinclaw::cli::run_repo_projects_command(command.clone()).await
                }
            }
        }
        Some(Command::Runtime(runtime_cmd)) => {
            init_cli_tracing(cli.debug);
            match runtime_cmd {
                thinclaw::cli::RuntimeCommand::Web(command) => {
                    run_gateway_command(command.clone(), context).await
                }
                #[cfg(feature = "repl")]
                thinclaw::cli::RuntimeCommand::Service(command) => {
                    thinclaw::cli::run_service_command(command)
                }
                thinclaw::cli::RuntimeCommand::Logs(command) => {
                    thinclaw::cli::run_log_command(command.clone()).await
                }
                thinclaw::cli::RuntimeCommand::Update(command) => {
                    thinclaw::cli::run_update_command(command.clone()).await
                }
            }
        }
        Some(Command::Extensions(extensions_cmd)) => {
            init_cli_tracing(cli.debug);
            match extensions_cmd {
                thinclaw::cli::ExtensionsCommand::Channels(command) => {
                    run_channels_command(command.clone()).await
                }
                thinclaw::cli::ExtensionsCommand::Tools(command) => {
                    run_tool_command(command.clone()).await
                }
                thinclaw::cli::ExtensionsCommand::Registry(command) => {
                    thinclaw::cli::run_registry_command(command.clone()).await
                }
                thinclaw::cli::ExtensionsCommand::Mcp(command) => {
                    run_mcp_command(command.clone()).await
                }
            }
        }
        Some(Command::Data(data_cmd)) => {
            init_cli_tracing(cli.debug);
            match data_cmd {
                thinclaw::cli::DataCommand::Memory(command) => run_memory_command(command).await,
                thinclaw::cli::DataCommand::Conversations(command) => {
                    thinclaw::cli::run_sessions_command(command.clone(), context)
                        .await
                        .map_err(anyhow::Error::from)
                }
                thinclaw::cli::DataCommand::Backup(command) => {
                    thinclaw::cli::run_backup_command(command.clone()).await
                }
                thinclaw::cli::DataCommand::Trajectories(command) => {
                    run_trajectory_command(command.clone()).await
                }
            }
        }
        Some(Command::Access(access_cmd)) => {
            init_cli_tracing(cli.debug);
            match access_cmd {
                thinclaw::cli::AccessCommand::Identities(command) => {
                    run_identity_command(command.clone()).await
                }
                thinclaw::cli::AccessCommand::Senders(command) => {
                    run_pairing_command(command.clone())
                        .await
                        .map_err(|error| anyhow::anyhow!("{error}"))
                }
                thinclaw::cli::AccessCommand::Devices(command) => {
                    thinclaw::cli::run_devices_command(command.clone()).await
                }
            }
        }
        Some(Command::Labs(labs_cmd)) => {
            init_cli_tracing(cli.debug);
            match labs_cmd {
                thinclaw::cli::LabsCommand::Experiments(command) => {
                    thinclaw::cli::run_experiments_command(command.clone()).await
                }
            }
        }
        Some(Command::Media(media_cmd)) => {
            init_cli_tracing(cli.debug);
            match media_cmd {
                thinclaw::cli::MediaCommand::Comfy(command) => {
                    thinclaw::cli::run_comfy_command(command.clone()).await
                }
            }
        }
        Some(Command::Dev(dev_cmd)) => {
            init_cli_tracing(cli.debug);
            match dev_cmd {
                thinclaw::cli::DevCommand::Browser(command) => {
                    thinclaw::cli::run_browser_command(command.clone()).await
                }
            }
        }
        Some(Command::Experiments(experiments_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_experiments_command(experiments_cmd.clone()).await
        }
        Some(Command::Gateway(gw_cmd)) => {
            init_cli_tracing(cli.debug);
            run_gateway_command(gw_cmd.clone(), context).await
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
            thinclaw::cli::run_agents_command(agent_cmd.clone(), context)
                .await
                .map_err(anyhow::Error::from)
        }
        Some(Command::Sessions(session_cmd)) => {
            init_cli_tracing(cli.debug);
            thinclaw::cli::run_sessions_command(session_cmd.clone(), context)
                .await
                .map_err(anyhow::Error::from)
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
