use crate::cli::{
    config::read_config,
    version::{
        current_version, get_latest_remote_version, get_or_download_version, update_bin_exe,
    },
};
use anyhow::Context;
use clap::Args;
use colored::Colorize;
use semver::BuildMetadata;

#[derive(Debug, Args)]
pub struct SelfUpgradeCommand {
    /// Whether to use the version from the "upgrades available" message
    #[clap(long, default_value_t = false)]
    use_cached: bool,
}

impl SelfUpgradeCommand {
    pub async fn run(self, reqwest: reqwest::Client) -> anyhow::Result<()> {
        let latest_version = if self.use_cached {
            read_config()
                .await?
                .last_checked_updates
                .context("no cached version found")?
                .1
        } else {
            get_latest_remote_version(&reqwest).await?
        };

        if latest_version <= current_version() {
            println!("already up to date");
            return Ok(());
        }

        let display_latest_version = {
            let mut ver = latest_version.clone();
            // remove build metadata to make it more readable
            ver.build = BuildMetadata::EMPTY;
            ver.to_string().yellow().bold()
        };

        if !inquire::prompt_confirmation(format!(
            "are you sure you want to upgrade {} from {} to {display_latest_version}?",
            env!("CARGO_BIN_NAME").cyan(),
            env!("CARGO_PKG_VERSION").yellow().bold()
        ))? {
            println!("cancelled upgrade");
            return Ok(());
        }

        let path = get_or_download_version(&reqwest, &latest_version, true)
            .await?
            .unwrap();
        update_bin_exe(&path).await?;

        println!("upgraded to version {display_latest_version}!",);

        Ok(())
    }
}
