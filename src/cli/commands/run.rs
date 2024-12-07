use crate::cli::up_to_date_lockfile;
use anyhow::Context;
use clap::Args;
use futures::{StreamExt, TryStreamExt};
use pesde::{
    linking::generator::generate_bin_linking_module,
    names::{PackageName, PackageNames},
    Project, MANIFEST_FILE_NAME, PACKAGES_CONTAINER_NAME,
};
use relative_path::RelativePathBuf;
use std::{
    collections::HashSet, env::current_dir, ffi::OsString, io::Write, path::PathBuf,
    process::Command,
};

#[derive(Debug, Args)]
pub struct RunCommand {
    /// The package name, script name, or path to a script to run
    #[arg(index = 1)]
    package_or_script: Option<String>,

    /// Arguments to pass to the script
    #[arg(index = 2, last = true)]
    args: Vec<OsString>,
}

impl RunCommand {
    pub async fn run(self, project: Project) -> anyhow::Result<()> {
        let run = |root: PathBuf, file_path: PathBuf| {
            let mut caller = tempfile::NamedTempFile::new().expect("failed to create tempfile");
            caller
                .write_all(
                    generate_bin_linking_module(
                        root,
                        &format!("{:?}", file_path.to_string_lossy()),
                    )
                    .as_bytes(),
                )
                .expect("failed to write to tempfile");

            let status = Command::new("lune")
                .arg("run")
                .arg(caller.path())
                .arg("--")
                .args(&self.args)
                .current_dir(current_dir().expect("failed to get current directory"))
                .status()
                .expect("failed to run script");

            drop(caller);

            std::process::exit(status.code().unwrap_or(1))
        };

        let Some(package_or_script) = self.package_or_script else {
            if let Some(script_path) = project.deser_manifest().await?.target.bin_path() {
                run(
                    project.package_dir().to_owned(),
                    script_path.to_path(project.package_dir()),
                );
                return Ok(());
            }

            anyhow::bail!("no package or script specified, and no bin path found in manifest")
        };

        if let Ok(pkg_name) = package_or_script.parse::<PackageName>() {
            let graph = if let Some(lockfile) = up_to_date_lockfile(&project).await? {
                lockfile.graph
            } else {
                anyhow::bail!("outdated lockfile, please run the install command first")
            };

            let pkg_name = PackageNames::Pesde(pkg_name);

            for (version_id, node) in graph.get(&pkg_name).context("package not found in graph")? {
                if node.node.direct.is_none() {
                    continue;
                }

                let Some(bin_path) = node.target.bin_path() else {
                    anyhow::bail!("package has no bin path");
                };

                let base_folder = project
                    .deser_manifest()
                    .await?
                    .target
                    .kind()
                    .packages_folder(version_id.target());
                let container_folder = node.node.container_folder(
                    &project
                        .package_dir()
                        .join(base_folder)
                        .join(PACKAGES_CONTAINER_NAME),
                    &pkg_name,
                    version_id.version(),
                );

                let path = bin_path.to_path(&container_folder);

                run(path.clone(), path);
                return Ok(());
            }
        }

        if let Ok(manifest) = project.deser_manifest().await {
            if let Some(script_path) = manifest.scripts.get(&package_or_script) {
                run(
                    project.package_dir().to_path_buf(),
                    script_path.to_path(project.package_dir()),
                );
                return Ok(());
            }
        };

        let relative_path = RelativePathBuf::from(package_or_script);
        let path = relative_path.to_path(project.package_dir());

        if !path.exists() {
            anyhow::bail!("path `{}` does not exist", path.display());
        }

        let workspace_dir = project
            .workspace_dir()
            .unwrap_or_else(|| project.package_dir());

        let members = match project.workspace_members(workspace_dir).await {
            Ok(members) => members.boxed(),
            Err(pesde::errors::WorkspaceMembersError::ManifestMissing(e))
                if e.kind() == std::io::ErrorKind::NotFound =>
            {
                futures::stream::empty().boxed()
            }
            Err(e) => Err(e).context("failed to get workspace members")?,
        };

        let members = members
            .map(|res| {
                res.map_err(anyhow::Error::from)
                    .and_then(|(path, _)| path.canonicalize().map_err(Into::into))
            })
            .chain(futures::stream::once(async {
                workspace_dir.canonicalize().map_err(Into::into)
            }))
            .try_collect::<HashSet<_>>()
            .await
            .context("failed to collect workspace members")?;

        let root = 'finder: {
            let mut current_path = path.to_path_buf();
            loop {
                let canonical_path = current_path
                    .canonicalize()
                    .context("failed to canonicalize parent")?;

                if members.contains(&canonical_path)
                    && canonical_path.join(MANIFEST_FILE_NAME).exists()
                {
                    break 'finder canonical_path;
                }

                if let Some(parent) = current_path.parent() {
                    current_path = parent.to_path_buf();
                } else {
                    break;
                }
            }

            project.package_dir().to_path_buf()
        };

        run(root, path);

        Ok(())
    }
}
