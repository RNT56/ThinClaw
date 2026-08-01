//! Canonical automation command category.

use clap::Subcommand;

use super::{CronCommand, JobCommand, RepoProjectCommand};

#[derive(Subcommand, Debug, Clone)]
pub enum AutomationCommand {
    /// Scheduled and event-driven routines
    #[command(subcommand)]
    Routines(CronCommand),
    /// Running-runtime supervised jobs
    #[command(subcommand)]
    Jobs(JobCommand),
    /// Repository project supervisor
    #[command(subcommand)]
    Projects(RepoProjectCommand),
}
