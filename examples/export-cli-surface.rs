use clap::CommandFactory;
use clap_complete::Shell;
use thinclaw::cli::Cli;
use thinclaw_types::slash_commands::{CommandSpec, SurfaceRoute};

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
    let value = serde_json::json!({
        "schema_version": 1,
        "root_help": help,
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
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("surface JSON")
    );
}
