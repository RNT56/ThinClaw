//! Shared CLI presentation and serialization policy.

use std::io::{self, Write};

use clap::ValueEnum;
use serde::Serialize;

use super::outcome::CliError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
    Jsonl,
}

impl OutputFormat {
    pub const fn is_machine(self) -> bool {
        matches!(self, Self::Json | Self::Jsonl)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputPolicy {
    pub format: OutputFormat,
    pub color: ColorChoice,
    pub color_enabled: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub debug: bool,
}

impl OutputPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        format: OutputFormat,
        color: ColorChoice,
        quiet: bool,
        verbose: bool,
        debug: bool,
        stdout_is_terminal: bool,
        no_color_is_set: bool,
    ) -> Result<Self, CliError> {
        if quiet && verbose {
            return Err(CliError::usage("--quiet conflicts with --verbose"));
        }
        if format.is_machine() && color == ColorChoice::Always {
            return Err(CliError::usage(
                "--color always cannot be used with JSON or JSONL output",
            ));
        }

        let color_enabled = if format.is_machine() {
            false
        } else {
            match color {
                ColorChoice::Always => true,
                ColorChoice::Never => false,
                ColorChoice::Auto => stdout_is_terminal && !no_color_is_set,
            }
        };

        Ok(Self {
            format,
            color,
            color_enabled,
            quiet,
            verbose,
            debug,
        })
    }

    pub fn write_record<T: Serialize>(
        &self,
        command: &str,
        data: &T,
        human: impl FnOnce(&T) -> String,
    ) -> Result<(), CliError> {
        let bytes = self.render_record(command, data, human)?;
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(&bytes)
            .and_then(|_| stdout.flush())
            .map_err(|error| CliError::operational(format!("failed to write stdout: {error}")))
    }

    pub fn render_record<T: Serialize>(
        &self,
        command: &str,
        data: &T,
        human: impl FnOnce(&T) -> String,
    ) -> Result<Vec<u8>, CliError> {
        match self.format {
            OutputFormat::Human => {
                let mut rendered = human(data).into_bytes();
                if !rendered.ends_with(b"\n") {
                    rendered.push(b'\n');
                }
                Ok(rendered)
            }
            OutputFormat::Json => encode_json_line(&RecordEnvelope {
                schema_version: 1,
                command,
                data,
            }),
            OutputFormat::Jsonl => encode_json_line(&EventEnvelope {
                schema_version: 1,
                command,
                event_type: "record",
                data,
            }),
        }
    }

    pub fn diagnostic(&self, message: impl AsRef<str>) -> Result<(), CliError> {
        let mut stderr = io::stderr().lock();
        writeln!(stderr, "{}", message.as_ref())
            .and_then(|_| stderr.flush())
            .map_err(|error| CliError::operational(format!("failed to write stderr: {error}")))
    }

    pub fn progress(&self, message: impl AsRef<str>) -> Result<(), CliError> {
        if self.quiet || self.format.is_machine() {
            return Ok(());
        }
        self.diagnostic(message)
    }
}

#[derive(Serialize)]
struct RecordEnvelope<'a, T> {
    schema_version: u8,
    command: &'a str,
    data: &'a T,
}

#[derive(Serialize)]
struct EventEnvelope<'a, T> {
    schema_version: u8,
    command: &'a str,
    #[serde(rename = "type")]
    event_type: &'static str,
    data: &'a T,
}

fn encode_json_line<T: Serialize>(value: &T) -> Result<Vec<u8>, CliError> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| {
        CliError::operational(format!("failed to encode command output: {error}"))
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(format: OutputFormat) -> OutputPolicy {
        OutputPolicy::resolve(format, ColorChoice::Auto, false, false, false, false, false)
            .expect("valid policy")
    }

    #[test]
    fn json_record_is_one_versioned_document() {
        let bytes = policy(OutputFormat::Json)
            .render_record("agents.list", &Vec::<String>::new(), |_| String::new())
            .expect("render JSON");
        assert!(!bytes.contains(&0x1b));
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["command"], "agents.list");
        assert_eq!(value["data"], serde_json::json!([]));
    }

    #[test]
    fn jsonl_record_is_one_versioned_event() {
        let bytes = policy(OutputFormat::Jsonl)
            .render_record("status", &serde_json::json!({"healthy": true}), |_| {
                String::new()
            })
            .expect("render JSONL");
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSONL row");
        assert_eq!(value["type"], "record");
    }

    #[test]
    fn no_color_and_machine_modes_disable_color() {
        let no_color = OutputPolicy::resolve(
            OutputFormat::Human,
            ColorChoice::Auto,
            false,
            false,
            false,
            true,
            true,
        )
        .expect("valid policy");
        assert!(!no_color.color_enabled);

        let machine = policy(OutputFormat::Json);
        assert!(!machine.color_enabled);
    }

    #[test]
    fn explicit_always_overrides_no_color_only_for_human_output() {
        let human = OutputPolicy::resolve(
            OutputFormat::Human,
            ColorChoice::Always,
            false,
            false,
            false,
            false,
            true,
        )
        .expect("valid human policy");
        assert!(human.color_enabled);

        let error = OutputPolicy::resolve(
            OutputFormat::Json,
            ColorChoice::Always,
            false,
            false,
            false,
            true,
            false,
        )
        .expect_err("machine color must conflict");
        assert_eq!(error.exit_class(), crate::cli::ExitClass::Usage);
    }
}
