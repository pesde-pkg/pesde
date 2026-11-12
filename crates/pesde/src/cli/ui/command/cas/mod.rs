use clap::Subcommand;

use crate::cli::ui::CommandContext;

mod path;
mod prune;

/// CAS-related commands
#[derive(Debug, Subcommand)]
pub enum CasCommand {
	Path(path::PathCommand),

	Prune(prune::PruneCommand),
}

impl CasCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		match self {
			CasCommand::Path(path) => path.run(ctx).await,
			CasCommand::Prune(prune) => prune.run(ctx).await,
		}
	}
}
