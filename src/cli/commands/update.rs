use crate::cli::install::{InstallOptions, install};
use clap::Args;
use pesde::{Subproject, download_and_link::InstallDependenciesMode};
use std::num::NonZeroUsize;

#[derive(Debug, Args, Copy, Clone)]
pub struct UpdateCommand {
	/// Update the dependencies but don't install them
	#[arg(long)]
	no_install: bool,

	/// The maximum number of concurrent network requests
	#[arg(long, default_value = "16")]
	network_concurrency: NonZeroUsize,

	/// Whether to re-install all dependencies even if they are already installed
	#[arg(long)]
	force: bool,
}

impl UpdateCommand {
	pub async fn run(self, subproject: Subproject, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let options = InstallOptions {
			locked: false,
			install_dependencies_mode: InstallDependenciesMode::All,
			write: !self.no_install,
			network_concurrency: self.network_concurrency,
			use_lockfile: false,
			force: self.force,
		};

		install(&options, subproject.project(), reqwest.clone()).await?;

		Ok(())
	}
}
