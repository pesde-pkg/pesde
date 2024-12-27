use crate::cli::{
    config::read_config,
    reporters::{self, CliReporter},
    VersionedPackageName,
};
use anyhow::Context;
use clap::Args;
use colored::Colorize;
use fs_err::tokio as fs;
use indicatif::MultiProgress;
use pesde::{
    download_and_link::DownloadAndLinkOptions,
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
    collections::HashSet,
    env::current_dir,
    ffi::OsString,
    io::{Stderr, Write},
    process::Command,
    sync::Arc,
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
    pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
        let multi_progress = MultiProgress::new();
        crate::PROGRESS_BARS
            .lock()
            .unwrap()
            .replace(multi_progress.clone());

        let (tempdir, bin_path) = reporters::run_with_reporter_and_writer(
            std::io::stderr(),
            |multi_progress, root_progress, reporter| async {
                let multi_progress = multi_progress;
                let root_progress = root_progress;

                root_progress.set_message("resolve");

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

                let tmp_dir = project.cas_dir().join(".tmp");
                fs::create_dir_all(&tmp_dir)
                    .await
                    .context("failed to create temporary directory")?;
                let tempdir = tempfile::tempdir_in(tmp_dir)
                    .context("failed to create temporary directory")?;

                let project = Project::new(
                    tempdir.path(),
                    None::<std::path::PathBuf>,
                    project.data_dir(),
                    project.cas_dir(),
                    project.auth_config().clone(),
                );

                let (fs, target) = source
                    .download(&pkg_ref, &project, &reqwest, Arc::new(()))
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

                multi_progress.suspend(|| {
                    eprintln!(
                        "{}",
                        format!("using {}", format!("{}@{version}", pkg_ref.name).bold()).dimmed()
                    )
                });

                root_progress.reset();
                root_progress.set_message("download");
                root_progress.set_style(reporters::root_progress_style_with_progress());

                project
                    .download_and_link(
                        &Arc::new(graph),
                        DownloadAndLinkOptions::<CliReporter<Stderr>, ()>::new(reqwest)
                            .reporter(reporter)
                            .refreshed_sources(Mutex::new(refreshed_sources))
                            .prod(true)
                            .write(true),
                    )
                    .await
                    .context("failed to download and link dependencies")?;

                anyhow::Ok((tempdir, bin_path.clone()))
            },
        )
        .await?;

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
