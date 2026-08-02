//! Per-invocation CLI context resolved after environment bootstrap.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::OnceCell;

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

pub struct CliContext {
    output: OutputPolicy,
    config_path: Option<PathBuf>,
    config: OnceCell<crate::config::Config>,
    database: OnceCell<Arc<dyn crate::db::Database>>,
}

impl std::fmt::Debug for CliContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CliContext")
            .field("output", &self.output)
            .field("config_path", &self.config_path)
            .field("config_resolved", &self.config.initialized())
            .field("database_connected", &self.database.initialized())
            .finish()
    }
}

impl CliContext {
    pub fn resolve(options: CliContextOptions) -> Result<Self, CliError> {
        let config_path = options
            .config_path
            .as_deref()
            .map(validate_explicit_config)
            .transpose()?;
        if let Some(path) = config_path.as_deref() {
            crate::config::select_cli_toml(path).map_err(|error| {
                CliError::operational(format!(
                    "failed to select explicit configuration file: {error}"
                ))
            })?;
        }
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
            config: OnceCell::new(),
            database: OnceCell::new(),
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

    /// Resolve configuration once for this invocation, honoring the explicit
    /// global `--config` file selected before command dispatch.
    pub async fn config(&self) -> Result<&crate::config::Config, CliError> {
        self.config
            .get_or_try_init(|| async {
                crate::config::Config::from_env_with_toml(self.config_path())
                    .await
                    .map_err(|error| {
                        CliError::operational(format!("failed to resolve configuration: {error}"))
                    })
            })
            .await
    }

    /// Connect to the configured durable store once for this invocation using
    /// the same backend factory and migration path as runtime startup.
    pub async fn database(&self) -> Result<Arc<dyn crate::db::Database>, CliError> {
        let database = self
            .database
            .get_or_try_init(|| async {
                let config = self.config().await?;
                crate::db::connect_from_config(&config.database)
                    .await
                    .map_err(|error| {
                        CliError::operational(format!(
                            "failed to initialize configured database: {error}"
                        ))
                    })
            })
            .await?;
        Ok(Arc::clone(database))
    }

    /// Construct the durable registry used by both runtime startup and CLI
    /// administration. Loading is part of construction so reads never observe
    /// a freshly-created empty router.
    pub async fn agent_registry(
        &self,
    ) -> Result<crate::agent::agent_registry::AgentRegistry, CliError> {
        let database = self.database().await?;
        let registry = crate::agent::agent_registry::AgentRegistry::new(
            Arc::new(crate::agent::AgentRouter::new()),
            Some(database),
        );
        registry.load_from_db().await.map_err(|error| {
            CliError::operational(format!("failed to load durable agent registry: {error}"))
        })?;
        Ok(registry)
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
