use crate::cli::up_to_date_lockfile;
use anyhow::Context;
use clap::Args;
use pesde::{names::PackageNames, patches::create_patch, source::version_id::VersionId, Project};
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Args)]
pub struct PatchCommitCommand {
    /// The directory containing the patch to commit
    #[arg(index = 1)]
    directory: PathBuf,
}

impl PatchCommitCommand {
    pub fn run(self, project: Project) -> anyhow::Result<()> {
        let graph = if let Some(lockfile) = up_to_date_lockfile(&project)? {
            lockfile.graph
        } else {
            anyhow::bail!("outdated lockfile, please run the install command first")
        };

        let (name, version_id) = (
            PackageNames::from_escaped(
                self.directory
                    .parent()
                    .context("directory has no parent")?
                    .parent()
                    .context("directory has no grandparent")?
                    .file_name()
                    .context("directory grandparent has no name")?
                    .to_str()
                    .context("directory grandparent name is not valid")?,
            )?,
            VersionId::from_escaped(
                self.directory
                    .parent()
                    .context("directory has no parent")?
                    .file_name()
                    .context("directory parent has no name")?
                    .to_str()
                    .context("directory parent name is not valid")?,
            )?,
        );

        graph
            .get(&name)
            .and_then(|versions| versions.get(&version_id))
            .context("package not found in graph")?;

        let mut manifest = toml_edit::DocumentMut::from_str(
            &project.read_manifest().context("failed to read manifest")?,
        )
        .context("failed to parse manifest")?;

        let patch = create_patch(&self.directory).context("failed to create patch")?;
        fs_err::remove_dir_all(self.directory).context("failed to remove patch directory")?;

        let patches_dir = project.package_dir().join("patches");
        fs_err::create_dir_all(&patches_dir).context("failed to create patches directory")?;

        let patch_file_name = format!("{}-{}.patch", name.escaped(), version_id.escaped());

        let patch_file = patches_dir.join(&patch_file_name);
        if patch_file.exists() {
            anyhow::bail!("patch file already exists: {}", patch_file.display());
        }

        fs_err::write(&patch_file, patch).context("failed to write patch file")?;

        manifest["patches"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            [&name.to_string()][&version_id.to_string()] =
            toml_edit::value(format!("patches/{patch_file_name}"));

        project
            .write_manifest(manifest.to_string())
            .context("failed to write manifest")?;

        println!(concat!(
            "done! run `",
            env!("CARGO_BIN_NAME"),
            " install` to apply the patch"
        ));

        Ok(())
    }
}
