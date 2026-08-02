//! Executable cross-surface slash-command contract.
//!
//! This module deliberately lives in `thinclaw-types`: both the root-independent
//! channel adapters and the agent parser consume it, so placing the vocabulary in
//! either higher-level crate would create a dependency cycle.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentSchema {
    Exact,
    OptionalText,
    RequiredText,
    ToolsQuery,
    JobCommand,
    ThreadId,
    CheckpointId,
}

impl ArgumentSchema {
    pub const fn accepts_arguments(self) -> bool {
        !matches!(self, Self::Exact)
    }
}

/// Commands rendered or acted upon by a client rather than the agent loop.
/// Surface handlers match this enum exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCommand {
    Help,
    Debug,
    Skin,
    Status,
    Tools,
    ClearConversation,
    ClearScreen,
    NewConversation,
    Quit,
    Back,
    Top,
    Bottom,
    Interrupt,
}

/// Agent-side system commands. `Input` means the original command line is
/// forwarded to the submission parser (for dedicated `Submission` variants,
/// thread/job routes, and compatibility aliases).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemCommandRoute {
    Input,
    Help,
    Status,
    Context,
    Model,
    Rollback,
    Rewind,
    Plan,
    Version,
    Tools,
    Debug,
    Ping,
    Identity,
    Personality,
    Skin,
    Memory,
    Skills,
}

impl SystemCommandRoute {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "",
            Self::Help => "help",
            Self::Status => "status",
            Self::Context => "context",
            Self::Model => "model",
            Self::Rollback => "rollback",
            Self::Rewind => "rewind",
            Self::Plan => "plan",
            Self::Version => "version",
            Self::Tools => "tools",
            Self::Debug => "debug",
            Self::Ping => "ping",
            Self::Identity => "identity",
            Self::Personality => "personality",
            Self::Skin => "skin",
            Self::Memory => "memory",
            Self::Skills => "skills",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "help" => Self::Help,
            "status" => Self::Status,
            "context" => Self::Context,
            "model" => Self::Model,
            "rollback" => Self::Rollback,
            "rewind" => Self::Rewind,
            "plan" => Self::Plan,
            "version" => Self::Version,
            "tools" => Self::Tools,
            "debug" => Self::Debug,
            "ping" => Self::Ping,
            "identity" => Self::Identity,
            "personality" | "vibe" => Self::Personality,
            "skin" => Self::Skin,
            "memory" => Self::Memory,
            "skills" => Self::Skills,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceRoute {
    Local(LocalCommand),
    Forward(SystemCommandRoute),
    Unsupported,
}

impl SurfaceRoute {
    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityPredicate {
    Always,
    LocalClient,
    RunningRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationRequirement {
    Anyone,
    Authenticated,
    Administrator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandVisibility {
    Common,
    Expert,
    Hidden,
    Removed,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub argument_schema: ArgumentSchema,
    pub help: &'static str,
    pub repl: SurfaceRoute,
    pub tui: SurfaceRoute,
    pub agent_message: SurfaceRoute,
    pub capability: CapabilityPredicate,
    pub minimum_authorization: AuthorizationRequirement,
    pub visibility: CommandVisibility,
}

impl CommandSpec {
    pub fn all_names(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name).chain(self.aliases.iter().copied())
    }

    pub fn matches_token(&self, token: &str) -> bool {
        self.all_names().any(|name| name == token)
    }

    pub const fn system_command(self) -> Option<SystemCommandRoute> {
        match self.agent_message {
            SurfaceRoute::Forward(route) if !matches!(route, SystemCommandRoute::Input) => {
                Some(route)
            }
            _ => None,
        }
    }
}

const F_INPUT: SurfaceRoute = SurfaceRoute::Forward(SystemCommandRoute::Input);
const UNSUPPORTED: SurfaceRoute = SurfaceRoute::Unsupported;

macro_rules! system {
    ($name:literal, $aliases:expr, $args:ident, $route:ident, $help:literal) => {
        CommandSpec {
            name: $name,
            aliases: $aliases,
            argument_schema: ArgumentSchema::$args,
            help: $help,
            repl: SurfaceRoute::Forward(SystemCommandRoute::$route),
            tui: SurfaceRoute::Forward(SystemCommandRoute::$route),
            agent_message: SurfaceRoute::Forward(SystemCommandRoute::$route),
            capability: CapabilityPredicate::RunningRuntime,
            minimum_authorization: AuthorizationRequirement::Authenticated,
            visibility: CommandVisibility::Common,
        }
    };
}

macro_rules! input {
    ($name:literal, $aliases:expr, $args:ident, $help:literal) => {
        CommandSpec {
            name: $name,
            aliases: $aliases,
            argument_schema: ArgumentSchema::$args,
            help: $help,
            repl: F_INPUT,
            tui: F_INPUT,
            agent_message: F_INPUT,
            capability: CapabilityPredicate::RunningRuntime,
            minimum_authorization: AuthorizationRequirement::Authenticated,
            visibility: CommandVisibility::Common,
        }
    };
}

pub const COMMAND_REGISTRY: &[CommandSpec] = &[
    CommandSpec {
        name: "/help",
        aliases: &["/?"],
        argument_schema: ArgumentSchema::Exact,
        help: "Show command help",
        repl: SurfaceRoute::Local(LocalCommand::Help),
        tui: SurfaceRoute::Local(LocalCommand::Help),
        agent_message: SurfaceRoute::Forward(SystemCommandRoute::Help),
        capability: CapabilityPredicate::Always,
        minimum_authorization: AuthorizationRequirement::Anyone,
        visibility: CommandVisibility::Common,
    },
    CommandSpec {
        name: "/status",
        aliases: &[],
        argument_schema: ArgumentSchema::Exact,
        help: "Show the latest complete runtime capability revision",
        repl: SurfaceRoute::Local(LocalCommand::Status),
        tui: SurfaceRoute::Local(LocalCommand::Status),
        agent_message: SurfaceRoute::Forward(SystemCommandRoute::Status),
        capability: CapabilityPredicate::Always,
        minimum_authorization: AuthorizationRequirement::Authenticated,
        visibility: CommandVisibility::Common,
    },
    system!(
        "/context",
        &[],
        Exact,
        Context,
        "List injected context sources"
    ),
    system!(
        "/model",
        &["/models"],
        OptionalText,
        Model,
        "Show or switch the active model"
    ),
    system!(
        "/rollback",
        &[],
        OptionalText,
        Rollback,
        "Inspect or restore filesystem checkpoints"
    ),
    system!(
        "/rewind",
        &[],
        OptionalText,
        Rewind,
        "Rewind conversation and files to an earlier turn"
    ),
    system!("/plan", &[], OptionalText, Plan, "Show or toggle plan mode"),
    system!("/version", &[], Exact, Version, "Show version information"),
    CommandSpec {
        name: "/tools",
        aliases: &[],
        argument_schema: ArgumentSchema::ToolsQuery,
        help: "List tools, inspect NAME, or include unavailable entries with --all",
        repl: SurfaceRoute::Local(LocalCommand::Tools),
        tui: SurfaceRoute::Local(LocalCommand::Tools),
        agent_message: SurfaceRoute::Forward(SystemCommandRoute::Tools),
        capability: CapabilityPredicate::Always,
        minimum_authorization: AuthorizationRequirement::Authenticated,
        visibility: CommandVisibility::Common,
    },
    CommandSpec {
        name: "/debug",
        aliases: &[],
        argument_schema: ArgumentSchema::Exact,
        help: "Toggle this client's diagnostic event detail",
        repl: SurfaceRoute::Local(LocalCommand::Debug),
        tui: SurfaceRoute::Local(LocalCommand::Debug),
        agent_message: UNSUPPORTED,
        capability: CapabilityPredicate::LocalClient,
        minimum_authorization: AuthorizationRequirement::Anyone,
        visibility: CommandVisibility::Expert,
    },
    system!("/ping", &[], Exact, Ping, "Check runtime connectivity"),
    input!("/undo", &[], Exact, "Undo the last turn"),
    input!("/redo", &[], Exact, "Redo the undone turn"),
    input!(
        "/compress",
        &["/compact"],
        Exact,
        "Compress the context window"
    ),
    CommandSpec {
        name: "/clear",
        aliases: &[],
        argument_schema: ArgumentSchema::Exact,
        help: "Clear the current durable conversation",
        repl: F_INPUT,
        tui: SurfaceRoute::Local(LocalCommand::ClearConversation),
        agent_message: F_INPUT,
        capability: CapabilityPredicate::RunningRuntime,
        minimum_authorization: AuthorizationRequirement::Authenticated,
        visibility: CommandVisibility::Common,
    },
    CommandSpec {
        name: "/interrupt",
        aliases: &["/stop"],
        argument_schema: ArgumentSchema::Exact,
        help: "Stop the current operation between tool iterations",
        repl: F_INPUT,
        tui: SurfaceRoute::Local(LocalCommand::Interrupt),
        agent_message: F_INPUT,
        capability: CapabilityPredicate::RunningRuntime,
        minimum_authorization: AuthorizationRequirement::Authenticated,
        visibility: CommandVisibility::Common,
    },
    CommandSpec {
        name: "/new",
        aliases: &["/reset"],
        argument_schema: ArgumentSchema::Exact,
        help: "Start a new durable conversation",
        repl: F_INPUT,
        tui: SurfaceRoute::Local(LocalCommand::NewConversation),
        agent_message: F_INPUT,
        capability: CapabilityPredicate::RunningRuntime,
        minimum_authorization: AuthorizationRequirement::Authenticated,
        visibility: CommandVisibility::Common,
    },
    input!(
        "/thread",
        &[],
        ThreadId,
        "Create or switch to a durable conversation"
    ),
    input!("/resume", &[], CheckpointId, "Resume from a checkpoint"),
    system!(
        "/identity",
        &[],
        Exact,
        Identity,
        "Show the active identity stack"
    ),
    system!(
        "/personality",
        &["/vibe"],
        OptionalText,
        Personality,
        "Show or set the session personality"
    ),
    CommandSpec {
        name: "/skin",
        aliases: &[],
        argument_schema: ArgumentSchema::OptionalText,
        help: "Show or switch this client's CLI skin",
        repl: SurfaceRoute::Local(LocalCommand::Skin),
        tui: SurfaceRoute::Local(LocalCommand::Skin),
        agent_message: UNSUPPORTED,
        capability: CapabilityPredicate::LocalClient,
        minimum_authorization: AuthorizationRequirement::Anyone,
        visibility: CommandVisibility::Common,
    },
    system!(
        "/memory",
        &[],
        Exact,
        Memory,
        "Summarize memory and continuity surfaces"
    ),
    input!("/heartbeat", &[], Exact, "Run the heartbeat check"),
    input!(
        "/summarize",
        &["/summary"],
        Exact,
        "Summarize the current conversation"
    ),
    input!("/suggest", &[], Exact, "Suggest next steps"),
    system!(
        "/skills",
        &[],
        OptionalText,
        Skills,
        "List installed skills or search the registry"
    ),
    input!("/restart", &[], Exact, "Restart the agent process"),
    CommandSpec {
        name: "/quit",
        aliases: &["/exit", "/shutdown"],
        argument_schema: ArgumentSchema::Exact,
        help: "Exit the current client",
        repl: SurfaceRoute::Local(LocalCommand::Quit),
        tui: SurfaceRoute::Local(LocalCommand::Quit),
        agent_message: F_INPUT,
        capability: CapabilityPredicate::LocalClient,
        minimum_authorization: AuthorizationRequirement::Anyone,
        visibility: CommandVisibility::Common,
    },
    input!(
        "/job",
        &[],
        JobCommand,
        "Manage jobs: create, list, status, cancel, or help"
    ),
    CommandSpec {
        name: "/create",
        aliases: &[],
        argument_schema: ArgumentSchema::RequiredText,
        help: "Legacy alias for /job create",
        repl: F_INPUT,
        tui: F_INPUT,
        agent_message: F_INPUT,
        capability: CapabilityPredicate::RunningRuntime,
        minimum_authorization: AuthorizationRequirement::Authenticated,
        visibility: CommandVisibility::Hidden,
    },
    CommandSpec {
        name: "/list",
        aliases: &["/jobs"],
        argument_schema: ArgumentSchema::OptionalText,
        help: "Legacy alias for /job list",
        repl: F_INPUT,
        tui: F_INPUT,
        agent_message: F_INPUT,
        capability: CapabilityPredicate::RunningRuntime,
        minimum_authorization: AuthorizationRequirement::Authenticated,
        visibility: CommandVisibility::Hidden,
    },
    CommandSpec {
        name: "/cancel",
        aliases: &[],
        argument_schema: ArgumentSchema::RequiredText,
        help: "Legacy alias for /job cancel",
        repl: F_INPUT,
        tui: F_INPUT,
        agent_message: F_INPUT,
        capability: CapabilityPredicate::RunningRuntime,
        minimum_authorization: AuthorizationRequirement::Authenticated,
        visibility: CommandVisibility::Hidden,
    },
    CommandSpec {
        name: "/cls",
        aliases: &[],
        argument_schema: ArgumentSchema::Exact,
        help: "Clear only the local terminal viewport",
        repl: SurfaceRoute::Local(LocalCommand::ClearScreen),
        tui: SurfaceRoute::Local(LocalCommand::ClearScreen),
        agent_message: UNSUPPORTED,
        capability: CapabilityPredicate::LocalClient,
        minimum_authorization: AuthorizationRequirement::Anyone,
        visibility: CommandVisibility::Expert,
    },
    CommandSpec {
        name: "/back",
        aliases: &["/close", "/dismiss"],
        argument_schema: ArgumentSchema::Exact,
        help: "Close the active detail view",
        repl: UNSUPPORTED,
        tui: SurfaceRoute::Local(LocalCommand::Back),
        agent_message: UNSUPPORTED,
        capability: CapabilityPredicate::LocalClient,
        minimum_authorization: AuthorizationRequirement::Anyone,
        visibility: CommandVisibility::Expert,
    },
    CommandSpec {
        name: "/top",
        aliases: &[],
        argument_schema: ArgumentSchema::Exact,
        help: "Jump to the oldest visible activity",
        repl: UNSUPPORTED,
        tui: SurfaceRoute::Local(LocalCommand::Top),
        agent_message: UNSUPPORTED,
        capability: CapabilityPredicate::LocalClient,
        minimum_authorization: AuthorizationRequirement::Anyone,
        visibility: CommandVisibility::Expert,
    },
    CommandSpec {
        name: "/bottom",
        aliases: &[],
        argument_schema: ArgumentSchema::Exact,
        help: "Jump to the latest activity",
        repl: UNSUPPORTED,
        tui: SurfaceRoute::Local(LocalCommand::Bottom),
        agent_message: UNSUPPORTED,
        capability: CapabilityPredicate::LocalClient,
        minimum_authorization: AuthorizationRequirement::Anyone,
        visibility: CommandVisibility::Expert,
    },
];

pub fn match_command(lower: &str) -> Option<&'static CommandSpec> {
    COMMAND_REGISTRY.iter().find(|spec| {
        spec.all_names().any(|name| {
            lower == name
                || (spec.argument_schema.accepts_arguments()
                    && lower
                        .strip_prefix(name)
                        .is_some_and(|rest| rest.starts_with(' ')))
        })
    })
}

/// Surface-aware match that preserves the two argument-sensitive legacy job
/// aliases without making bare `/help` or `/status` ambiguous.
pub fn match_surface_command(lower: &str) -> Option<&'static CommandSpec> {
    match_command(lower).or_else(|| {
        let legacy_job_alias = ["/status ", "/help "]
            .iter()
            .any(|prefix| lower.starts_with(prefix) && lower.len() > prefix.len());
        legacy_job_alias.then(|| {
            COMMAND_REGISTRY
                .iter()
                .find(|spec| spec.name == "/job")
                .expect("canonical /job registry entry")
        })
    })
}

pub fn route_for(surface: fn(&CommandSpec) -> SurfaceRoute, input: &str) -> SurfaceRoute {
    match_command(input.trim().to_ascii_lowercase().as_str())
        .map(surface)
        .unwrap_or(SurfaceRoute::Unsupported)
}

pub fn help_entries() -> impl Iterator<Item = &'static CommandSpec> {
    COMMAND_REGISTRY.iter().filter(|spec| {
        matches!(
            spec.visibility,
            CommandVisibility::Common | CommandVisibility::Expert
        )
    })
}

pub fn autocomplete_names(
    surface: fn(&CommandSpec) -> SurfaceRoute,
) -> impl Iterator<Item = &'static str> {
    COMMAND_REGISTRY
        .iter()
        .filter(move |spec| {
            !matches!(
                spec.visibility,
                CommandVisibility::Hidden | CommandVisibility::Removed
            ) && surface(spec).is_supported()
        })
        .flat_map(CommandSpec::all_names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn canonical_names_and_aliases_are_unique() {
        let mut seen = HashSet::new();
        for spec in COMMAND_REGISTRY {
            assert!(spec.name.starts_with('/'));
            assert!(seen.insert(spec.name), "duplicate command {}", spec.name);
            for alias in spec.aliases {
                assert!(alias.starts_with('/'));
                assert!(seen.insert(*alias), "duplicate alias {}", alias);
                assert_eq!(
                    match_command(alias).map(|found| found.name),
                    Some(spec.name)
                );
            }
        }
    }

    #[test]
    fn tools_accepts_typed_arguments_and_local_routes() {
        for input in ["/tools", "/tools shell", "/tools --all"] {
            let spec = match_command(input).expect("tools route");
            assert_eq!(spec.argument_schema, ArgumentSchema::ToolsQuery);
            assert_eq!(spec.repl, SurfaceRoute::Local(LocalCommand::Tools));
            assert_eq!(spec.tui, SurfaceRoute::Local(LocalCommand::Tools));
        }
    }

    #[test]
    fn removed_shell_and_reasoning_surfaces_are_absent() {
        assert!(match_command("/think").is_none());
        assert!(match_command("!echo nope").is_none());
        assert!(autocomplete_names(|spec| spec.tui).all(|name| name != "/think"));
    }

    #[test]
    fn local_only_commands_are_not_agent_message_routes() {
        for name in ["/debug", "/skin", "/cls", "/back", "/top", "/bottom"] {
            let spec = match_command(name).unwrap();
            assert_eq!(spec.agent_message, SurfaceRoute::Unsupported, "{name}");
        }
    }

    #[test]
    fn canonical_and_legacy_job_forms_are_unambiguous() {
        assert_eq!(match_command("/job status abc").unwrap().name, "/job");
        assert_eq!(match_command("/status").unwrap().name, "/status");
        assert!(match_command("/status abc").is_none());
        assert_eq!(match_surface_command("/status abc").unwrap().name, "/job");
        assert_eq!(match_command("/help").unwrap().name, "/help");
        assert!(match_command("/help abc").is_none());
        assert_eq!(match_surface_command("/help abc").unwrap().name, "/job");
        for legacy in ["/create x", "/list", "/jobs", "/cancel abc"] {
            assert!(matches!(
                match_command(legacy).unwrap().visibility,
                CommandVisibility::Hidden
            ));
        }
    }
}
