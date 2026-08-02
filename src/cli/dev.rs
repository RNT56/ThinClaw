use clap::Subcommand;

use super::BrowserCommand;

#[derive(Subcommand, Debug, Clone)]
pub enum DevCommand {
    #[command(subcommand)]
    Browser(BrowserCommand),
}
