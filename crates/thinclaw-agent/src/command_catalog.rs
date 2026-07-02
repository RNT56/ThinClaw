//! Shared cross-surface command vocabulary.
//!
//! The help table and the TUI's forwarded/autocomplete command lists are
//! generated from the single [`crate::command_registry::COMMAND_REGISTRY`]
//! table, so the three surfaces cannot drift from each other.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::command_registry::{self, CommandSpec};

/// Help-table sections, identified by which registry entries belong in them.
/// Order matches the historical section layout.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HelpSection {
    System,
    Session,
    Identity,
    Memory,
    Skills,
    Agent,
}

fn help_section_for(name: &str) -> HelpSection {
    match name {
        "/help" | "/status" | "/context" | "/model" | "/rollback" | "/version" | "/tools"
        | "/debug" | "/ping" => HelpSection::System,
        "/undo" | "/redo" | "/compress" | "/clear" | "/interrupt" | "/new" | "/thread new"
        | "/thread <id>" | "/resume <id>" => HelpSection::Session,
        "/identity" | "/personality" | "/skin" => HelpSection::Identity,
        "/memory" | "/heartbeat" | "/summarize" | "/suggest" => HelpSection::Memory,
        "/skills" => HelpSection::Skills,
        "/restart" | "/quit" => HelpSection::Agent,
        // A new registry entry without a section assignment lands in the
        // System table instead of panicking help rendering at runtime.
        _ => HelpSection::System,
    }
}

/// Display label for a help-listed command. A handful of commands show
/// extra usage hints (`[name]`, `...`, `<id>`) that aren't part of the
/// matchable command name itself.
fn help_label(spec: &CommandSpec) -> &'static str {
    match spec.name {
        "/context" => "/context",
        "/model" => "/model [name]",
        "/rollback" => "/rollback ...",
        "/personality" => "/personality [name]",
        "/skin" => "/skin [name]",
        other => other,
    }
}

fn help_entries_for(section: HelpSection) -> Vec<(&'static str, &'static str)> {
    let mut out: Vec<(&'static str, &'static str)> = command_registry::help_entries()
        .filter(|spec| help_section_for(spec.name) == section)
        .map(|spec| (help_label(spec), spec.help_text))
        .collect();

    // `/context detail` is a usage variant of `/context`, not its own
    // registry entry; keep it directly under `/context` in the System
    // section like the original hand-written table did.
    if section == HelpSection::System
        && let Some(pos) = out.iter().position(|(label, _)| *label == "/context")
    {
        out.insert(pos + 1, ("/context detail", "Show full injected context"));
    }

    out
}

fn render_section(title: &str, entries: &[(&'static str, &'static str)]) -> String {
    let mut lines = vec![format!("{title}:")];
    for (command, description) in entries {
        lines.push(format!("  {:<22} {}", command, description));
    }
    lines.join("\n")
}

pub fn agent_help_text() -> String {
    [
        render_section("System", &help_entries_for(HelpSection::System)),
        render_section("Session", &help_entries_for(HelpSection::Session)),
        render_section(
            "Identity & Personality",
            &help_entries_for(HelpSection::Identity),
        ),
        render_section("Memory & Growth", &help_entries_for(HelpSection::Memory)),
        render_section("Skills", &help_entries_for(HelpSection::Skills)),
        render_section("Agent", &help_entries_for(HelpSection::Agent)),
    ]
    .join("\n\n")
}

/// Format a count with a suffix, using K/M abbreviations for large numbers.
pub fn format_count(n: u64, suffix: &str) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M {}", n as f64 / 1_000_000.0, suffix)
    } else if n >= 1_000 {
        format!("{:.1}K {}", n as f64 / 1_000.0, suffix)
    } else {
        format!("{} {}", n, suffix)
    }
}

pub fn format_checkpoint_age(timestamp: DateTime<Utc>) -> String {
    let age = Utc::now().signed_duration_since(timestamp);
    if age.num_seconds() < 60 {
        format!("{}s ago", age.num_seconds().max(0))
    } else if age.num_minutes() < 60 {
        format!("{}m ago", age.num_minutes())
    } else if age.num_hours() < 24 {
        format!("{}h ago", age.num_hours())
    } else {
        format!("{}d ago", age.num_days())
    }
}

pub fn rollback_usage() -> &'static str {
    "Usage:\n  /rollback list\n  /rollback diff <N>\n  /rollback <N> [file]"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackCheckpointView {
    pub commit_hash: String,
    pub timestamp: DateTime<Utc>,
    pub summary: String,
}

pub fn rollback_active_project_text(project_root: &std::path::Path) -> String {
    format!(
        "{}\n\nActive project: {}",
        rollback_usage(),
        project_root.display()
    )
}

pub fn rollback_no_checkpoints_text(project_root: &std::path::Path) -> String {
    format!(
        "No filesystem checkpoints found for {}.",
        project_root.display()
    )
}

pub fn rollback_checkpoint_list_text(
    project_root: &std::path::Path,
    entries: &[RollbackCheckpointView],
) -> String {
    let mut out = format!("Filesystem checkpoints for {}:\n", project_root.display());
    for (idx, entry) in entries.iter().enumerate() {
        out.push_str(&format!(
            "  {}. {}  {}  {}\n",
            idx + 1,
            short_commit_hash(&entry.commit_hash),
            format_checkpoint_age(entry.timestamp),
            entry.summary
        ));
    }
    out
}

pub fn rollback_diff_usage_error_text() -> String {
    format!(
        "{}\n\n`/rollback diff <N>` does not take a file path.",
        rollback_usage()
    )
}

pub fn rollback_positive_index_error_text() -> &'static str {
    "Rollback index must be a positive integer."
}

pub fn rollback_checkpoint_not_found_text(index: usize) -> String {
    format!(
        "Checkpoint {} not found. Run `/rollback list` to inspect available checkpoints.",
        index
    )
}

pub fn rollback_empty_diff_text(index: usize) -> String {
    format!(
        "No differences between checkpoint {} and the current project state.",
        index
    )
}

pub fn rollback_diff_text(index: usize, commit_hash: &str, diff: &str) -> String {
    format!(
        "Diff for checkpoint {} ({})\n\n{}",
        index,
        short_commit_hash(commit_hash),
        diff.trim_end()
    )
}

pub fn rollback_restored_text(index: usize, commit_hash: &str, file: Option<&str>) -> String {
    match file {
        Some(file) => format!(
            "Restored {} from checkpoint {} ({})",
            file,
            index,
            short_commit_hash(commit_hash)
        ),
        None => format!(
            "Restored project state from checkpoint {} ({})",
            index,
            short_commit_hash(commit_hash)
        ),
    }
}

fn short_commit_hash(commit_hash: &str) -> &str {
    &commit_hash[..commit_hash.len().min(12)]
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobSummaryView {
    pub total: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub stuck: usize,
}

pub fn created_job_text(title: &str, job_id: Uuid) -> String {
    format!(
        "Created job: {}\nID: {}\n\nThe job has been scheduled and is now running.",
        title, job_id
    )
}

pub fn job_status_text(
    title: &str,
    state: impl std::fmt::Debug,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    actual_cost: Decimal,
) -> String {
    format!(
        "Job: {}\nStatus: {:?}\nCreated: {}\nStarted: {}\nActual cost: {}",
        title,
        state,
        created_at.format("%Y-%m-%d %H:%M:%S"),
        started_at
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Not started".to_string()),
        actual_cost
    )
}

pub fn jobs_summary_text(summary: JobSummaryView) -> String {
    format!(
        "Jobs summary:\n  Total: {}\n  In Progress: {}\n  Completed: {}\n  Failed: {}\n  Stuck: {}",
        summary.total, summary.in_progress, summary.completed, summary.failed, summary.stuck
    )
}

pub fn cancelled_job_text(job_id: &str) -> String {
    format!("Job {} has been cancelled.", job_id)
}

pub fn job_list_text<I, S>(jobs: I) -> String
where
    I: IntoIterator<Item = (Uuid, String, S)>,
    S: std::fmt::Debug,
{
    let mut iter = jobs.into_iter().peekable();
    if iter.peek().is_none() {
        return "No jobs found.".to_string();
    }

    let mut output = String::from("Jobs:\n");
    for (job_id, title, state) in iter {
        output.push_str(&format!("  {} - {} ({:?})\n", job_id, title, state));
    }
    output
}

pub fn stuck_job_recovery_text(job_id: &str, next_attempt: u32) -> String {
    format!(
        "Job {} was stuck. Attempting recovery (attempt #{}).",
        job_id, next_attempt
    )
}

pub fn job_not_stuck_text(job_id: &str, state: impl std::fmt::Debug) -> String {
    format!(
        "Job {} is not stuck (current state: {:?}). No help needed.",
        job_id, state
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledSkillView {
    pub name: String,
    pub version: String,
    pub trust: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSearchResultView {
    pub name: String,
    pub version: String,
    pub owner: Option<String>,
    pub stars: Option<u64>,
    pub downloads: Option<u64>,
    pub description: String,
}

pub fn installed_skills_text(skills: &[InstalledSkillView]) -> String {
    if skills.is_empty() {
        return "No skills installed.\n\nUse /skills search <query> to find skills on ClawHub."
            .to_string();
    }

    let mut out = String::from("Installed skills:\n\n");
    for skill in skills {
        let desc = if skill.description.chars().count() > 60 {
            let truncated: String = skill.description.chars().take(57).collect();
            format!("{}...", truncated)
        } else {
            skill.description.clone()
        };
        out.push_str(&format!(
            "  {:<24} v{:<10} [{}]  {}\n",
            skill.name, skill.version, skill.trust, desc,
        ));
    }
    out.push_str("\nUse /skills search <query> to find more on ClawHub.");
    out
}

pub fn skill_search_text(
    query: &str,
    entries: &[SkillSearchResultView],
    registry_error: Option<&str>,
    installed_matches: &[InstalledSkillView],
) -> String {
    let mut out = format!("ClawHub results for \"{}\":\n\n", query);

    if entries.is_empty() {
        if let Some(err) = registry_error {
            out.push_str(&format!("  (registry error: {})\n", err));
        } else {
            out.push_str("  No results found.\n");
        }
    } else {
        for entry in entries {
            let owner_str = entry
                .owner
                .as_deref()
                .map(|owner| format!("  by {}", owner))
                .unwrap_or_default();

            let stats_parts: Vec<String> = [
                entry.stars.map(|stars| format!("{} stars", stars)),
                entry
                    .downloads
                    .map(|downloads| format_count(downloads, "downloads")),
            ]
            .into_iter()
            .flatten()
            .collect();
            let stats_str = if stats_parts.is_empty() {
                String::new()
            } else {
                format!("  {}", stats_parts.join("  "))
            };

            out.push_str(&format!(
                "  {:<24} v{:<10}{}{}\n",
                entry.name, entry.version, owner_str, stats_str,
            ));
            if !entry.description.is_empty() {
                out.push_str(&format!("    {}\n\n", entry.description));
            }
        }
    }

    if !installed_matches.is_empty() {
        out.push_str(&format!("Installed skills matching \"{}\":\n", query));
        for skill in installed_matches {
            out.push_str(&format!(
                "  {:<24} v{:<10} [{}]\n",
                skill.name, skill.version, skill.trust,
            ));
        }
    }

    out
}

pub fn agent_display_name(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Assistant"
    } else {
        trimmed
    }
}

pub fn memory_growth_text(workspace_available: bool) -> String {
    format!(
        "Memory & Growth\n\nWorkspace memory: {}\nCore tools: memory_search, memory_read, memory_write, memory_tree, session_search\nLearning tools: learning_status, learning_outcomes, learning_history, learning_feedback, learning_proposal_review, prompt_manage\nShared commands: /compress, /summarize, /skills, /heartbeat\nWebUI surfaces: Memory & Growth, Skills, Learning Ledger\n\nUse /skills to inspect installed skills and the WebUI tabs to browse durable memory and learning history.",
        if workspace_available {
            "available"
        } else {
            "unavailable until a workspace/database is attached"
        }
    )
}

pub fn agent_status_text(model: &str, workspace_mode: &str) -> String {
    format!(
        "Agent status\n\
         ──────────────────────\n\
         ✅ Reachable\n\
         Model:     {model}\n\
         Workspace: {workspace_mode}",
    )
}

pub fn skin_command_text(
    args: &[String],
    configured_skin: &str,
    available_skins: &[String],
) -> String {
    let available = available_skins.join(", ");
    if args.is_empty() || args[0].eq_ignore_ascii_case("current") {
        format!(
            "Current CLI skin: {}\nAvailable skins: {}\n\nUse /skin <name> in your local CLI client to switch immediately.",
            configured_skin, available
        )
    } else if args[0].eq_ignore_ascii_case("list") {
        format!(
            "Available skins: {}\n\nUse /skin <name> in your local CLI client to switch immediately.",
            available
        )
    } else if args[0].eq_ignore_ascii_case("reset") {
        format!(
            "Local clients can reset to their configured default skin. This agent is currently configured for '{}'.",
            configured_skin
        )
    } else {
        let requested = args.join(" ");
        format!(
            "Skin '{}' is available as a local client preset. Current configured skin: {}\nAvailable skins: {}",
            requested, configured_skin, available
        )
    }
}

pub fn active_model_text(current: &str, models: Result<&[String], &str>) -> String {
    let mut out = format!("Active model: {}\n", current);
    match models {
        Ok(models) if !models.is_empty() => {
            out.push_str("\nAvailable models:\n");
            for model in models {
                let marker = if model == current { " (active)" } else { "" };
                out.push_str(&format!("  {}{}\n", model, marker));
            }
            out.push_str("\nUse /model <name> to switch.");
        }
        Ok(_) => {
            out.push_str("\nCould not fetch model list. Use /model <name> to switch.");
        }
        Err(error) => {
            out.push_str(&format!(
                "\nCould not fetch models: {}. Use /model <name> to switch.",
                error
            ));
        }
    }
    out
}

pub fn unknown_model_text(requested: &str, models: &[String]) -> String {
    format!(
        "Unknown model: {}. Available models:\n  {}",
        requested,
        models.join("\n  ")
    )
}

pub fn invalid_model_spec_text() -> &'static str {
    "Use /model <provider/model> or /model reset. Example: /model openai/gpt-4o"
}

pub fn model_reset_text() -> &'static str {
    "Switched back to the default routed model."
}

pub fn scoped_model_switched_text(requested: &str) -> String {
    format!("Switched model for this conversation to: {}", requested)
}

pub fn global_model_switched_text(requested: &str) -> String {
    format!("Switched model to: {}", requested)
}

pub fn model_switch_failed_text(error: impl std::fmt::Display) -> String {
    format!("Failed to switch model: {}", error)
}

pub fn heartbeat_clear_text() -> &'static str {
    "Heartbeat: all clear, nothing needs attention."
}

pub fn heartbeat_findings_text(message: &str) -> String {
    format!("Heartbeat findings:\n\n{}", message)
}

pub fn heartbeat_skipped_text() -> &'static str {
    "Heartbeat skipped: no HEARTBEAT.md checklist found in workspace."
}

pub fn heartbeat_failed_text(error: impl std::fmt::Display) -> String {
    format!("Heartbeat failed: {}", error)
}

pub fn empty_summary_text() -> &'static str {
    "Nothing to summarize (empty thread)."
}

pub fn thread_summary_text(summary: &str) -> String {
    format!("Thread Summary:\n\n{}", summary.trim())
}

pub fn summarize_failed_text(error: impl std::fmt::Display) -> String {
    format!("Summarize failed: {}", error)
}

pub fn empty_suggest_text() -> &'static str {
    "Nothing to suggest from (empty thread)."
}

pub fn suggested_next_steps_text(suggestions: &str) -> String {
    format!("Suggested Next Steps:\n\n{}", suggestions.trim())
}

pub fn suggest_failed_text(error: impl std::fmt::Display) -> String {
    format!("Suggest failed: {}", error)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSourceSection {
    pub label: String,
    pub active: bool,
    pub preview: String,
}

impl ContextSourceSection {
    pub fn new(label: impl Into<String>, active: bool, preview: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            active,
            preview: preview.into(),
        }
    }
}

pub fn context_sources_text(sections: &[ContextSourceSection], detail: bool) -> String {
    let mut out = String::from("Context sources\n──────────────────────\n");
    for section in sections {
        let icon = if section.active { "✅" } else { "❌" };
        if detail && !section.preview.is_empty() {
            out.push_str(&format!(
                "\n{} {}\n{}\n",
                icon, section.label, section.preview
            ));
        } else {
            out.push_str(&format!(
                "{} {}  {}\n",
                icon, section.label, section.preview
            ));
        }
    }
    out
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
        render_section("Shared system", &help_entries_for(HelpSection::System)),
        render_section("Shared session", &help_entries_for(HelpSection::Session)),
        render_section(
            "Shared memory & growth",
            &help_entries_for(HelpSection::Memory)
        ),
        render_section("Shared identity, skills, and agent", &{
            let mut combined = help_entries_for(HelpSection::Identity);
            combined.extend(help_entries_for(HelpSection::Skills));
            combined.extend(help_entries_for(HelpSection::Agent));
            combined
        }),
    )
}

/// TUI-local commands that never appear in the shared submission-parser
/// vocabulary (they are handled entirely inside the TUI, e.g. job shortcuts
/// or view controls), plus the `/thread` and `/resume` prefix tokens which
/// the registry only tracks as usage-hint help labels
/// (`/thread new`, `/thread <id>`, `/resume <id>`).
const TUI_ONLY_FORWARDED: &[&str] = &["/job", "/cancel", "/list", "/thread", "/resume"];

const TUI_ONLY_AUTOCOMPLETE: &[&str] = &[
    "/back", "/close", "/dismiss", "/top", "/bottom", "/reset", "/think", "/job", "/cancel",
    "/list", "/thread", "/resume", "/cls",
];

pub fn tui_forwarded_commands() -> &'static [&'static str] {
    static COMMANDS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    COMMANDS.get_or_init(|| {
        let mut names: Vec<&'static str> = command_registry::forwarded_names().collect();
        names.extend_from_slice(TUI_ONLY_FORWARDED);
        names
    })
}

pub fn tui_autocomplete_commands() -> &'static [&'static str] {
    static COMMANDS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    COMMANDS.get_or_init(|| {
        let mut names: Vec<&'static str> = command_registry::autocomplete_names().collect();
        names.extend_from_slice(TUI_ONLY_AUTOCOMPLETE);
        names
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_format_uses_plain_k_and_m_suffixes() {
        assert_eq!(format_count(42, "downloads"), "42 downloads");
        assert_eq!(format_count(1_250, "downloads"), "1.2K downloads");
        assert_eq!(format_count(1_250_000, "downloads"), "1.2M downloads");
    }

    /// Regression test for command-vocabulary drift: every registry command
    /// flagged `tui_forwarded`/`tui_autocomplete` must actually show up in
    /// the generated TUI lists. This previously drifted twice: `/debug` was
    /// help-listed but missing from `tui_autocomplete_commands()`, and
    /// `/skin` was autocompletable but missing from
    /// `tui_forwarded_commands()`.
    #[test]
    fn help_forwarded_and_autocomplete_lists_stay_consistent_with_registry() {
        let forwarded = tui_forwarded_commands();
        let autocomplete = tui_autocomplete_commands();

        for spec in command_registry::COMMAND_REGISTRY {
            if spec.tui_forwarded {
                assert!(
                    forwarded.contains(&spec.name),
                    "{:?} is flagged tui_forwarded but missing from tui_forwarded_commands()",
                    spec.name
                );
            }
            if spec.tui_autocomplete {
                assert!(
                    autocomplete.contains(&spec.name),
                    "{:?} is flagged tui_autocomplete but missing from tui_autocomplete_commands()",
                    spec.name
                );
            }
        }

        // The two drift items the registry design fixes: /debug is
        // autocompletable, and /skin is forwarded.
        assert!(autocomplete.contains(&"/debug"));
        assert!(forwarded.contains(&"/skin"));
    }

    #[test]
    fn blank_agent_name_falls_back_to_assistant() {
        assert_eq!(agent_display_name("   "), "Assistant");
        assert_eq!(agent_display_name("ThinClaw"), "ThinClaw");
    }

    #[test]
    fn rollback_usage_lists_supported_forms() {
        let usage = rollback_usage();
        assert!(usage.contains("/rollback list"));
        assert!(usage.contains("/rollback diff <N>"));
    }

    #[test]
    fn rollback_text_helpers_render_checkpoint_responses() {
        let root = std::path::Path::new("/tmp/project");
        assert!(rollback_active_project_text(root).contains("Active project"));
        assert!(rollback_no_checkpoints_text(root).contains("/tmp/project"));
        assert_eq!(
            rollback_checkpoint_not_found_text(3),
            "Checkpoint 3 not found. Run `/rollback list` to inspect available checkpoints."
        );
        assert_eq!(
            rollback_empty_diff_text(1),
            "No differences between checkpoint 1 and the current project state."
        );
        assert_eq!(
            rollback_restored_text(2, "1234567890abcdef", Some("src/main.rs")),
            "Restored src/main.rs from checkpoint 2 (1234567890ab)"
        );
    }

    #[test]
    fn job_command_text_helpers_render_stable_messages() {
        let job_id = Uuid::nil();
        assert!(created_job_text("Build", job_id).contains("Created job: Build"));
        assert_eq!(cancelled_job_text("abc"), "Job abc has been cancelled.");
        assert_eq!(
            jobs_summary_text(JobSummaryView {
                total: 2,
                in_progress: 1,
                completed: 1,
                failed: 0,
                stuck: 0,
            }),
            "Jobs summary:\n  Total: 2\n  In Progress: 1\n  Completed: 1\n  Failed: 0\n  Stuck: 0"
        );
        assert_eq!(
            job_list_text(Vec::<(Uuid, String, &str)>::new()),
            "No jobs found."
        );
        assert!(stuck_job_recovery_text("abc", 2).contains("attempt #2"));
        assert!(job_not_stuck_text("abc", "Completed").contains("not stuck"));
    }

    #[test]
    fn skill_command_text_helpers_render_lists_and_search_results() {
        assert!(installed_skills_text(&[]).contains("No skills installed"));
        let skills = vec![InstalledSkillView {
            name: "review".to_string(),
            version: "1.0.0".to_string(),
            trust: "user".to_string(),
            description: "A very useful review skill".to_string(),
        }];
        assert!(installed_skills_text(&skills).contains("review"));

        let entries = vec![SkillSearchResultView {
            name: "research".to_string(),
            version: "0.2.0".to_string(),
            owner: Some("team".to_string()),
            stars: Some(5),
            downloads: Some(1_250),
            description: "Research helper".to_string(),
        }];
        let search = skill_search_text("res", &entries, None, &skills);
        assert!(search.contains("ClawHub results"));
        assert!(search.contains("1.2K downloads"));
        assert!(search.contains("Installed skills matching"));
    }

    #[test]
    fn command_response_helpers_render_core_statuses() {
        assert!(memory_growth_text(true).contains("Workspace memory: available"));
        assert!(memory_growth_text(false).contains("unavailable"));
        assert!(agent_status_text("provider/model", "project").contains("provider/model"));

        let skins = vec!["plain".to_string(), "neon".to_string()];
        assert!(skin_command_text(&[], "plain", &skins).contains("Current CLI skin: plain"));
        assert!(
            skin_command_text(&["list".to_string()], "plain", &skins).contains("Available skins")
        );
        assert!(
            skin_command_text(&["neon".to_string()], "plain", &skins)
                .contains("Skin 'neon' is available")
        );
    }

    #[test]
    fn model_command_text_helpers_render_model_states() {
        let models = vec!["openai/gpt-4o".to_string(), "anthropic/sonnet".to_string()];
        assert!(active_model_text("openai/gpt-4o", Ok(&models)).contains("(active)"));
        assert!(active_model_text("openai/gpt-4o", Ok(&[])).contains("Could not fetch"));
        assert!(active_model_text("openai/gpt-4o", Err("offline")).contains("offline"));
        assert!(unknown_model_text("bogus", &models).contains("Available models"));
        assert_eq!(
            invalid_model_spec_text(),
            "Use /model <provider/model> or /model reset. Example: /model openai/gpt-4o"
        );
        assert_eq!(
            model_reset_text(),
            "Switched back to the default routed model."
        );
        assert!(scoped_model_switched_text("openai/gpt-4o").contains("conversation"));
        assert_eq!(global_model_switched_text("x/y"), "Switched model to: x/y");
        assert_eq!(
            model_switch_failed_text("nope"),
            "Failed to switch model: nope"
        );
    }

    #[test]
    fn heartbeat_summary_and_suggest_text_helpers_render_wrappers() {
        assert_eq!(
            heartbeat_clear_text(),
            "Heartbeat: all clear, nothing needs attention."
        );
        assert_eq!(
            heartbeat_findings_text("check item"),
            "Heartbeat findings:\n\ncheck item"
        );
        assert!(heartbeat_skipped_text().contains("skipped"));
        assert_eq!(heartbeat_failed_text("boom"), "Heartbeat failed: boom");
        assert_eq!(empty_summary_text(), "Nothing to summarize (empty thread).");
        assert_eq!(thread_summary_text("  done  "), "Thread Summary:\n\ndone");
        assert_eq!(summarize_failed_text("bad"), "Summarize failed: bad");
        assert_eq!(
            empty_suggest_text(),
            "Nothing to suggest from (empty thread)."
        );
        assert_eq!(
            suggested_next_steps_text("  1. Ship  "),
            "Suggested Next Steps:\n\n1. Ship"
        );
        assert_eq!(suggest_failed_text("bad"), "Suggest failed: bad");
    }

    #[test]
    fn context_sources_render_detail_and_summary_modes() {
        let sections = vec![
            ContextSourceSection::new("Safety guardrails", true, ""),
            ContextSourceSection::new("Workspace", false, "(no workspace connected)"),
        ];
        let summary = context_sources_text(&sections, false);
        assert!(summary.contains("✅ Safety guardrails"));
        assert!(summary.contains("❌ Workspace  (no workspace connected)"));

        let detail = context_sources_text(
            &[ContextSourceSection::new("AGENTS.md", true, "# Agents")],
            true,
        );
        assert!(detail.contains("\n✅ AGENTS.md\n# Agents\n"));
    }
}
