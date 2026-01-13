use clap::Subcommand;

mod default_index;
#[cfg(feature = "global-binaries")]
mod global_binaries;

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
	/// Configuration for the default index
	DefaultIndex(default_index::DefaultIndexCommand),
	/// Whether to generate global binary linkers (e.g. `mybin` when you depend on mybin that has a binary)
	#[cfg(feature = "global-binaries")]
	GlobalBinaries(global_binaries::GlobalBinariesCommand),
}

impl ConfigCommands {
	pub async fn run(self) -> anyhow::Result<()> {
		match self {
			ConfigCommands::DefaultIndex(default_index) => default_index.run().await,
			#[cfg(feature = "global-binaries")]
			ConfigCommands::GlobalBinaries(global_binaries) => global_binaries.run().await,
		}
	}
}
