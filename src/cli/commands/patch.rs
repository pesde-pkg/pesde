use std::sync::Arc;

use crate::cli::{up_to_date_lockfile, VersionedPackageName};
use anyhow::Context;
use clap::Args;
use colored::Colorize;
use fs_err::tokio as fs;
use pesde::{
    patches::setup_patches_repo,
    source::{
        refs::PackageRefs,
        traits::{PackageRef, PackageSource},
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

        let (name, version_id) = self.package.get(&graph)?;

        let node = graph
            .get(&name)
            .and_then(|versions| versions.get(&version_id))
            .context("package not found in graph")?;

        if matches!(node.node.pkg_ref, PackageRefs::Workspace(_)) {
            anyhow::bail!("cannot patch a workspace package")
        }

        let source = node.node.pkg_ref.source();

        let directory = project
            .data_dir()
            .join("patches")
            .join(name.escaped())
            .join(version_id.escaped())
            .join(chrono::Utc::now().timestamp().to_string());
        fs::create_dir_all(&directory).await?;

        source
            .download(&node.node.pkg_ref, &project, &reqwest, Arc::new(()))
            .await?
            .0
            .write_to(&directory, project.cas_dir(), false)
            .await
            .context("failed to write package contents")?;

        setup_patches_repo(&directory)?;

        println!(
            concat!(
                "done! modify the files in the directory, then run `",
                env!("CARGO_BIN_NAME"),
                r#" patch-commit {}` to apply.
{}: do not commit these changes
{}: the {} file will be ignored when patching"#
            ),
            directory.display().to_string().bold().cyan(),
            "warning".yellow(),
            "note".blue(),
            MANIFEST_FILE_NAME
        );

        open::that(directory)?;

        Ok(())
    }
}
