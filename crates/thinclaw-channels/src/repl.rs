//! Root-independent REPL input policy.

/// Max characters for tool result previews in the terminal.
pub const CLI_TOOL_RESULT_MAX: usize = 200;

/// Max characters for thinking/status messages in the terminal.
pub const CLI_STATUS_MAX: usize = 200;

use thinclaw_types::slash_commands::{
    LocalCommand, SurfaceRoute, autocomplete_names, match_surface_command,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplInputAction {
    Ignore,
    Submit(String),
    Local {
        command: LocalCommand,
        argument: String,
    },
    Unsupported(String),
}

pub fn classify_repl_line(line: &str) -> ReplInputAction {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ReplInputAction::Ignore;
    }

    if trimmed.eq_ignore_ascii_case("/think") {
        return ReplInputAction::Unsupported(
            "`/think` was removed because it did not control a real reasoning view.".to_string(),
        );
    }

    let lower = trimmed.to_ascii_lowercase();
    if let Some(spec) = match_surface_command(&lower) {
        return match spec.repl {
            SurfaceRoute::Local(command) => {
                let argument = trimmed
                    .split_once(char::is_whitespace)
                    .map(|(_, value)| value.trim().to_string())
                    .unwrap_or_default();
                ReplInputAction::Local { command, argument }
            }
            SurfaceRoute::Forward(_) => ReplInputAction::Submit(trimmed.to_string()),
            SurfaceRoute::Unsupported => {
                ReplInputAction::Unsupported(format!("{} is not available in the REPL", spec.name))
            }
        };
    }
    ReplInputAction::Submit(trimmed.to_string())
}

pub fn slash_command_matches(prefix: &str) -> Vec<String> {
    autocomplete_names(|spec| spec.repl)
        .filter(|cmd| cmd.starts_with(prefix))
        .map(|cmd| cmd.to_string())
        .collect()
}

pub fn slash_command_hint(line: &str, pos: usize) -> Option<String> {
    if !line.starts_with('/') || pos < line.len() {
        return None;
    }

    autocomplete_names(|spec| spec.repl)
        .find(|cmd| cmd.starts_with(line) && *cmd != line)
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
        assert_eq!(
            classify_repl_line("/skin list"),
            ReplInputAction::Local {
                command: LocalCommand::Skin,
                argument: "list".to_string(),
            }
        );
        assert!(matches!(
            classify_repl_line("/quit"),
            ReplInputAction::Local {
                command: LocalCommand::Quit,
                ..
            }
        ));
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
