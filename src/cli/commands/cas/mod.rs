use clap::Subcommand;
use pesde::Subproject;

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
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		match self {
			CasCommands::Path(path) => path.run(subproject),
			CasCommands::Prune(prune) => prune.run(subproject).await,
		}
	}
}
