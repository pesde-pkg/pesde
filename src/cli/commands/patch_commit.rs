use anyhow::Context as _;
use base64::Engine as _;
use clap::Args;
use fs_err::tokio as fs;
use pesde::Importer;
use pesde::Project;
use pesde::RefreshedSources;
use pesde::patches::create_patch;
use pesde::source::ids::PackageId;
use std::path::PathBuf;
use std::str::FromStr as _;

use crate::cli::install::get_graph_loose;

#[derive(Debug, Args)]
pub struct PatchCommitCommand {
	/// The directory containing the patch to commit
	#[arg(index = 1)]
	directory: PathBuf,
}

impl PatchCommitCommand {
	pub async fn run(self, project: Project) -> anyhow::Result<()> {
		let refreshed_sources = RefreshedSources::new();
		let graph = get_graph_loose(&project, &refreshed_sources).await?;

		let id = self
			.directory
			.parent()
			.context("directory has no parent")?
			.file_name()
			.context("directory parent has no name")?
			.to_str()
			.context("directory parent name is not valid")?;
		let id = base64::engine::general_purpose::URL_SAFE_NO_PAD
			.decode(id)
			.context("failed to decode packge id")?;
		let id = std::str::from_utf8(&id).context("failed to parse package id as UTF-8")?;
		let id = PackageId::from_str(id).context("failed to parse package id")?;

		graph.nodes.get(&id).context("package not found in graph")?;

		let mut manifest = toml_edit::DocumentMut::from_str(
			&project
				.clone()
				.subproject(Importer::root())
				.read_manifest()
				.await
				.context("failed to read manifest")?,
		)
		.context("failed to parse manifest")?;

		let patch = create_patch(&self.directory).context("failed to create patch")?;

		let patches_dir = project.dir().join("patches");
		fs::create_dir_all(&patches_dir)
			.await
			.context("failed to create patches directory")?;

		let patch_file_name = format!("{id}.patch");

		let patch_file = patches_dir.join(&patch_file_name);

		fs::write(&patch_file, patch)
			.await
			.context("failed to write patch file")?;

		manifest["workspace"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))["patches"]
			.or_insert(toml_edit::Item::Table(toml_edit::Table::new()))[&id.to_string()] =
			toml_edit::value(format!("patches/{patch_file_name}"));

		project
			.subproject(Importer::root())
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
