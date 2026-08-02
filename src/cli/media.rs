use clap::Subcommand;

use super::ComfyCommand;

#[derive(Subcommand, Debug, Clone)]
pub enum MediaCommand {
    #[command(subcommand)]
    Comfy(ComfyCommand),
}
