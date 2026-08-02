use std::collections::HashSet;

use clap::CommandFactory;
use thinclaw::cli::Cli;
use thinclaw_types::slash_commands::{
    ArgumentSchema, CommandVisibility, LocalCommand, SurfaceRoute, match_command,
};

#[test]
fn generated_root_and_slash_surfaces_exclude_removed_execution_paths() {
    let help = Cli::command().render_long_help().to_string();
    assert!(!help.contains("/think"));
    assert!(!help.contains("!<command>"));
    assert!(match_command("/think").is_none());
    assert!(match_command("!echo unsafe").is_none());
}

#[test]
fn every_visible_surface_entry_is_supported_and_aliases_share_one_route() {
    let mut names = HashSet::new();
    for spec in thinclaw_types::slash_commands::COMMAND_REGISTRY {
        assert!(names.insert(spec.name));
        if matches!(
            spec.visibility,
            CommandVisibility::Common | CommandVisibility::Expert
        ) {
            assert!(spec.repl.is_supported() || spec.tui.is_supported());
        }
        for alias in spec.aliases {
            assert!(names.insert(*alias), "duplicate command identity {alias}");
            let matched = match_command(alias).expect("alias route");
            assert_eq!(matched.name, spec.name);
            assert_eq!(matched.repl, spec.repl);
            assert_eq!(matched.tui, spec.tui);
            assert_eq!(matched.agent_message, spec.agent_message);
        }
    }
}

#[test]
fn local_admin_presentation_cannot_become_an_agent_message() {
    for (name, local) in [
        ("/debug", LocalCommand::Debug),
        ("/skin", LocalCommand::Skin),
        ("/cls", LocalCommand::ClearScreen),
        ("/back", LocalCommand::Back),
    ] {
        let spec = match_command(name).expect("registered local command");
        assert!(matches!(spec.tui, SurfaceRoute::Local(command) if command == local));
        assert_eq!(spec.agent_message, SurfaceRoute::Unsupported);
    }
}

#[test]
fn tools_and_job_grammars_remain_typed_and_unambiguous() {
    let tools = match_command("/tools --all").expect("tools route");
    assert_eq!(tools.argument_schema, ArgumentSchema::ToolsQuery);
    assert!(matches!(
        tools.repl,
        SurfaceRoute::Local(LocalCommand::Tools)
    ));
    assert_eq!(match_command("/status").unwrap().name, "/status");
    assert!(match_command("/status id").is_none());
    assert_eq!(
        thinclaw_types::slash_commands::match_surface_command("/status id")
            .unwrap()
            .name,
        "/job"
    );
    assert_eq!(match_command("/job status id").unwrap().name, "/job");
    assert!(matches!(
        match_command("/cancel id").unwrap().visibility,
        CommandVisibility::Hidden
    ));
}

#[test]
fn static_tool_catalog_is_exact_and_unique() {
    assert_eq!(thinclaw_tools::STATIC_TOOL_CATALOG.len(), 124);
    let unique = thinclaw_tools::STATIC_TOOL_CATALOG
        .iter()
        .map(|descriptor| descriptor.name)
        .collect::<HashSet<_>>();
    assert_eq!(unique.len(), 124);
}
