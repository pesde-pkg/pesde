use crate::cli::install::CliInstallOptions;
use crate::cli::install::install;
use crate::cli::ui::CommandContext;
use clap::Args;
use pesde::install::InstallDependenciesMode;
use std::num::NonZero;

/// Installs all dependencies for the subproject
#[derive(Debug, Args, Copy, Clone)]
#[command(name = "install", visible_alias = "i")]
pub struct InstallCommand {
	/// Whether to error on changes in the lockfile
	#[arg(short, long)]
	locked: bool,

	/// Whether to not install dev dependencies
	#[arg(long)]
	prod: bool,

	/// Whether to only install dev dependencies
	#[arg(long)]
	dev: bool,

	/// The maximum number of concurrent network requests
	#[arg(short, long, default_value = "16")]
	network_concurrency: NonZero<usize>,

	/// Whether to re-install all dependencies even if they are already installed
	#[arg(short, long)]
	force: bool,
}

impl InstallCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let install_dependencies_mode = match (self.prod, self.dev) {
			(true, true) => anyhow::bail!("cannot have both prod and dev flags enabled"),
			(true, false) => InstallDependenciesMode::Prod,
			(false, true) => InstallDependenciesMode::Dev,
			(false, false) => InstallDependenciesMode::All,
		};

		let options = CliInstallOptions {
			locked: self.locked,
			install_dependencies_mode,
			write: true,
			network_concurrency: self.network_concurrency,
			use_lockfile: true,
			force: self.force,
		};

		let project = ctx.project().await?;
		install(&options, project).await?;

		Ok(())
	}
}
