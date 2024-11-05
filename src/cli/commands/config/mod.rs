use clap::Subcommand;

mod default_index;
mod scripts_repo;

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Configuration for the default index
    DefaultIndex(default_index::DefaultIndexCommand),

    /// Configuration for the scripts repository
    ScriptsRepo(scripts_repo::ScriptsRepoCommand),
}

impl ConfigCommands {
    pub async fn run(self) -> anyhow::Result<()> {
        match self {
            ConfigCommands::DefaultIndex(default_index) => default_index.run().await,
            ConfigCommands::ScriptsRepo(scripts_repo) => scripts_repo.run().await,
        }
    }
}
