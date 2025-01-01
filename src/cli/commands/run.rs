use crate::cli::up_to_date_lockfile;
use anyhow::Context;
use clap::Args;
use futures::{StreamExt, TryStreamExt};
use pesde::{
    errors::{ManifestReadError, WorkspaceMembersError},
    linking::generator::generate_bin_linking_module,
    names::{PackageName, PackageNames},
    source::traits::{GetTargetOptions, PackageRef, PackageSource, RefreshOptions},
    Project, MANIFEST_FILE_NAME, PACKAGES_CONTAINER_NAME,
};
use relative_path::RelativePathBuf;
use std::{
    collections::HashSet, env::current_dir, ffi::OsString, io::Write, path::Path, process::Command,
    sync::Arc,
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
        let run = |root: &Path, file_path: &Path| {
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
                    project.package_dir(),
                    &script_path.to_path(project.package_dir()),
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

            let mut versions = graph
                .into_iter()
                .filter(|(id, node)| *id.name() == pkg_name && node.direct.is_some())
                .collect::<Vec<_>>();

            let (id, node) = match versions.len() {
                0 => anyhow::bail!("package not found"),
                1 => versions.pop().unwrap(),
                _ => anyhow::bail!("multiple versions found. use the package's alias instead."),
            };

            let base_folder = project
                .deser_manifest()
                .await?
                .target
                .kind()
                .packages_folder(id.version_id().target());
            let container_folder = node.container_folder(
                &project
                    .package_dir()
                    .join(base_folder)
                    .join(PACKAGES_CONTAINER_NAME),
                &id,
            );

            let source = node.pkg_ref.source();
            source
                .refresh(&RefreshOptions {
                    project: project.clone(),
                })
                .await
                .context("failed to refresh source")?;
            let target = source
                .get_target(
                    &node.pkg_ref,
                    &GetTargetOptions {
                        project,
                        path: Arc::from(container_folder.as_path()),
                    },
                )
                .await?;

            let Some(bin_path) = target.bin_path() else {
                anyhow::bail!("package has no bin path");
            };

            let path = bin_path.to_path(&container_folder);

            run(&path, &path);
            return Ok(());
        }

        if let Ok(manifest) = project.deser_manifest().await {
            if let Some(script_path) = manifest.scripts.get(&package_or_script) {
                run(
                    project.package_dir(),
                    &script_path.to_path(project.package_dir()),
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

        let members = match project.workspace_members(false).await {
            Ok(members) => members.boxed(),
            Err(WorkspaceMembersError::ManifestParse(ManifestReadError::Io(e)))
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

        run(&root, &path);

        Ok(())
    }
}
