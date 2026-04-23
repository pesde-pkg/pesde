use crate::cli::AnyPackageIdentifier;
use crate::cli::config::read_config;
use crate::cli::reporters;
use crate::cli::reporters::CliReporter;
use anyhow::Context as _;
use clap::Args;
use console::style;
use indicatif::MultiProgress;
use pesde::Importer;
use pesde::Project;
use pesde::RefreshedSources;
use pesde::Subproject;
use pesde::download_and_link::DownloadAndLinkOptions;
use pesde::download_and_link::InstallDependenciesMode;
use pesde::scripts::execute_script;
use pesde::source::PackageSource as _;
use pesde::source::ResolveResult;
use pesde::source::ResolvedPackage;
use pesde::source::ids::PackageId;
use std::ffi::OsString;
use std::io::Stderr;
use tempfile::TempDir;

#[derive(Debug, Args)]
pub struct ExecuteCommand {
	/// The command to execute the binary export with (the path to the binary export will be passed as the first argument)
	#[arg(index = 1)]
	command: String,

	/// The package to run
	#[arg(index = 2)]
	package: AnyPackageIdentifier,

	/// Arguments to pass to the script
	#[arg(index = 3, trailing_var_arg = true, allow_hyphen_values = true)]
	args: Vec<OsString>,
}

impl ExecuteCommand {
	pub async fn run(mut self, subproject: Subproject) -> anyhow::Result<()> {
		let multi_progress = MultiProgress::new();
		crate::PROGRESS_BARS
			.lock()
			.unwrap()
			.replace(multi_progress.clone());

		let (project, tempdir, bin_file) = reporters::run_with_reporter_and_writer(
			std::io::stderr(),
			|multi_progress, root_progress, reporter| async {
				let multi_progress = multi_progress;
				let root_progress = root_progress;

				root_progress.set_message("resolve");

				let (source, specifier) = self
					.package
					.source_and_specifier(None, async |_| {
						let index = read_config().await?.default_index;
						Ok((index.to_string(), index))
					})
					.await
					.context("failed to parse package identifier")?;

				let refreshed_sources = RefreshedSources::new();
				refreshed_sources
					.refresh(&source, subproject.project())
					.await
					.context("failed to refresh source")?;

				let ResolveResult {
					source,
					pkg_ref,
					structure_kind,
					mut versions,
				} = source
					.resolve(&subproject, &specifier, &refreshed_sources)
					.await
					.context("failed to resolve package")?;

				let Some((version, _)) = versions.pop_last() else {
					anyhow::bail!("no compatible package could be found");
				};

				if structure_kind.is_wally() {
					anyhow::bail!("executing binaries from wally packages is not supported");
				}

				let package = ResolvedPackage {
					id: PackageId::new(source, pkg_ref, version),
					structure_kind,
				};

				multi_progress.suspend(|| {
					eprintln!(
						"{}",
						style(format_args!("using {}", style(&package.id).bold())).dim()
					);
				});

				root_progress.reset();
				root_progress.set_message("download");
				root_progress.set_style(reporters::root_progress_style_with_progress());

				let tempdir = TempDir::new_in(subproject.project().cas_dir().join(".tmp"))
					.context("failed to create temporary directory")?;

				let fs = package
					.id
					.source()
					.download(subproject.project(), &package, ().into())
					.await
					.context("failed to download package")?;

				fs.write_to(tempdir.path(), subproject.project().cas_dir(), true)
					.await
					.context("failed to write package contents")?;

				let exports = package
					.id
					.source()
					.get_exports(subproject.project(), &package, tempdir.path())
					.await
					.context("failed to get package exports")?;

				let project = Project::new(
					tempdir.path(),
					subproject.project().data_dir(),
					subproject.project().cas_dir(),
					subproject.project().auth_config().clone(),
					subproject.project().reqwest().clone(),
				);

				let graph = project
					.dependency_graph(None, &refreshed_sources, true)
					.await
					.context("failed to build dependency graph")?
					.0;

				project
					.download_and_link(
						&graph,
						DownloadAndLinkOptions::<CliReporter<Stderr>>::new()
							.reporter(reporter)
							.refreshed_sources(refreshed_sources)
							.install_dependencies_mode(InstallDependenciesMode::Prod),
					)
					.await
					.context("failed to download and link dependencies")?;

				Ok((
					project,
					tempdir,
					exports.bin_file.context("package has no binary export")?,
				))
			},
		)
		.await?;

		self.args.insert(0, bin_file.into_string().into());

		let code = execute_script(
			&project.subproject(Importer::root()),
			&self.command,
			&mut (),
			self.args,
		)
		.await
		.context("failed to execute script")?;

		// explicitly drop the tempdir before exiting to ensure all files are cleaned up
		drop(tempdir);

		std::process::exit(code);
	}
}
