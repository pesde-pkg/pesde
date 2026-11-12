use std::ffi::OsString;
use std::io::Stderr;
use std::num::NonZero;
use std::sync::Arc;

use anyhow::Context as _;
use clap::Args;
use console::style;
use futures::TryStreamExt as _;
use indicatif::MultiProgress;
use pesde::Manifest;
use pesde::Project;
use pesde::bounded::BoundedBTreeMap;
use pesde::install::InstallDependenciesMode;
use pesde::install::InstallOptions;
use pesde::link::PackageStructure;
use pesde::registry::manifest::ManifestPackage;
use pesde::reporters::DownloadsReporter as _;
use pesde::source::DependencySpecifier;
use pesde::source::PackageId;
use pesde::source::PackageIdComparison;
use pesde::vfs::Vfs as _;
use pesde::vfs::VfsPath;
use std::cmp::Ordering;

use crate::cli::reporters;
use crate::cli::reporters::CliReporter;
use crate::cli::scripts::execute_script;
use crate::cli::source::CliSourceProvider;
use crate::cli::ui::CommandContext;
use crate::cli::ui::specifier::CliDependencySpecifier;
use crate::cli::vfs::FsVfs;

/// Executes a one-off binary package
#[derive(Debug, Args)]
#[command(visible_alias = "exec", visible_alias = "x")]
pub struct ExecuteCommand {
	/// The command to execute the binary export with
	///
	/// The path to the binary export will be passed as the first argument
	command: String,

	/// The package to run
	package: CliDependencySpecifier,

	/// The maximum number of concurrent network requests
	#[arg(short, long, default_value = "16")]
	network_concurrency: NonZero<usize>,

	/// Arguments to pass to the script
	#[arg(allow_hyphen_values = true)]
	args: Vec<OsString>,
}

impl ExecuteCommand {
	pub async fn run(mut self, ctx: CommandContext) -> anyhow::Result<()> {
		let reqwest = ctx.reqwest().clone();
		let cli_config = ctx.cli_config().await?;
		let vfs = ctx.vfs().await?;
		let member = ctx.member().await?;
		let project = ctx.project().await?;

		let multi_progress = MultiProgress::new();
		crate::PROGRESS_BARS
			.lock()
			.unwrap()
			.replace(multi_progress.clone());

		let (tempdir, bin_file) = reporters::run_with_reporter_and_writer(
			std::io::stderr(),
			async |multi_progress, root_progress, reporter| {
				let multi_progress = multi_progress;
				let root_progress = root_progress;

				root_progress.set_message("prepare");

				let specifier = self.package.resolve(member).await?;
				if matches!(specifier, DependencySpecifier::Wally(..)) {
					anyhow::bail!("cannot execute wally packages");
				} else if matches!(specifier, DependencySpecifier::Workspace(..)) {
					anyhow::bail!("cannot execute workspace packages");
				}

				let versions = crate::cli::ui::specifier::resolve(project, specifier).await?;

				let package_id = versions
					.try_fold(None::<PackageId>, async |acc, (candidate, _)| {
						let Some(acc) = acc else {
							return Ok(Some(candidate));
						};

						match candidate.compare(&acc) {
							PackageIdComparison::Comparable(ordering) => {
								if ordering == Ordering::Greater {
									Ok(Some(candidate))
								} else {
									Ok(Some(acc))
								}
							}
							PackageIdComparison::Incomparable => Ok(Some(candidate)),
							PackageIdComparison::NotSameSource => anyhow::bail!(
								"package `{candidate}` is not of the same source as package `{acc}`"
							),
						}
					})
					.await
					.context("failed to find version")?
					.context("no versions found for package")?;

				multi_progress.suspend(|| {
					eprintln!(
						"{}",
						style(format_args!("using {}", style(&package_id).bold())).dim()
					);
				});

				root_progress.reset();
				root_progress.set_message("download");
				root_progress.set_style(reporters::root_progress_style_with_progress());

				let source = project
					.source_provider()
					.get(&package_id.source_id())
					.await
					.map_err(anyhow::Error::from_boxed)
					.context("failed to get package source")?;

				let inspect_result = source
					.inspect(&package_id)
					.await
					.map_err(anyhow::Error::from_boxed)
					.context("failed to inspect package")?;

				let structure_kind = inspect_result.structure.kind();
				let (manifest, bin_file) = match inspect_result.structure {
					PackageStructure::Pesde(exports, config) => {
						let mut manifest = Manifest::new_empty(ManifestPackage {
							name: "fake/name".parse().unwrap(),
							version: "0.1.0-fake".parse().unwrap(),
							description: "".parse().unwrap(),
							license: "".parse().unwrap(),
							repository: None,
						});

						let bin_file = exports
							.bin()
							.context("package does not have a binary export")?
							.clone();

						manifest.exports = Some(exports);
						manifest.config = config;
						manifest.dependencies = BoundedBTreeMap::new(inspect_result.dependencies)?;

						(manifest, bin_file)
					}
					PackageStructure::LegacyPesde(exports, _) => {
						let mut manifest = Manifest::new_empty(ManifestPackage {
							name: "fake/name".parse().unwrap(),
							version: "0.1.0-fake".parse().unwrap(),
							description: "".parse().unwrap(),
							license: "".parse().unwrap(),
							repository: None,
						});

						let bin_file = exports
							.bin()
							.context("package does not have a binary export")?
							.clone();

						manifest.exports = Some(exports);
						manifest.dependencies = BoundedBTreeMap::new(inspect_result.dependencies)?;

						(manifest, bin_file)
					}
					PackageStructure::Wally(..) => {
						anyhow::bail!("cannot execute wally packages")
					}
				};

				let tempdir = vfs.unique_tmp_path();
				tokio::fs::create_dir_all(&tempdir).await.with_context(|| {
					format!(
						"failed to create temporary directory `{}`",
						tempdir.display()
					)
				})?;

				let temp_vfs = FsVfs::from_parts(vfs.cas_dir.clone(), tempdir.clone());
				let temp_vfs = Arc::new(temp_vfs);

				let storer = temp_vfs
					.clone()
					.copy_and_maybe_store_package(
						true,
						&package_id,
						structure_kind,
						&VfsPath::root(),
					)
					.await
					.context("failed to copy package to temporary directory")?;
				if let Some(mut storer) = storer {
					source
						.download(
							&package_id,
							Arc::new(reporter.clone().report_download(package_id.to_string())),
							&mut *storer,
						)
						.await
						.map_err(anyhow::Error::from_boxed)
						.context("failed to download package")?;
					storer
						.commit()
						.await
						.context("failed to commit package download")?;
				}

				let temp_project = Project::for_published_package(
					temp_vfs,
					|workspace| {
						Arc::new(CliSourceProvider::new(
							workspace.clone(),
							reqwest,
							cli_config.auth_scripts.clone(),
						))
					},
					Arc::new(manifest),
				);

				let (resolution, _) = temp_project
					.solve(None)
					.await
					.context("failed to resolve package")?;

				temp_project
					.install(
						&resolution,
						InstallOptions::<CliReporter<Stderr>>::new()
							.reporter(reporter)
							.install_dependencies_mode(InstallDependenciesMode::Prod)
							.network_concurrency(self.network_concurrency),
					)
					.await
					.context("failed to install package")?;

				Ok((tempdir, bin_file))
			},
		)
		.await?;

		self.args.insert(0, bin_file.to_string().into());

		let code = execute_script(tempdir.clone().into(), &self.command, &mut (), self.args)
			.await
			.context("failed to execute script")?;

		tokio::fs::remove_dir_all(&tempdir).await.with_context(|| {
			format!(
				"failed to remove temporary directory `{}`",
				tempdir.display()
			)
		})?;

		std::process::exit(code);
	}
}
