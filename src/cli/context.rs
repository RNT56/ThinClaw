//! Per-invocation CLI context resolved after environment bootstrap.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use super::{CliError, ColorChoice, OutputFormat, OutputPolicy};

#[derive(Debug, Clone)]
pub struct CliContextOptions {
    pub output_format: OutputFormat,
    pub color: ColorChoice,
    pub quiet: bool,
    pub verbose: bool,
    pub debug: bool,
    pub config_path: Option<PathBuf>,
}

#[derive(Debug)]
pub struct CliContext {
    output: OutputPolicy,
    config_path: Option<PathBuf>,
}

impl CliContext {
    pub fn resolve(options: CliContextOptions) -> Result<Self, CliError> {
        let config_path = options
            .config_path
            .as_deref()
            .map(validate_explicit_config)
            .transpose()?;
        let output = OutputPolicy::resolve(
            options.output_format,
            options.color,
            options.quiet,
            options.verbose,
            options.debug,
            std::io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").is_some(),
        )?;

        Ok(Self {
            output,
            config_path,
        })
    }

    pub const fn output(&self) -> &OutputPolicy {
        &self.output
    }

    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    pub const fn debug(&self) -> bool {
        self.output.debug
    }
}

fn validate_explicit_config(path: &Path) -> Result<PathBuf, CliError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        CliError::operational(format!(
            "explicit configuration file '{}' is unavailable: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CliError::operational(format!(
            "explicit configuration path '{}' must be a regular non-symlink file",
            path.display()
        )));
    }
    std::fs::File::open(path).map_err(|error| {
        CliError::operational(format!(
            "explicit configuration file '{}' is not readable: {error}",
            path.display()
        ))
    })?;
    path.canonicalize().map_err(|error| {
        CliError::operational(format!(
            "failed to canonicalize explicit configuration file '{}': {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(path: Option<PathBuf>) -> CliContextOptions {
        CliContextOptions {
            output_format: OutputFormat::Human,
            color: ColorChoice::Never,
            quiet: false,
            verbose: false,
            debug: false,
            config_path: path,
        }
    }

    #[test]
    fn explicit_missing_config_fails() {
        let missing =
            std::env::temp_dir().join(format!("thinclaw-missing-config-{}", uuid::Uuid::new_v4()));
        let error = CliContext::resolve(options(Some(missing))).expect_err("missing path fails");
        assert_eq!(error.exit_class(), crate::cli::ExitClass::Operational);
    }

    #[test]
    fn explicit_regular_config_is_canonicalized() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("thinclaw.toml");
        std::fs::write(&path, b"# test\n").expect("write fixture");
        let context = CliContext::resolve(options(Some(path.clone()))).expect("valid context");
        assert_eq!(
            context.config_path(),
            Some(path.canonicalize().unwrap().as_path())
        );
    }
}
