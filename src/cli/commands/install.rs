use crate::cli::install::InstallOptions;
use crate::cli::install::install;
use clap::Args;
use pesde::Subproject;
use pesde::download_and_link::InstallDependenciesMode;
use std::num::NonZeroUsize;

#[derive(Debug, Args, Copy, Clone)]
pub struct InstallCommand {
	/// Whether to error on changes in the lockfile
	#[arg(long)]
	locked: bool,

	/// Whether to not install dev dependencies
	#[arg(long)]
	prod: bool,

	/// Whether to only install dev dependencies
	#[arg(long)]
	dev: bool,

	/// The maximum number of concurrent network requests
	#[arg(long, default_value = "16")]
	network_concurrency: NonZeroUsize,

	/// Whether to re-install all dependencies even if they are already installed
	#[arg(long)]
	force: bool,
}

impl InstallCommand {
	pub async fn run(self, subproject: Subproject, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let install_dependencies_mode = match (self.prod, self.dev) {
			(true, true) => anyhow::bail!("cannot have both prod and dev flags enabled"),
			(true, false) => InstallDependenciesMode::Prod,
			(false, true) => InstallDependenciesMode::Dev,
			(false, false) => InstallDependenciesMode::All,
		};

		let options = InstallOptions {
			locked: self.locked,
			install_dependencies_mode,
			write: true,
			network_concurrency: self.network_concurrency,
			use_lockfile: true,
			force: self.force,
		};

		install(&options, subproject.project(), reqwest.clone()).await?;

		Ok(())
	}
}
