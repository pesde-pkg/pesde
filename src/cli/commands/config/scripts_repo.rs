use crate::cli::{
    config::{read_config, write_config, CliConfig},
    home_dir,
};
use anyhow::Context;
use clap::Args;
use fs_err::tokio as fs;

#[derive(Debug, Args)]
pub struct ScriptsRepoCommand {
    /// The new repo URL to set as default, don't pass any value to check the current default repo
    #[arg(index = 1, value_parser = crate::cli::parse_gix_url)]
    repo: Option<gix::Url>,

    /// Resets the default repo to the default value
    #[arg(short, long, conflicts_with = "repo")]
    reset: bool,
}

impl ScriptsRepoCommand {
    pub async fn run(self) -> anyhow::Result<()> {
        let mut config = read_config().await?;

        let repo = if self.reset {
            Some(CliConfig::default().scripts_repo)
        } else {
            self.repo
        };

        match repo {
            Some(repo) => {
                config.scripts_repo = repo.clone();
                write_config(&config).await?;

                fs::remove_dir_all(home_dir()?.join("scripts"))
                    .await
                    .context("failed to remove scripts directory")?;

                println!("scripts repo set to: {repo}");
            }
            None => {
                println!("current scripts repo: {}", config.scripts_repo);
            }
        }

        Ok(())
    }
}
