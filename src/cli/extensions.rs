//! Canonical extension and integration operations.

use clap::Subcommand;

use super::{ChannelCommand, McpCommand, RegistryCommand, ToolCommand};

#[derive(Subcommand, Debug, Clone)]
pub enum ExtensionsCommand {
    #[command(subcommand)]
    Channels(ChannelCommand),
    #[command(subcommand)]
    Tools(ToolCommand),
    #[command(subcommand)]
    Registry(RegistryCommand),
    #[command(subcommand)]
    Mcp(McpCommand),
}
