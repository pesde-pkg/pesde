use crate::{
	cli::{
		config::read_config,
		version::{
			current_version, find_latest_version, get_or_download_engine, replace_pesde_bin_exe,
		},
	},
	util::no_build_metadata,
};
use anyhow::Context;
use clap::Args;
use colored::Colorize;
use pesde::engine::EngineKind;
use semver::VersionReq;

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
			find_latest_version(&reqwest).await?
		};

		let latest_version_no_metadata = no_build_metadata(&latest_version);

		if latest_version_no_metadata <= current_version() {
			println!("already up to date");
			return Ok(());
		}

		let display_latest_version = latest_version_no_metadata.to_string().yellow().bold();

		if !inquire::prompt_confirmation(format!(
			"are you sure you want to upgrade {} from {} to {display_latest_version}?",
			env!("CARGO_BIN_NAME").cyan(),
			env!("CARGO_PKG_VERSION").yellow().bold()
		))? {
			println!("cancelled upgrade");
			return Ok(());
		}

		let path = get_or_download_engine(
			&reqwest,
			EngineKind::Pesde,
			VersionReq::parse(&format!("={latest_version}")).unwrap(),
		)
		.await?;
		replace_pesde_bin_exe(&path).await?;

		println!("upgraded to version {display_latest_version}!");

		Ok(())
	}
}
