//! Declarative slash-command registry.
//!
//! The slash-command vocabulary used to be hand-maintained in three separate
//! places: [`crate::submission::SubmissionParser`]'s if-chain, the help
//! tables in [`crate::command_catalog`], and the TUI's forwarded/autocomplete
//! command lists (also in `command_catalog`). Those lists drifted from each
//! other over time (e.g. `/debug` missing from autocomplete, `/skin` missing
//! from forwarded commands).
//!
//! This module is the single source of truth: each [`CommandSpec`] carries
//! the canonical command name, its aliases, how its arguments are parsed,
//! which surfaces it should appear on, and its help text. Consumers derive
//! their lists from [`COMMAND_REGISTRY`] instead of hand-writing them.

/// How a command's arguments are recognized in free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgStyle {
    /// Only the bare command (and its aliases) match; no arguments are
    /// accepted. e.g. `/undo`.
    ExactOnly,
    /// The bare command matches, or the command followed by a space and
    /// arguments matches. A prefix match without a following space is
    /// intentionally rejected so `/modelling ideas` does not become a
    /// `/model` invocation.
    ExactOrSpaceDelimitedArgs,
}

/// A single slash command's cross-surface metadata.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    /// Canonical command name, always starting with `/`.
    pub name: &'static str,
    /// Additional tokens that resolve to the same command (also starting
    /// with `/`).
    pub aliases: &'static [&'static str],
    /// The `SystemCommand` name emitted by the submission parser, if this
    /// command maps to one. `None` for commands that map to a dedicated
    /// `Submission` variant instead (e.g. `/undo` -> `Submission::Undo`).
    pub system_command: Option<&'static str>,
    /// How arguments are parsed for this command.
    pub arg_style: ArgStyle,
    /// Whether this command is listed in the shared help text.
    pub in_help: bool,
    /// Whether the TUI forwards this command to the agent loop.
    pub tui_forwarded: bool,
    /// Whether the TUI autocompletes this command.
    pub tui_autocomplete: bool,
    /// Help text shown next to the command name. Empty when the command is
    /// not help-listed.
    pub help_text: &'static str,
}

impl CommandSpec {
    /// All names this spec matches on (canonical name plus aliases).
    pub fn all_names(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name).chain(self.aliases.iter().copied())
    }

    /// Whether `token` (already lowercased, no arguments) matches this
    /// command's canonical name or one of its aliases.
    pub fn matches_token(&self, token: &str) -> bool {
        self.all_names().any(|name| name == token)
    }

    /// Entries whose canonical name contains a `<placeholder>` exist for the
    /// help listing only and must never match real input.
    pub fn is_display_only(&self) -> bool {
        self.name.contains('<')
    }
}

/// The declarative slash-command table. This is the single source of truth
/// for command vocabulary shared across the submission parser, the router,
/// and the help/autocomplete/forwarded surfaces.
pub const COMMAND_REGISTRY: &[CommandSpec] = &[
    CommandSpec {
        name: "/help",
        aliases: &["/?"],
        system_command: Some("help"),
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: true,
        help_text: "Show this help",
    },
    CommandSpec {
        name: "/status",
        aliases: &[],
        system_command: Some("status"),
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: true,
        help_text: "Session status, context usage, model info",
    },
    CommandSpec {
        name: "/context",
        aliases: &[],
        system_command: Some("context"),
        // `/context` has bespoke sub-command parsing in the submission
        // parser (`/context` and `/context list` both mean "list", and
        // `/context detail` means "detail"; anything else falls through to
        // plain chat rather than being treated as command args). It is kept
        // `ExactOnly` here so the registry's generic
        // exact-or-space-delimited-args matcher does not swallow arbitrary
        // trailing tokens like `/context foo`.
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "List injected context sources",
    },
    CommandSpec {
        name: "/model",
        aliases: &["/models"],
        system_command: Some("model"),
        arg_style: ArgStyle::ExactOrSpaceDelimitedArgs,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Show or switch the active model",
    },
    CommandSpec {
        name: "/rollback",
        aliases: &[],
        system_command: Some("rollback"),
        arg_style: ArgStyle::ExactOrSpaceDelimitedArgs,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Filesystem rollback command family",
    },
    CommandSpec {
        name: "/version",
        aliases: &[],
        system_command: Some("version"),
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Show version info",
    },
    CommandSpec {
        name: "/tools",
        aliases: &[],
        system_command: Some("tools"),
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "List available tools",
    },
    CommandSpec {
        name: "/debug",
        aliases: &[],
        system_command: Some("debug"),
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: true,
        help_text: "Toggle debug mode",
    },
    CommandSpec {
        name: "/ping",
        aliases: &[],
        system_command: Some("ping"),
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Connectivity check",
    },
    CommandSpec {
        name: "/undo",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Undo last turn",
    },
    CommandSpec {
        name: "/redo",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Redo undone turn",
    },
    CommandSpec {
        name: "/compress",
        aliases: &["/compact"],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Compress the context window (`/compact` alias)",
    },
    CommandSpec {
        name: "/clear",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: true,
        help_text: "Clear current thread",
    },
    CommandSpec {
        name: "/interrupt",
        aliases: &["/stop"],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: true,
        help_text: "Stop current operation between tool iterations",
    },
    CommandSpec {
        name: "/new",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: true,
        help_text: "Start a new conversation thread",
    },
    CommandSpec {
        name: "/thread new",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: false,
        help_text: "Start a new conversation thread",
    },
    CommandSpec {
        name: "/thread <id>",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: false,
        help_text: "Switch to a thread",
    },
    CommandSpec {
        name: "/resume <id>",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: false,
        help_text: "Resume from a checkpoint",
    },
    CommandSpec {
        name: "/identity",
        aliases: &[],
        system_command: Some("identity"),
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Show the active agent name, base pack, skin, and session overlay",
    },
    CommandSpec {
        name: "/personality",
        aliases: &["/vibe"],
        system_command: Some("personality"),
        arg_style: ArgStyle::ExactOrSpaceDelimitedArgs,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Set, show, or clear a temporary session personality (`/vibe` alias)",
    },
    CommandSpec {
        name: "/skin",
        aliases: &[],
        system_command: Some("skin"),
        arg_style: ArgStyle::ExactOrSpaceDelimitedArgs,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Show or describe the configured CLI skin",
    },
    CommandSpec {
        name: "/memory",
        aliases: &[],
        system_command: Some("memory"),
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Summarize memory, recall, learning, and continuity surfaces",
    },
    CommandSpec {
        name: "/heartbeat",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Run the heartbeat check",
    },
    CommandSpec {
        name: "/summarize",
        aliases: &["/summary"],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Summarize the current thread",
    },
    CommandSpec {
        name: "/suggest",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Suggest next steps",
    },
    CommandSpec {
        name: "/skills",
        aliases: &[],
        system_command: Some("skills"),
        arg_style: ArgStyle::ExactOrSpaceDelimitedArgs,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "List installed skills or search the registry",
    },
    CommandSpec {
        name: "/restart",
        aliases: &[],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: true,
        tui_autocomplete: true,
        help_text: "Restart the agent process",
    },
    CommandSpec {
        name: "/quit",
        aliases: &["/exit", "/shutdown"],
        system_command: None,
        arg_style: ArgStyle::ExactOnly,
        in_help: true,
        tui_forwarded: false,
        tui_autocomplete: true,
        help_text: "Exit the current client",
    },
];

/// Match `lower` (the trimmed, lowercased full input) against a registry
/// entry's arg style, returning the matched spec if `lower` is either the
/// bare command/alias or `<command/alias> <args...>`.
///
/// This centralizes the "exact or space-delimited args" matching rule used
/// throughout the submission parser so each command site does not have to
/// hand-write `lower == "/x" || lower.starts_with("/x ")` chains.
pub fn match_command<'a>(lower: &str) -> Option<&'a CommandSpec> {
    COMMAND_REGISTRY.iter().find(|spec| {
        // Placeholder entries like "/thread <id>" exist for help display
        // only; matching them would send literal placeholder input (a user
        // copying the help line verbatim) into command dispatch that has no
        // handler for them.
        if spec.is_display_only() {
            return false;
        }
        spec.all_names().any(|name| match spec.arg_style {
            ArgStyle::ExactOnly => lower == name,
            ArgStyle::ExactOrSpaceDelimitedArgs => lower
                .strip_prefix(name)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' ')),
        })
    })
}

/// All registry entries flagged for the shared help listing, in table order.
pub fn help_entries() -> impl Iterator<Item = &'static CommandSpec> {
    COMMAND_REGISTRY.iter().filter(|spec| spec.in_help)
}

/// Canonical names (not aliases) of every command forwarded by the TUI.
pub fn forwarded_names() -> impl Iterator<Item = &'static str> {
    COMMAND_REGISTRY
        .iter()
        .filter(|spec| spec.tui_forwarded)
        .flat_map(|spec| spec.all_names())
}

/// Canonical names (not aliases) of every command the TUI autocompletes.
pub fn autocomplete_names() -> impl Iterator<Item = &'static str> {
    COMMAND_REGISTRY
        .iter()
        .filter(|spec| spec.tui_autocomplete)
        .flat_map(|spec| spec.all_names())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_name_starts_with_slash() {
        for spec in COMMAND_REGISTRY {
            assert!(
                spec.name.starts_with('/'),
                "command name {:?} must start with '/'",
                spec.name
            );
            for alias in spec.aliases {
                assert!(
                    alias.starts_with('/'),
                    "alias {:?} of {:?} must start with '/'",
                    alias,
                    spec.name
                );
            }
        }
    }

    #[test]
    fn match_command_respects_arg_style() {
        let spec = match_command("/model gpt-4o").expect("model matches");
        assert_eq!(spec.name, "/model");

        let spec = match_command("/models").expect("models alias matches");
        assert_eq!(spec.name, "/model");

        assert!(match_command("/modelling ideas").is_none());
        assert!(match_command("/undo extra").is_none());
        assert!(match_command("/undo").is_some());
    }

    #[test]
    fn matches_token_covers_canonical_and_alias() {
        let by_token = |token: &str| {
            COMMAND_REGISTRY
                .iter()
                .find(|spec| spec.matches_token(token))
        };
        assert_eq!(by_token("/compact").unwrap().name, "/compress");
        assert_eq!(by_token("/vibe").unwrap().name, "/personality");
        assert!(by_token("/nonexistent").is_none());
    }
}
