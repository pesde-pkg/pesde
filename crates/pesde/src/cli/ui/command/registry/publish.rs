use clap::Args;

use crate::cli::ui::command::registry::RegistryCommandContext;

/// Publishes a package
#[derive(Debug, Args)]
pub struct PublishCommand {
	/// Whether to save the package to a file instead of publishing it to the registry
	#[arg(short, long)]
	dry_run: bool,

	/// Agree to all prompts
	#[arg(short, long)]
	yes: bool,
}

impl PublishCommand {
	pub async fn run(self, ctx: RegistryCommandContext) -> anyhow::Result<()> {
		Ok(())
	}
}
