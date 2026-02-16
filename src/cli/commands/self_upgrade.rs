use crate::cli::reporters::run_with_reporter;
use crate::cli::style::ADDED_STYLE;
use crate::cli::style::CLI_STYLE;
use crate::cli::style::REMOVED_STYLE;
use crate::cli::version::current_version;
use crate::cli::version::get_or_download_version;
use crate::cli::version::query_versions;
use crate::cli::version::replace_pesde_bin_exe;
use crate::util::no_build_metadata;
use clap::Args;

#[derive(Debug, Args)]
pub struct SelfUpgradeCommand {
	/// Whether to include pre-releases
	#[clap(long, default_value_t = false)]
	include_pre: bool,
}

impl SelfUpgradeCommand {
	pub async fn run(
		self,
		project: pesde::Project,
		reqwest: reqwest::Client,
	) -> anyhow::Result<()> {
		let Some(latest_version) = query_versions(&reqwest, project.auth_config())
			.await?
			.map(|(version, _)| version)
			.filter(|version| self.include_pre || version.pre.is_empty())
			.max()
		else {
			eprintln!("no releases found");
			return Ok(());
		};

		let latest_version_no_metadata = no_build_metadata(&latest_version);

		if latest_version_no_metadata <= current_version() {
			println!("already up to date");
			return Ok(());
		}

		let display_latest_version = ADDED_STYLE.apply_to(latest_version_no_metadata);

		let confirmed = inquire::prompt_confirmation(format!(
			"are you sure you want to upgrade {} from {} to {display_latest_version}?",
			CLI_STYLE.apply_to(env!("CARGO_BIN_NAME")),
			REMOVED_STYLE.apply_to(env!("CARGO_PKG_VERSION"))
		))?;
		if !confirmed {
			println!("cancelled upgrade");
			return Ok(());
		}

		let path = run_with_reporter(|_, root_progress, reporter| async {
			let root_progress = root_progress;

			root_progress.reset();
			root_progress.set_message("download");

			get_or_download_version(
				&reqwest,
				&format!("={latest_version}").parse().unwrap(),
				reporter,
				project.auth_config(),
			)
			.await
		})
		.await?;

		replace_pesde_bin_exe(&path).await?;

		println!("upgraded to version {display_latest_version}!");

		Ok(())
	}
}
