use crate::cli::{
	install::{install, InstallOptions},
	run_on_workspace_members,
};
use clap::Args;
use pesde::Project;
use std::num::NonZeroUsize;

#[derive(Debug, Args, Copy, Clone)]
pub struct InstallCommand {
	/// Whether to error on changes in the lockfile
	#[arg(long)]
	locked: bool,

	/// Whether to not install dev dependencies
	#[arg(long)]
	prod: bool,

	/// The maximum number of concurrent network requests
	#[arg(long, default_value = "16")]
	network_concurrency: NonZeroUsize,

	/// Whether to re-install all dependencies even if they are already installed
	#[arg(long)]
	force: bool,
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
struct CallbackError(#[from] anyhow::Error);
impl InstallCommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let options = InstallOptions {
			locked: self.locked,
			prod: self.prod,
			write: true,
			network_concurrency: self.network_concurrency,
			use_lockfile: true,
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
