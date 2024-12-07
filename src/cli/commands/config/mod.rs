use clap::Subcommand;

mod default_index;

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Configuration for the default index
    DefaultIndex(default_index::DefaultIndexCommand),
}

impl ConfigCommands {
    pub async fn run(self) -> anyhow::Result<()> {
        match self {
            ConfigCommands::DefaultIndex(default_index) => default_index.run().await,
        }
    }
}
