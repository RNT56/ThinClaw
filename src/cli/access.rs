//! Canonical identity, sender, and device access operations.

use clap::Subcommand;

use super::{DeviceCommand, IdentityCommand, PairingCommand};

#[derive(Subcommand, Debug, Clone)]
pub enum AccessCommand {
    #[command(subcommand)]
    Identities(IdentityCommand),
    #[command(subcommand)]
    Senders(PairingCommand),
    #[command(subcommand)]
    Devices(DeviceCommand),
}
