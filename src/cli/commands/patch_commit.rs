use crate::cli::up_to_date_lockfile;
use anyhow::Context as _;
use clap::Args;
use fs_err::tokio as fs;
use pesde::{Project, patches::create_patch};
use std::{path::PathBuf, str::FromStr as _};

#[derive(Debug, Args)]
pub struct PatchCommitCommand {
	/// The directory containing the patch to commit
	#[arg(index = 1)]
	directory: PathBuf,
}

impl PatchCommitCommand {
	pub async fn run(self, project: Project) -> anyhow::Result<()> {
		let graph = if let Some(lockfile) = up_to_date_lockfile(&project).await? {
			lockfile.graph
		} else {
			anyhow::bail!("outdated lockfile, please run the install command first")
		};

		let id = self
			.directory
			.parent()
			.context("directory has no parent")?
			.file_name()
			.context("directory parent has no name")?
			.to_str()
			.context("directory parent name is not valid")?
			.parse()
			.context("failed to parse package id")?;

		graph.get(&id).context("package not found in graph")?;

		let mut manifest = toml_edit::DocumentMut::from_str(
			&project
				.read_manifest()
				.await
				.context("failed to read manifest")?,
		)
		.context("failed to parse manifest")?;

		let patch = create_patch(&self.directory).context("failed to create patch")?;

		let patches_dir = project.package_dir().join("patches");
		fs::create_dir_all(&patches_dir)
			.await
			.context("failed to create patches directory")?;

		let patch_file_name = format!("{id}.patch");

		let patch_file = patches_dir.join(&patch_file_name);

		fs::write(&patch_file, patch)
			.await
			.context("failed to write patch file")?;

		manifest["patches"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
			[&id.to_string()] = toml_edit::value(format!("patches/{patch_file_name}"));

		project
			.write_manifest(manifest.to_string())
			.await
			.context("failed to write manifest")?;

		fs::remove_dir_all(self.directory)
			.await
			.context("failed to remove patch directory")?;

		println!(concat!(
			"done! run `",
			env!("CARGO_BIN_NAME"),
			" install` to apply the patch"
		));

		Ok(())
	}
}
