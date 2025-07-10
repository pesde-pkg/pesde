use crate::cli::{
	compatible_runtime,
	config::read_config,
	get_project_engines,
	reporters::{self, CliReporter},
	VersionedPackageName,
};
use anyhow::Context as _;
use clap::Args;
use console::style;
use fs_err::tokio as fs;
use indicatif::MultiProgress;
use pesde::{
	download_and_link::{DownloadAndLinkOptions, InstallDependenciesMode},
	linking::generator::{generate_bin_linking_module, get_bin_require_path},
	manifest::target::TargetKind,
	names::{PackageName, PackageNames},
	source::{
		ids::PackageId,
		pesde::{specifier::PesdeDependencySpecifier, PesdePackageSource},
		traits::{DownloadOptions, PackageSource as _, RefreshOptions, ResolveOptions},
		PackageSources,
	},
	Project, RefreshedSources, DEFAULT_INDEX_NAME,
};
use semver::VersionReq;
use std::{
	env::current_dir,
	ffi::OsString,
	io::{Stderr, Write as _},
	sync::Arc,
};

#[derive(Debug, Args)]
pub struct ExecuteCommand {
	/// The package to run
	#[arg(index = 1)]
	package: VersionedPackageName<VersionReq, PackageName>,

	/// The target of the package to run
	#[arg(short, long, default_value_t = TargetKind::Luau)]
	target: TargetKind,

	/// The index URL to use for the package
	#[arg(short, long, value_parser = crate::cli::parse_gix_url)]
	index: Option<gix::Url>,

	/// Arguments to pass to the script
	#[arg(index = 2, last = true)]
	args: Vec<OsString>,
}

impl ExecuteCommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		if !self.target.has_bin() {
			anyhow::bail!("{} doesn't support bin exports!", self.target);
		}

		let multi_progress = MultiProgress::new();
		crate::PROGRESS_BARS
			.lock()
			.unwrap()
			.replace(multi_progress.clone());

		let refreshed_sources = RefreshedSources::new();

		let (tempdir, runtime, bin_path) = reporters::run_with_reporter_and_writer(
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
				refreshed_sources
					.refresh(
						&PackageSources::Pesde(source.clone()),
						&RefreshOptions {
							project: project.clone(),
						},
					)
					.await
					.context("failed to refresh source")?;

				let version_req = self.package.1.unwrap_or(VersionReq::STAR);
				let Some((v_id, pkg_ref)) = source
					.resolve(
						&PesdeDependencySpecifier {
							name: self.package.0.clone(),
							version: version_req.clone(),
							index: DEFAULT_INDEX_NAME.into(),
							target: None,
						},
						&ResolveOptions {
							project: project.clone(),
							target: self.target,
							refreshed_sources: refreshed_sources.clone(),
							loose_target: true,
						},
					)
					.await
					.context("failed to resolve package")?
					.1
					.pop_last()
				else {
					anyhow::bail!(
						"no compatible package could be found for {}@{version_req}",
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

				let name = self.package.0.clone();
				let mut file = source
					.read_index_file(&name, &project)
					.await
					.context("failed to read package index file")?
					.context("package doesn't exist on the index")?;

				let entry = file
					.entries
					.remove(&v_id)
					.context("version id not present in index file")?;

				let bin_path = entry
					.target
					.bin_path()
					.context("package has no binary export")?;

				let id = Arc::new(PackageId::new(PackageNames::Pesde(name), v_id));

				let fs = source
					.download(
						&pkg_ref,
						&DownloadOptions {
							project: project.clone(),
							reqwest: reqwest.clone(),
							reporter: ().into(),
							id: id.clone(),
						},
					)
					.await
					.context("failed to download package")?;

				fs.write_to(tempdir.path(), project.cas_dir(), true)
					.await
					.context("failed to write package contents")?;

				let graph = project
					.dependency_graph(None, refreshed_sources.clone(), true)
					.await
					.context("failed to build dependency graph")?;

				multi_progress.suspend(|| {
					eprintln!("{}", style(format!("using {}", style(id).bold())).dim());
				});

				root_progress.reset();
				root_progress.set_message("download");
				root_progress.set_style(reporters::root_progress_style_with_progress());

				project
					.download_and_link(
						&graph,
						DownloadAndLinkOptions::<CliReporter<Stderr>, ()>::new(reqwest.clone())
							.reporter(reporter)
							.refreshed_sources(refreshed_sources)
							.install_dependencies_mode(InstallDependenciesMode::Prod),
					)
					.await
					.context("failed to download and link dependencies")?;

				let manifest = project
					.deser_manifest()
					.await
					.context("failed to deserialize manifest")?;

				let engines =
					get_project_engines(&manifest, &reqwest, project.auth_config()).await?;

				anyhow::Ok((
					tempdir,
					compatible_runtime(entry.target.kind(), &engines)?,
					bin_path.to_relative_path_buf(),
				))
			},
		)
		.await?;

		let mut caller =
			tempfile::NamedTempFile::new_in(tempdir.path()).context("failed to create tempfile")?;
		caller
			.write_all(
				generate_bin_linking_module(
					tempdir.path(),
					&get_bin_require_path(tempdir.path(), &bin_path, tempdir.path()),
				)
				.as_bytes(),
			)
			.context("failed to write to tempfile")?;

		let status = runtime
			.prepare_command(caller.path().as_os_str(), self.args)
			.current_dir(current_dir().context("failed to get current directory")?)
			.status()
			.await
			.context("failed to run script")?;

		drop(caller);
		drop(tempdir);

		std::process::exit(status.code().unwrap_or(1i32))
	}
}
