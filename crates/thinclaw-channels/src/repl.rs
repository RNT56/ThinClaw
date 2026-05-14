//! Root-independent REPL input policy.

/// Max characters for tool result previews in the terminal.
pub const CLI_TOOL_RESULT_MAX: usize = 200;

/// Max characters for thinking/status messages in the terminal.
pub const CLI_STATUS_MAX: usize = 200;

/// Slash commands available in the REPL.
pub const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/quit",
    "/exit",
    "/debug",
    "/model",
    "/undo",
    "/redo",
    "/clear",
    "/compress",
    "/compact",
    "/new",
    "/interrupt",
    "/version",
    "/tools",
    "/ping",
    "/context",
    "/job",
    "/status",
    "/cancel",
    "/list",
    "/identity",
    "/memory",
    "/skills",
    "/heartbeat",
    "/summarize",
    "/suggest",
    "/thread",
    "/resume",
    "/rollback",
    "/personality",
    "/vibe",
    "/skin",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplInputAction {
    Ignore,
    Submit(String),
    Quit,
    Help,
    ToggleDebug,
    Skin(String),
}

pub fn classify_repl_line(line: &str) -> ReplInputAction {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ReplInputAction::Ignore;
    }

    match trimmed.to_lowercase().as_str() {
        "/quit" | "/exit" => ReplInputAction::Quit,
        "/help" => ReplInputAction::Help,
        "/debug" => ReplInputAction::ToggleDebug,
        _ if trimmed.starts_with("/skin") => {
            let arg = trimmed.strip_prefix("/skin").map(str::trim).unwrap_or("");
            ReplInputAction::Skin(arg.to_string())
        }
        _ => ReplInputAction::Submit(trimmed.to_string()),
    }
}

pub fn slash_command_matches(prefix: &str) -> Vec<String> {
    SLASH_COMMANDS
        .iter()
        .filter(|cmd| cmd.starts_with(prefix))
        .map(|cmd| cmd.to_string())
        .collect()
}

pub fn slash_command_hint(line: &str, pos: usize) -> Option<String> {
    if !line.starts_with('/') || pos < line.len() {
        return None;
    }

    SLASH_COMMANDS
        .iter()
        .find(|cmd| cmd.starts_with(line) && **cmd != line)
        .map(|cmd| cmd[line.len()..].to_string())
}

pub fn repl_input_is_incomplete(input: &str) -> bool {
    input.ends_with('\\') || !input.matches("```").count().is_multiple_of(2)
}

/// Collapse output into a single-line preview for terminal status display.
pub fn truncate_for_terminal_preview(output: &str, max_chars: usize) -> String {
    let collapsed: String = output
        .chars()
        .take(max_chars + 50)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() > max_chars {
        let byte_offset = collapsed
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(collapsed.len());
        format!("{}...", &collapsed[..byte_offset])
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_local_commands_and_submissions() {
        assert_eq!(classify_repl_line("   "), ReplInputAction::Ignore);
        assert_eq!(classify_repl_line("/quit"), ReplInputAction::Quit);
        assert_eq!(classify_repl_line("/exit"), ReplInputAction::Quit);
        assert_eq!(classify_repl_line("/help"), ReplInputAction::Help);
        assert_eq!(classify_repl_line("/debug"), ReplInputAction::ToggleDebug);
        assert_eq!(
            classify_repl_line("/skin list"),
            ReplInputAction::Skin("list".to_string())
        );
        assert_eq!(
            classify_repl_line("hello"),
            ReplInputAction::Submit("hello".to_string())
        );
    }

    #[test]
    fn completes_and_hints_slash_commands() {
        assert!(slash_command_matches("/he").contains(&"/help".to_string()));
        assert_eq!(slash_command_hint("/he", 3), Some("lp".to_string()));
        assert_eq!(slash_command_hint("hello", 5), None);
    }

    #[test]
    fn detects_multiline_input() {
        assert!(repl_input_is_incomplete("continued\\"));
        assert!(repl_input_is_incomplete("```rust\nlet x = 1;"));
        assert!(!repl_input_is_incomplete("```rust\nlet x = 1;\n```"));
    }

    #[test]
    fn terminal_preview_collapses_and_truncates() {
        assert_eq!(truncate_for_terminal_preview("hello", 10), "hello");
        assert_eq!(
            truncate_for_terminal_preview("line1\n   line2", 20),
            "line1 line2"
        );
        assert_eq!(truncate_for_terminal_preview("abcdef", 3), "abc...");
        assert_eq!(truncate_for_terminal_preview("éééé", 2), "éé...");
    }
}
