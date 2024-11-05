use anyhow::Context;
use colored::Colorize;
use fs_err::tokio as fs;
use futures::StreamExt;
use indicatif::MultiProgress;
use pesde::{
    lockfile::{DependencyGraph, DownloadedGraph, Lockfile},
    manifest::target::TargetKind,
    names::{PackageName, PackageNames},
    source::{version_id::VersionId, workspace::specifier::VersionTypeOrReq, PackageSources},
    Project,
};
use relative_path::RelativePathBuf;
use std::{
    collections::{BTreeMap, HashSet},
    future::Future,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use tokio::pin;

pub mod auth;
pub mod commands;
pub mod config;
pub mod files;
pub mod repos;
#[cfg(feature = "version-management")]
pub mod version;

pub const HOME_DIR: &str = concat!(".", env!("CARGO_PKG_NAME"));

pub fn home_dir() -> anyhow::Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("failed to get home directory")?
        .join(HOME_DIR))
}

pub async fn bin_dir() -> anyhow::Result<PathBuf> {
    let bin_dir = home_dir()?.join("bin");
    fs::create_dir_all(&bin_dir)
        .await
        .context("failed to create bin folder")?;
    Ok(bin_dir)
}

pub async fn up_to_date_lockfile(project: &Project) -> anyhow::Result<Option<Lockfile>> {
    let manifest = project.deser_manifest().await?;
    let lockfile = match project.deser_lockfile().await {
        Ok(lockfile) => lockfile,
        Err(pesde::errors::LockfileReadError::Io(e))
            if e.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };

    if manifest.overrides != lockfile.overrides {
        log::debug!("overrides are different");
        return Ok(None);
    }

    if manifest.target.kind() != lockfile.target {
        log::debug!("target kind is different");
        return Ok(None);
    }

    if manifest.name != lockfile.name || manifest.version != lockfile.version {
        log::debug!("name or version is different");
        return Ok(None);
    }

    let specs = lockfile
        .graph
        .iter()
        .flat_map(|(_, versions)| versions)
        .filter_map(|(_, node)| {
            node.node
                .direct
                .as_ref()
                .map(|(_, spec)| (spec, node.node.ty))
        })
        .collect::<HashSet<_>>();

    let same_dependencies = manifest
        .all_dependencies()
        .context("failed to get all dependencies")?
        .iter()
        .all(|(_, (spec, ty))| specs.contains(&(spec, *ty)));

    log::debug!("dependencies are the same: {same_dependencies}");

    Ok(if same_dependencies {
        Some(lockfile)
    } else {
        None
    })
}

#[derive(Debug, Clone)]
struct VersionedPackageName<V: FromStr = VersionId, N: FromStr = PackageNames>(N, Option<V>);

impl<V: FromStr<Err = E>, E: Into<anyhow::Error>, N: FromStr<Err = F>, F: Into<anyhow::Error>>
    FromStr for VersionedPackageName<V, N>
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, '@');
        let name = parts.next().unwrap();
        let version = parts
            .next()
            .map(FromStr::from_str)
            .transpose()
            .map_err(Into::into)?;

        Ok(VersionedPackageName(
            name.parse().map_err(Into::into)?,
            version,
        ))
    }
}

impl VersionedPackageName {
    #[cfg(feature = "patches")]
    fn get(self, graph: &DownloadedGraph) -> anyhow::Result<(PackageNames, VersionId)> {
        let version_id = match self.1 {
            Some(version) => version,
            None => {
                let versions = graph.get(&self.0).context("package not found in graph")?;
                if versions.len() == 1 {
                    let version = versions.keys().next().unwrap().clone();
                    log::debug!("only one version found, using {version}");
                    version
                } else {
                    anyhow::bail!(
                        "multiple versions found, please specify one of: {}",
                        versions
                            .keys()
                            .map(|v| v.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        };

        Ok((self.0, version_id))
    }
}

#[derive(Debug, Clone)]
enum AnyPackageIdentifier<V: FromStr = VersionId, N: FromStr = PackageNames> {
    PackageName(VersionedPackageName<V, N>),
    Url((gix::Url, String)),
    Workspace(VersionedPackageName<VersionTypeOrReq, PackageName>),
}

impl<V: FromStr<Err = E>, E: Into<anyhow::Error>, N: FromStr<Err = F>, F: Into<anyhow::Error>>
    FromStr for AnyPackageIdentifier<V, N>
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(s) = s.strip_prefix("gh#") {
            let s = format!("https://github.com/{s}");
            let (repo, rev) = s.split_once('#').context("missing revision")?;

            Ok(AnyPackageIdentifier::Url((
                repo.try_into()?,
                rev.to_string(),
            )))
        } else if let Some(rest) = s.strip_prefix("workspace:") {
            Ok(AnyPackageIdentifier::Workspace(rest.parse()?))
        } else if s.contains(':') {
            let (url, rev) = s.split_once('#').context("missing revision")?;

            Ok(AnyPackageIdentifier::Url((
                url.try_into()?,
                rev.to_string(),
            )))
        } else {
            Ok(AnyPackageIdentifier::PackageName(s.parse()?))
        }
    }
}

pub fn parse_gix_url(s: &str) -> Result<gix::Url, gix::url::parse::Error> {
    s.try_into()
}

#[allow(clippy::too_many_arguments)]
pub async fn download_graph(
    project: &Project,
    refreshed_sources: &mut HashSet<PackageSources>,
    graph: &DependencyGraph,
    multi: &MultiProgress,
    reqwest: &reqwest::Client,
    prod: bool,
    write: bool,
    progress_msg: String,
    finish_msg: String,
) -> anyhow::Result<DownloadedGraph> {
    let bar = multi.add(
        indicatif::ProgressBar::new(graph.values().map(|versions| versions.len() as u64).sum())
            .with_style(
                indicatif::ProgressStyle::default_bar()
                    .template("{msg} {bar:40.208/166} {pos}/{len} {percent}% {elapsed_precise}")?,
            )
            .with_message(progress_msg),
    );
    bar.enable_steady_tick(Duration::from_millis(100));

    let (mut rx, downloaded_graph) = project
        .download_graph(graph, refreshed_sources, reqwest, prod, write)
        .await
        .context("failed to download dependencies")?;

    while let Some(result) = rx.recv().await {
        bar.inc(1);

        match result {
            Ok(()) => {}
            Err(e) => return Err(e.into()),
        }
    }

    bar.finish_with_message(finish_msg);

    Ok(Arc::into_inner(downloaded_graph)
        .unwrap()
        .into_inner()
        .unwrap())
}

pub fn shift_project_dir(project: &Project, pkg_dir: PathBuf) -> Project {
    Project::new(
        pkg_dir,
        Some(project.package_dir()),
        project.data_dir(),
        project.cas_dir(),
        project.auth_config().clone(),
    )
}

pub async fn run_on_workspace_members<F: Future<Output = anyhow::Result<()>>>(
    project: &Project,
    f: impl Fn(Project) -> F,
) -> anyhow::Result<BTreeMap<PackageName, BTreeMap<TargetKind, RelativePathBuf>>> {
    // this might seem counterintuitive, but remember that
    // the presence of a workspace dir means that this project is a member of one
    if project.workspace_dir().is_some() {
        return Ok(Default::default());
    }

    let members_future = project.workspace_members(project.package_dir()).await?;
    pin!(members_future);

    let mut results = BTreeMap::<PackageName, BTreeMap<TargetKind, RelativePathBuf>>::new();

    while let Some((path, manifest)) = members_future.next().await.transpose()? {
        let relative_path =
            RelativePathBuf::from_path(path.strip_prefix(project.package_dir()).unwrap()).unwrap();

        f(shift_project_dir(project, path)).await?;

        results
            .entry(manifest.name)
            .or_default()
            .insert(manifest.target.kind(), relative_path);
    }

    Ok(results)
}

pub fn display_err(result: anyhow::Result<()>, prefix: &str) {
    if let Err(err) = result {
        eprintln!("{}: {err}\n", format!("error{prefix}").red().bold());

        let cause = err.chain().skip(1).collect::<Vec<_>>();

        if !cause.is_empty() {
            eprintln!("{}:", "caused by".red().bold());
            for err in cause {
                eprintln!("  - {err}");
            }
        }

        let backtrace = err.backtrace();
        match backtrace.status() {
            std::backtrace::BacktraceStatus::Disabled => {
                eprintln!(
                    "\n{}: set RUST_BACKTRACE=1 for a backtrace",
                    "help".yellow().bold()
                );
            }
            std::backtrace::BacktraceStatus::Captured => {
                eprintln!("\n{}:\n{backtrace}", "backtrace".yellow().bold());
            }
            _ => {
                eprintln!("\n{}: not captured", "backtrace".yellow().bold());
            }
        }
    }
}
