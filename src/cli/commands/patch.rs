use std::sync::Arc;

use crate::cli::{
	style::{CLI_STYLE, INFO_STYLE, WARN_PREFIX},
	up_to_date_lockfile, VersionedPackageName,
};
use anyhow::Context;
use clap::Args;
use console::style;
use fs_err::tokio as fs;
use pesde::{
	patches::setup_patches_repo,
	source::{
		refs::PackageRefs,
		traits::{DownloadOptions, PackageRef, PackageSource},
	},
	Project, MANIFEST_FILE_NAME,
};

#[derive(Debug, Args)]
pub struct PatchCommand {
	/// The package name to patch
	#[arg(index = 1)]
	package: VersionedPackageName,
}

impl PatchCommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let graph = if let Some(lockfile) = up_to_date_lockfile(&project).await? {
			lockfile.graph
		} else {
			anyhow::bail!("outdated lockfile, please run the install command first")
		};

		let id = self.package.get(&graph)?;

		let node = graph.get(&id).context("package not found in graph")?;

		if matches!(
			node.pkg_ref,
			PackageRefs::Workspace(_) | PackageRefs::Path(_)
		) {
			anyhow::bail!("cannot patch a workspace or a path package")
		}

		let source = node.pkg_ref.source();

		let directory = project
			.data_dir()
			.join("patches")
			.join(id.name().escaped())
			.join(id.version_id().escaped())
			.join(jiff::Timestamp::now().as_second().to_string());
		fs::create_dir_all(&directory).await?;

		source
			.download(
				&node.pkg_ref,
				&DownloadOptions {
					project: project.clone(),
					reqwest,
					reporter: Arc::new(()),
					id: Arc::new(id),
				},
			)
			.await?
			.write_to(&directory, project.cas_dir(), false)
			.await
			.context("failed to write package contents")?;

		setup_patches_repo(&directory)?;

		println!(
			r#"done! modify the files in the directory, then run {} {}{} to apply.
{WARN_PREFIX}: do not commit these changes
{}: the {MANIFEST_FILE_NAME} file will be ignored when patching"#,
			CLI_STYLE.apply_to(concat!("`", env!("CARGO_BIN_NAME"), " patch-commit")),
			style(format!("'{}'", directory.display())).cyan().bold(),
			CLI_STYLE.apply_to("`"),
			INFO_STYLE.apply_to("note")
		);

		open::that(directory)?;

		Ok(())
	}
}
