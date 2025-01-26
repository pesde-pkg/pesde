use clap::Subcommand;
use pesde::Project;

mod prune;

#[derive(Debug, Subcommand)]
pub enum CasCommands {
	/// Removes unused files from the CAS
	Prune(prune::PruneCommand),
}

impl CasCommands {
	pub async fn run(self, project: Project) -> anyhow::Result<()> {
		match self {
			CasCommands::Prune(prune) => prune.run(project).await,
		}
	}
}
