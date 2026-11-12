use clap::Args;

use crate::cli::ui::CommandContext;

/// Prints the path of the CAS used by the current location
#[derive(Debug, Args)]
pub struct PathCommand;

impl PathCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let vfs = ctx.vfs().await?;
		println!("{}", vfs.cas_dir.display());

		Ok(())
	}
}
