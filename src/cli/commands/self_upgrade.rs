use crate::cli::{config::read_config, version::get_or_download_version};
use clap::Args;

#[derive(Debug, Args)]
pub struct SelfUpgradeCommand {}

impl SelfUpgradeCommand {
    pub async fn run(self, reqwest: reqwest::Client) -> anyhow::Result<()> {
        let config = read_config().await?;

        get_or_download_version(&reqwest, &config.last_checked_updates.unwrap().1).await?;
        // a call to `update_bin_exe` or other similar function *should* be here, in case new versions
        // have fixes to bugs in executing other versions, but that would cause
        // the current file to be overwritten by itself, so this needs more thought

        Ok(())
    }
}
