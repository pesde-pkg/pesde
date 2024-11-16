use crate::cli::{repos::update_scripts, up_to_date_lockfile};
use anyhow::Context;
use clap::Args;
use pesde::{
    linking::generator::generate_bin_linking_module,
    names::{PackageName, PackageNames},
    Project, PACKAGES_CONTAINER_NAME,
};
use relative_path::RelativePathBuf;
use std::{env::current_dir, ffi::OsString, io::Write, path::PathBuf, process::Command};

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
        let run = |path: PathBuf| {
            let package_dir = project.package_dir().to_path_buf();
            let fut = update_scripts(&project);
            async move {
                fut.await.expect("failed to update scripts");

                let mut caller = tempfile::NamedTempFile::new().expect("failed to create tempfile");
                caller
                    .write_all(
                        generate_bin_linking_module(
                            package_dir,
                            &format!("{:?}", path.to_string_lossy()),
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
            }
        };

        let Some(package_or_script) = self.package_or_script else {
            if let Some(script_path) = project.deser_manifest().await?.target.bin_path() {
                run(script_path.to_path(project.package_dir())).await;
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

                run(bin_path.to_path(&container_folder)).await;
                return Ok(());
            }
        }

        if let Ok(manifest) = project.deser_manifest().await {
            if let Some(script_path) = manifest.scripts.get(&package_or_script) {
                run(script_path.to_path(project.package_dir())).await;
                return Ok(());
            }
        };

        let relative_path = RelativePathBuf::from(package_or_script);
        let path = relative_path.to_path(project.package_dir());

        if !path.exists() {
            anyhow::bail!("path `{}` does not exist", path.display());
        }

        run(path).await;

        Ok(())
    }
}
