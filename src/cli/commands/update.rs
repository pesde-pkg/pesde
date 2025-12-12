use crate::cli::{
	install::{InstallOptions, install},
	run_on_workspace_members,
};
use clap::Args;
use pesde::{Project, download_and_link::InstallDependenciesMode};
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
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let options = InstallOptions {
			locked: false,
			install_dependencies_mode: InstallDependenciesMode::All,
			write: !self.no_install,
			network_concurrency: self.network_concurrency,
			use_lockfile: false,
			force: self.force,
		};

		install(&options, &project, reqwest.clone(), true).await?;

		run_on_workspace_members(&project, |project| {
			let reqwest = reqwest.clone();
			async move {
				install(&options, &project, reqwest, false).await?;
				Ok(())
			}
		})
		.await?;

		Ok(())
	}
}
