//! Canonical long-running runtime operations.

use clap::Subcommand;

use super::ServiceCommand;
use super::{GatewayCommand, LogCommand, UpdateCommand};

#[derive(Subcommand, Debug, Clone)]
pub enum RuntimeCommand {
    #[command(subcommand)]
    Web(GatewayCommand),
    #[command(subcommand)]
    Service(ServiceCommand),
    #[command(subcommand)]
    Logs(LogCommand),
    #[command(subcommand)]
    Update(UpdateCommand),
}
