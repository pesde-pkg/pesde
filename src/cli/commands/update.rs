use crate::cli::{
	install::{install, InstallOptions},
	run_on_workspace_members,
};
use clap::Args;
use pesde::Project;
use std::num::NonZeroUsize;

#[derive(Debug, Args, Copy, Clone)]
pub struct UpdateCommand {
	/// Update the dependencies but don't install them
	#[arg(long)]
	no_install: bool,

	/// The maximum number of concurrent network requests
	#[arg(long, default_value = "16")]
	network_concurrency: NonZeroUsize,
}

impl UpdateCommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let options = InstallOptions {
			locked: false,
			prod: false,
			write: !self.no_install,
			network_concurrency: self.network_concurrency,
			use_lockfile: false,
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
