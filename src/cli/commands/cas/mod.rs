use clap::Subcommand;
use pesde::Project;

mod path;
mod prune;

#[derive(Debug, Subcommand)]
pub enum CasCommands {
	/// Prints the path of the CAS used by the current location
	Path(path::PathCommand),

	/// Removes unused files from the CAS
	Prune(prune::PruneCommand),
}

impl CasCommands {
	pub async fn run(self, project: Project) -> anyhow::Result<()> {
		match self {
			CasCommands::Path(path) => path.run(project),
			CasCommands::Prune(prune) => prune.run(project).await,
		}
	}
}
