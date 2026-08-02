use clap::CommandFactory;
use clap_complete::Shell;
use thinclaw::cli::Cli;
use thinclaw_types::slash_commands::{CommandSpec, SurfaceRoute};

fn canonical_leaves(command: &clap::Command, prefix: &[String], output: &mut Vec<String>) {
    for child in command
        .get_subcommands()
        .filter(|child| !child.is_hide_set())
    {
        let mut path = prefix.to_vec();
        path.push(child.get_name().to_string());
        if child
            .get_subcommands()
            .any(|grandchild| !grandchild.is_hide_set())
        {
            canonical_leaves(child, &path, output);
        } else {
            output.push(path.join(" "));
        }
    }
}

fn route_name(route: SurfaceRoute) -> String {
    match route {
        SurfaceRoute::Local(command) => format!("local:{command:?}"),
        SurfaceRoute::Forward(command) => format!("forward:{command:?}"),
        SurfaceRoute::Unsupported => "unsupported".to_string(),
    }
}

fn slash_entry(spec: &CommandSpec) -> serde_json::Value {
    serde_json::json!({
        "name": spec.name,
        "aliases": spec.aliases,
        "argument_schema": format!("{:?}", spec.argument_schema),
        "help": spec.help,
        "repl": route_name(spec.repl),
        "tui": route_name(spec.tui),
        "agent_message": route_name(spec.agent_message),
        "capability": format!("{:?}", spec.capability),
        "minimum_authorization": format!("{:?}", spec.minimum_authorization),
        "visibility": format!("{:?}", spec.visibility),
    })
}

fn completion(shell: Shell) -> String {
    let mut command = Cli::command();
    let mut output = Vec::new();
    clap_complete::generate(shell, &mut command, "thinclaw", &mut output);
    String::from_utf8(output).expect("completion output is UTF-8")
}

fn main() {
    let mut command = Cli::command();
    let help = command.render_long_help().to_string();
    let mut leaves = Vec::new();
    canonical_leaves(&Cli::command(), &[], &mut leaves);
    leaves.sort();
    let canonical_leaves = leaves
        .into_iter()
        .map(|path| {
            let effect = thinclaw_types::canonical_cli_leaf_effect(&path)
                .unwrap_or_else(|error| panic!("{error}"));
            serde_json::json!({"path": path, "effect": effect})
        })
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "schema_version": 1,
        "root_help": help,
        "canonical_leaves": canonical_leaves,
        "completions": {
            "bash": completion(Shell::Bash),
            "elvish": completion(Shell::Elvish),
            "fish": completion(Shell::Fish),
            "powershell": completion(Shell::PowerShell),
            "zsh": completion(Shell::Zsh),
        },
        "slash_commands": thinclaw_types::slash_commands::COMMAND_REGISTRY
            .iter()
            .map(slash_entry)
            .collect::<Vec<_>>(),
        "static_tools": thinclaw_tools::STATIC_TOOL_CATALOG
            .iter()
            .map(|tool| serde_json::json!({
                "name": tool.name,
                "origin": tool.origin.to_string(),
            }))
            .collect::<Vec<_>>(),
        "dynamic_tool_origins": thinclaw_tools::registry::ALL_TOOL_ORIGINS
            .iter()
            .filter(|origin| matches!(
                origin,
                thinclaw_tools::ToolOrigin::Wasm
                    | thinclaw_tools::ToolOrigin::Mcp
                    | thinclaw_tools::ToolOrigin::UserTool
                    | thinclaw_tools::ToolOrigin::NativePlugin
            ))
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "channels": thinclaw::channels::catalog::static_channel_catalog(),
        "setup_phases": thinclaw_app::ALL_SETUP_WIZARD_PHASE_IDS
            .iter()
            .map(|phase| serde_json::json!({
                "id": format!("{phase:?}"),
                "title": phase.title(),
            }))
            .collect::<Vec<_>>(),
        "setup_steps": thinclaw_app::ALL_SETUP_WIZARD_STEP_IDS
            .iter()
            .map(|step| serde_json::json!({
                "id": format!("{step:?}"),
                "target_phase": format!("{:?}", step.target_phase()),
                "executable": step.executable(),
            }))
            .collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("surface JSON")
    );
}
