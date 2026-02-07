use crate::cli::style::CLI_STYLE;
use crate::cli::style::INFO_STYLE;
use crate::cli::style::WARN_PREFIX;
use anyhow::Context as _;
use base64::Engine as _;
use clap::Args;
use console::style;
use fs_err::tokio as fs;
use pesde::MANIFEST_FILE_NAME;
use pesde::Project;
use pesde::patches::setup_patches_repo;
use pesde::source::ids::PackageId;
use pesde::source::traits::DownloadOptions;
use pesde::source::traits::PackageSource as _;

#[derive(Debug, Args)]
pub struct PatchCommand {
	/// The package name to patch
	#[arg(index = 1)]
	package: PackageId,
}

impl PatchCommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		if self.package.pkg_ref().is_local() {
			anyhow::bail!("cannot patch a local package")
		}

		let source = self.package.source();
		let directory = project
			.data_dir()
			.join("patches")
			.join(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.package.to_string()))
			.join(jiff::Timestamp::now().as_second().to_string());
		fs::create_dir_all(&directory).await?;

		source
			.download(
				self.package.pkg_ref(),
				&DownloadOptions {
					project: project.clone(),
					reqwest,
					reporter: ().into(),
					version: self.package.version(),
				},
			)
			.await?
			.write_to(&directory, project.cas_dir(), false)
			.await
			.context("failed to write package contents")?;

		setup_patches_repo(&directory)?;

		println!(
			r"done! modify the files in the directory, then run {} {}{} to apply.
{WARN_PREFIX}: do not commit these changes
{}: the {MANIFEST_FILE_NAME} file will be ignored when patching",
			CLI_STYLE.apply_to(concat!("`", env!("CARGO_BIN_NAME"), " patch-commit")),
			style(format!("'{}'", directory.display())).cyan().bold(),
			CLI_STYLE.apply_to("`"),
			INFO_STYLE.apply_to("note")
		);

		open::that(directory)?;

		Ok(())
	}
}
