use clap::Subcommand;

use super::ExperimentsCommand;

#[derive(Subcommand, Debug, Clone)]
pub enum LabsCommand {
    #[command(subcommand)]
    Experiments(ExperimentsCommand),
}
