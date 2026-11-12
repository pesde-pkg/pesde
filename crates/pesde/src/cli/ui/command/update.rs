use crate::cli::install::CliInstallOptions;
use crate::cli::install::install;
use crate::cli::ui::CommandContext;
use clap::Args;
use pesde::install::InstallDependenciesMode;
use std::num::NonZero;

/// Updates the subproject's lockfile. Run install to apply changes
#[derive(Debug, Args, Copy, Clone)]
pub struct UpdateCommand {
	/// Update the dependencies but don't install them
	#[arg(long)]
	no_install: bool,

	/// The maximum number of concurrent network requests
	#[arg(short, long, default_value = "16")]
	network_concurrency: NonZero<usize>,

	/// Whether to re-install all dependencies even if they are already installed
	#[arg(short, long)]
	force: bool,
}

impl UpdateCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let options = CliInstallOptions {
			locked: false,
			install_dependencies_mode: InstallDependenciesMode::All,
			write: !self.no_install,
			network_concurrency: self.network_concurrency,
			use_lockfile: false,
			force: self.force,
		};

		let project = ctx.project().await?;
		install(&options, project).await?;

		Ok(())
	}
}
