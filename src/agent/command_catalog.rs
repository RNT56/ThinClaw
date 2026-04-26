//! Shared cross-surface command vocabulary.

#[derive(Clone, Copy)]
struct CommandEntry {
    command: &'static str,
    description: &'static str,
}

const SYSTEM_COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        command: "/help",
        description: "Show this help",
    },
    CommandEntry {
        command: "/status",
        description: "Session status, context usage, model info",
    },
    CommandEntry {
        command: "/context",
        description: "List injected context sources",
    },
    CommandEntry {
        command: "/context detail",
        description: "Show full injected context",
    },
    CommandEntry {
        command: "/model [name]",
        description: "Show or switch the active model",
    },
    CommandEntry {
        command: "/rollback ...",
        description: "Filesystem rollback command family",
    },
    CommandEntry {
        command: "/version",
        description: "Show version info",
    },
    CommandEntry {
        command: "/tools",
        description: "List available tools",
    },
    CommandEntry {
        command: "/debug",
        description: "Toggle debug mode",
    },
    CommandEntry {
        command: "/ping",
        description: "Connectivity check",
    },
];

const SESSION_COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        command: "/undo",
        description: "Undo last turn",
    },
    CommandEntry {
        command: "/redo",
        description: "Redo undone turn",
    },
    CommandEntry {
        command: "/compress",
        description: "Compress the context window (`/compact` alias)",
    },
    CommandEntry {
        command: "/clear",
        description: "Clear current thread",
    },
    CommandEntry {
        command: "/interrupt",
        description: "Stop current operation between tool iterations",
    },
    CommandEntry {
        command: "/new",
        description: "Start a new conversation thread",
    },
    CommandEntry {
        command: "/thread new",
        description: "Start a new conversation thread",
    },
    CommandEntry {
        command: "/thread <id>",
        description: "Switch to a thread",
    },
    CommandEntry {
        command: "/resume <id>",
        description: "Resume from a checkpoint",
    },
];

const IDENTITY_COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        command: "/identity",
        description: "Show the active agent name, base pack, skin, and session overlay",
    },
    CommandEntry {
        command: "/personality [name]",
        description: "Set, show, or clear a temporary session personality (`/vibe` alias)",
    },
    CommandEntry {
        command: "/skin [name]",
        description: "Show or describe the configured CLI skin",
    },
];

const MEMORY_COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        command: "/memory",
        description: "Summarize memory, recall, learning, and continuity surfaces",
    },
    CommandEntry {
        command: "/heartbeat",
        description: "Run the heartbeat check",
    },
    CommandEntry {
        command: "/summarize",
        description: "Summarize the current thread",
    },
    CommandEntry {
        command: "/suggest",
        description: "Suggest next steps",
    },
];

const SKILL_COMMANDS: &[CommandEntry] = &[CommandEntry {
    command: "/skills",
    description: "List installed skills or search the registry",
}];

const AGENT_COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        command: "/restart",
        description: "Restart the agent process",
    },
    CommandEntry {
        command: "/quit",
        description: "Exit the current client",
    },
];

fn render_section(title: &str, commands: &[CommandEntry]) -> String {
    let mut lines = vec![format!("{title}:")];
    for command in commands {
        lines.push(format!("  {:<22} {}", command.command, command.description));
    }
    lines.join("\n")
}

pub fn agent_help_text() -> String {
    [
        render_section("System", SYSTEM_COMMANDS),
        render_section("Session", SESSION_COMMANDS),
        render_section("Identity & Personality", IDENTITY_COMMANDS),
        render_section("Memory & Growth", MEMORY_COMMANDS),
        render_section("Skills", SKILL_COMMANDS),
        render_section("Agent", AGENT_COMMANDS),
    ]
    .join("\n\n")
}

pub fn tui_help_text() -> String {
    format!(
        "━━━ Agent cockpit controls ━━━\n\n\
{}\n\n\
{}\n\n\
{}\n\n\
{}\n\n\
Local TUI:\n\
  /back, /close          Close the most recent detail card\n\
  /top, /bottom          Jump to oldest/newest activity\n\
  /cls                   Clear the visible log\n\
  /think                 Toggle thinking updates\n\
  /exit, /quit           Leave the TUI\n\
  !<command>             Run a local shell command\n\n\
━━━ Movement ━━━\n\n\
  Enter                  Send a message\n\
  Ctrl+C                 Abort active run, press twice to exit\n\
  Ctrl+L                 Clear the screen\n\
  Up/Down                Browse input history\n\
  PageUp/Down            Scroll the conversation\n\
  Tab                    Autocomplete commands\n\
  Home/End               Jump to start/end of input",
        render_section("Shared system", SYSTEM_COMMANDS),
        render_section("Shared session", SESSION_COMMANDS),
        render_section("Shared memory & growth", MEMORY_COMMANDS),
        render_section(
            "Shared identity, skills, and agent",
            &[IDENTITY_COMMANDS, SKILL_COMMANDS, AGENT_COMMANDS].concat()
        ),
    )
}

pub fn tui_forwarded_commands() -> &'static [&'static str] {
    &[
        "/undo",
        "/redo",
        "/job",
        "/cancel",
        "/list",
        "/compress",
        "/compact",
        "/model",
        "/models",
        "/version",
        "/tools",
        "/context",
        "/ping",
        "/thread",
        "/resume",
        "/restart",
        "/rollback",
        "/identity",
        "/memory",
        "/skills",
        "/heartbeat",
        "/summarize",
        "/suggest",
        "/personality",
        "/vibe",
    ]
}

pub fn tui_autocomplete_commands() -> &'static [&'static str] {
    &[
        "/help",
        "/back",
        "/close",
        "/dismiss",
        "/top",
        "/bottom",
        "/clear",
        "/new",
        "/reset",
        "/exit",
        "/quit",
        "/think",
        "/status",
        "/interrupt",
        "/undo",
        "/redo",
        "/compress",
        "/compact",
        "/context",
        "/model",
        "/models",
        "/version",
        "/tools",
        "/thread",
        "/resume",
        "/restart",
        "/ping",
        "/job",
        "/cancel",
        "/list",
        "/rollback",
        "/identity",
        "/memory",
        "/skills",
        "/heartbeat",
        "/summarize",
        "/suggest",
        "/personality",
        "/vibe",
        "/skin",
        "/cls",
    ]
}
