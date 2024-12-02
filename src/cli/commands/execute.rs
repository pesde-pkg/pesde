use crate::cli::{config::read_config, progress_bar, VersionedPackageName};
use anyhow::Context;
use clap::Args;
use fs_err::tokio as fs;
use indicatif::MultiProgress;
use pesde::{
    linking::generator::generate_bin_linking_module,
    manifest::target::TargetKind,
    names::PackageName,
    source::{
        pesde::{specifier::PesdeDependencySpecifier, PesdePackageSource},
        traits::PackageSource,
    },
    Project,
};
use semver::VersionReq;
use std::{
    collections::HashSet, env::current_dir, ffi::OsString, io::Write, process::Command, sync::Arc,
};
use tokio::sync::Mutex;

#[derive(Debug, Args)]
pub struct ExecuteCommand {
    /// The package name, script name, or path to a script to run
    #[arg(index = 1)]
    package: VersionedPackageName<VersionReq, PackageName>,

    /// The index URL to use for the package
    #[arg(short, long, value_parser = crate::cli::parse_gix_url)]
    index: Option<gix::Url>,

    /// Arguments to pass to the script
    #[arg(index = 2, last = true)]
    args: Vec<OsString>,
}

impl ExecuteCommand {
    pub async fn run(
        self,
        project: Project,
        multi: MultiProgress,
        reqwest: reqwest::Client,
    ) -> anyhow::Result<()> {
        let index = match self.index {
            Some(index) => Some(index),
            None => read_config().await.ok().map(|c| c.default_index),
        }
        .context("no index specified")?;
        let source = PesdePackageSource::new(index);
        source
            .refresh(&project)
            .await
            .context("failed to refresh source")?;

        let version_req = self.package.1.unwrap_or(VersionReq::STAR);
        let Some((version, pkg_ref)) = ('finder: {
            let specifier = PesdeDependencySpecifier {
                name: self.package.0.clone(),
                version: version_req.clone(),
                index: None,
                target: None,
            };

            if let Some(res) = source
                .resolve(&specifier, &project, TargetKind::Lune, &mut HashSet::new())
                .await
                .context("failed to resolve package")?
                .1
                .pop_last()
            {
                break 'finder Some(res);
            }

            source
                .resolve(&specifier, &project, TargetKind::Luau, &mut HashSet::new())
                .await
                .context("failed to resolve package")?
                .1
                .pop_last()
        }) else {
            anyhow::bail!(
                "no Lune or Luau package could be found for {}@{version_req}",
                self.package.0,
            );
        };

        log::info!("found package {}@{version}", pkg_ref.name);

        let tmp_dir = project.cas_dir().join(".tmp");
        fs::create_dir_all(&tmp_dir)
            .await
            .context("failed to create temporary directory")?;
        let tempdir =
            tempfile::tempdir_in(tmp_dir).context("failed to create temporary directory")?;

        let project = Project::new(
            tempdir.path(),
            None::<std::path::PathBuf>,
            project.data_dir(),
            project.cas_dir(),
            project.auth_config().clone(),
        );

        let (fs, target) = source
            .download(&pkg_ref, &project, &reqwest)
            .await
            .context("failed to download package")?;
        let bin_path = target.bin_path().context("package has no binary export")?;

        fs.write_to(tempdir.path(), project.cas_dir(), true)
            .await
            .context("failed to write package contents")?;

        let mut refreshed_sources = HashSet::new();

        let graph = project
            .dependency_graph(None, &mut refreshed_sources, true)
            .await
            .context("failed to build dependency graph")?;
        let graph = Arc::new(graph);

        let (rx, downloaded_graph) = project
            .download_and_link(
                &graph,
                &Arc::new(Mutex::new(refreshed_sources)),
                &reqwest,
                true,
                true,
                |_| async { Ok::<_, std::io::Error>(()) },
            )
            .await
            .context("failed to download dependencies")?;

        progress_bar(
            graph.values().map(|versions| versions.len() as u64).sum(),
            rx,
            &multi,
            "📥 ".to_string(),
            "downloading dependencies".to_string(),
            "downloaded dependencies".to_string(),
        )
        .await?;

        downloaded_graph
            .await
            .context("failed to download & link dependencies")?;

        let mut caller =
            tempfile::NamedTempFile::new_in(tempdir.path()).context("failed to create tempfile")?;
        caller
            .write_all(
                generate_bin_linking_module(
                    tempdir.path(),
                    &format!("{:?}", bin_path.to_path(tempdir.path())),
                )
                .as_bytes(),
            )
            .context("failed to write to tempfile")?;

        let status = Command::new("lune")
            .arg("run")
            .arg(caller.path())
            .arg("--")
            .args(&self.args)
            .current_dir(current_dir().context("failed to get current directory")?)
            .status()
            .context("failed to run script")?;

        drop(caller);
        drop(tempdir);

        std::process::exit(status.code().unwrap_or(1))
    }
}
