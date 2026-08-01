//! Canonical durable data operations.

use clap::Subcommand;

use super::{BackupCommand, MemoryCommand, SessionCommand, TrajectoryCommand};

#[derive(Subcommand, Debug, Clone)]
pub enum DataCommand {
    #[command(subcommand)]
    Memory(MemoryCommand),
    #[command(subcommand)]
    Conversations(SessionCommand),
    #[command(subcommand)]
    Backup(BackupCommand),
    #[command(subcommand)]
    Trajectories(TrajectoryCommand),
}
