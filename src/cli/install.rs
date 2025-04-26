use super::files::make_executable;
use crate::cli::{
	bin_dir, dep_type_to_key,
	reporters::{self, run_with_reporter, CliReporter},
	resolve_overrides, run_on_workspace_members,
	style::{ADDED_STYLE, REMOVED_STYLE, WARN_PREFIX},
	up_to_date_lockfile,
};
use anyhow::Context as _;
use console::style;
use fs_err::tokio as fs;
use pesde::{
	download_and_link::{DownloadAndLinkHooks, DownloadAndLinkOptions, InstallDependenciesMode},
	engine::EngineKind,
	graph::{DependencyGraph, DependencyGraphWithTarget},
	lockfile::Lockfile,
	manifest::{DependencyType, Manifest},
	names::PackageNames,
	source::{
		pesde::PesdePackageSource,
		refs::PackageRefs,
		traits::{PackageRef as _, RefreshOptions},
		PackageSources,
	},
	version_matches, Project, RefreshedSources, MANIFEST_FILE_NAME,
};
use std::{
	collections::{BTreeMap, BTreeSet, HashMap, HashSet},
	num::NonZeroUsize,
	path::Path,
	sync::Arc,
	time::Instant,
};
use tokio::task::JoinSet;

pub struct InstallHooks {
	pub bin_folder: std::path::PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct InstallHooksError(#[from] anyhow::Error);

impl DownloadAndLinkHooks for InstallHooks {
	type Error = InstallHooksError;

	async fn on_bins_downloaded(
		&self,
		graph: &DependencyGraphWithTarget,
	) -> Result<(), Self::Error> {
		let binary_packages = graph
			.iter()
			.filter_map(|(id, node)| node.target.bin_path().is_some().then_some(id))
			.collect::<HashSet<_>>();

		let aliases = graph
			.iter()
			.flat_map(|(_, node)| node.node.dependencies.iter())
			.filter_map(|(id, alias)| binary_packages.contains(id).then_some(alias.as_str()))
			.chain(
				graph
					.iter()
					.filter(|(_, node)| node.target.bin_path().is_some())
					.filter_map(|(_, node)| node.node.direct.as_ref())
					.map(|(alias, _, _)| alias.as_str()),
			)
			.collect::<HashSet<_>>();

		let curr_exe: Arc<Path> = std::env::current_exe()
			.context("failed to get current executable path")?
			.as_path()
			.into();

		let mut tasks = aliases
			.into_iter()
			.map(|alias| {
				let bin_exec_file = self
					.bin_folder
					.join(alias)
					.with_extension(std::env::consts::EXE_EXTENSION);
				let curr_exe = curr_exe.clone();

				async move {
					// TODO: remove this in a major release
					#[cfg(unix)]
					if fs::metadata(&bin_exec_file)
						.await
						.is_ok_and(|m| !m.is_symlink())
					{
						fs::remove_file(&bin_exec_file)
							.await
							.context("failed to remove outdated bin linker")?;
					}

					#[cfg(windows)]
					let res = fs::symlink_file(curr_exe, &bin_exec_file).await;
					#[cfg(unix)]
					let res = fs::symlink(curr_exe, &bin_exec_file).await;

					match res {
						Ok(_) => {}
						Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
						e => e.context("failed to symlink bin link file")?,
					}

					make_executable(&bin_exec_file)
						.await
						.context("failed to make bin link file executable")?;

					Ok::<_, anyhow::Error>(())
				}
			})
			.collect::<JoinSet<_>>();

		while let Some(task) = tasks.join_next().await {
			task.unwrap()?;
		}

		Ok(())
	}
}

#[derive(Debug, Clone, Copy)]
pub struct InstallOptions {
	pub locked: bool,
	pub install_dependencies_mode: InstallDependenciesMode,
	pub write: bool,
	pub use_lockfile: bool,
	pub network_concurrency: NonZeroUsize,
	pub force: bool,
}

pub async fn install(
	options: &InstallOptions,
	project: &Project,
	reqwest: reqwest::Client,
	is_root: bool,
) -> anyhow::Result<()> {
	let start = Instant::now();

	let refreshed_sources = RefreshedSources::new();

	let manifest = project
		.deser_manifest()
		.await
		.context("failed to read manifest")?;

	let mut has_irrecoverable_changes = false;

	let lockfile = if options.locked {
		match up_to_date_lockfile(project).await? {
			None => {
				anyhow::bail!(
					"lockfile is out of sync, run `{} install` to update it",
					env!("CARGO_BIN_NAME")
				);
			}
			file => file,
		}
	} else {
		match project.deser_lockfile().await {
			Ok(lockfile) => {
				if lockfile.overrides != resolve_overrides(&manifest)? {
					tracing::debug!("overrides are different");
					has_irrecoverable_changes = true;
					None
				} else if lockfile.target != manifest.target.kind() {
					tracing::debug!("target kind is different");
					has_irrecoverable_changes = true;
					None
				} else {
					Some(lockfile)
				}
			}
			Err(pesde::errors::LockfileReadError::Io(e))
				if e.kind() == std::io::ErrorKind::NotFound =>
			{
				None
			}
			Err(e) => return Err(e.into()),
		}
	};

	let progress_prefix = format!("{} {}: ", manifest.name, manifest.target);

	#[cfg(feature = "version-management")]
	let resolved_engine_versions = run_with_reporter(|_, root_progress, reporter| async {
		let root_progress = root_progress;
		let reporter = reporter;

		root_progress.set_prefix(progress_prefix.clone());
		root_progress.reset();
		root_progress.set_message("update engines");

		let mut tasks = manifest
			.engines
			.iter()
			.map(|(engine, req)| {
				let engine = *engine;
				let req = req.clone();
				let reqwest = reqwest.clone();
				let reporter = reporter.clone();

				async move {
					let version = crate::cli::version::get_or_download_engine(
						&reqwest, engine, req, reporter,
					)
					.await?
					.1;
					crate::cli::version::make_linker_if_needed(engine).await?;

					Ok::<_, anyhow::Error>((engine, version))
				}
			})
			.collect::<JoinSet<_>>();

		let mut resolved_engine_versions = HashMap::new();

		while let Some(task) = tasks.join_next().await {
			let (engine, version) = task.unwrap()?;
			resolved_engine_versions.insert(engine, version);
		}

		Ok::<_, anyhow::Error>(resolved_engine_versions)
	})
	.await?;

	let overrides = resolve_overrides(&manifest)?;

	let (new_lockfile, old_graph) =
		reporters::run_with_reporter(|multi, root_progress, reporter| async {
			let multi = multi;
			let root_progress = root_progress;

			root_progress.set_prefix(progress_prefix);
			root_progress.reset();
			root_progress.set_message("resolve");

			let old_graph = lockfile.map(|lockfile| lockfile.graph);

			let graph = project
				.dependency_graph(
					old_graph.as_ref().filter(|_| options.use_lockfile),
					refreshed_sources.clone(),
					false,
				)
				.await
				.context("failed to build dependency graph")?;

			let mut tasks = graph
				.iter()
				.filter_map(|(id, node)| {
					let PackageSources::Pesde(source) = node.pkg_ref.source() else {
						return None;
					};
					#[allow(irrefutable_let_patterns)]
					let PackageNames::Pesde(name) = id.name().clone() else {
						panic!("unexpected package name");
					};
					let project = project.clone();
					let refreshed_sources = refreshed_sources.clone();

					Some(async move {
						refreshed_sources
							.refresh(
								&PackageSources::Pesde(source.clone()),
								&RefreshOptions {
									project: project.clone(),
								},
							)
							.await
							.context("failed to refresh source")?;

						let file = source.read_index_file(&name, &project)
							.await
							.context("failed to read package index file")?
							.context("package not found in index")?;

						Ok::<_, anyhow::Error>(if file.meta.deprecated.is_empty() {
							None
						} else {
							Some((name, file.meta.deprecated))
						})
					})
				})
				.collect::<JoinSet<_>>();

			while let Some(task) = tasks.join_next().await {
				let Some((name, reason)) = task.unwrap()? else {
					continue;
				};

				multi.suspend(|| {
					println!("{WARN_PREFIX}: package {name} is deprecated: {reason}");
				});
			}

			if options.write {
				root_progress.reset();
				root_progress.set_length(0);
				root_progress.set_message("download");
				root_progress.set_style(reporters::root_progress_style_with_progress());

				let hooks = InstallHooks {
					bin_folder: bin_dir().await?,
				};

				#[allow(unused_variables)]
				let downloaded_graph = project
					.download_and_link(
						&graph,
						DownloadAndLinkOptions::<CliReporter, InstallHooks>::new(reqwest.clone())
							.reporter(reporter)
							.hooks(hooks)
							.refreshed_sources(refreshed_sources.clone())
							.install_dependencies_mode(options.install_dependencies_mode)
							.network_concurrency(options.network_concurrency)
							.force(options.force || has_irrecoverable_changes),
					)
					.await
					.context("failed to download and link dependencies")?;

				#[cfg(feature = "version-management")]
				{
					let manifest_target_kind = manifest.target.kind();
					let mut tasks = downloaded_graph.iter()
						.map(|(id, node)| {
							let id = id.clone();
							let node = node.clone();
							let project = project.clone();
							let refreshed_sources = refreshed_sources.clone();

							async move {
								let engines = match &node.node.pkg_ref {
									PackageRefs::Pesde(pkg_ref) => {
										let source = PesdePackageSource::new(pkg_ref.index_url.clone());
										refreshed_sources
											.refresh(
												&PackageSources::Pesde(source.clone()),
												&RefreshOptions {
													project: project.clone(),
												},
											)
											.await
											.context("failed to refresh source")?;

										#[allow(irrefutable_let_patterns)]
										let PackageNames::Pesde(name) = id.name() else {
											panic!("unexpected package name");
										};

										let mut file = source.read_index_file(name, &project)
											.await
											.context("failed to read package index file")?
											.context("package not found in index")?;

										file
											.entries
											.remove(id.version_id())
											.context("package version not found in index")?
											.engines
									}
									#[cfg(feature = "wally-compat")]
									PackageRefs::Wally(_) => Default::default(),
									_ => {
										let path = node.node.container_folder_from_project(
											&id,
											&project,
											manifest_target_kind,
										);

										match fs::read_to_string(path.join(MANIFEST_FILE_NAME)).await {
											Ok(manifest) => match toml::from_str::<Manifest>(&manifest) {
												Ok(manifest) => manifest.engines,
												Err(e) => return Err(e).context("failed to read package manifest"),
											},
											Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
											Err(e) => return Err(e).context("failed to read package manifest"),
										}
									}
								};

								Ok((id, engines))
							}
						})
						.collect::<JoinSet<_>>();

					while let Some(task) = tasks.join_next().await {
						let (id, required_engines) = task.unwrap()?;

						for (engine, req) in required_engines {
							if engine == EngineKind::Pesde {
								continue;
							}

							let Some(version) = resolved_engine_versions.get(&engine) else {
								tracing::debug!("package {id} requires {engine} {req}, but it is not installed");
								continue;
							};

							if !version_matches(&req, version) {
								multi.suspend(|| {
									println!("{WARN_PREFIX}: package {id} requires {engine} {req}, but {version} is installed");
								});
							}
						}
					}
				}
			}

			root_progress.reset();
			root_progress.set_message("finish");

			let new_lockfile = Lockfile {
				name: manifest.name.clone(),
				version: manifest.version,
				target: manifest.target.kind(),
				overrides,

				graph,

				workspace: run_on_workspace_members(project, |_| async { Ok(()) }).await?,
			};

			project
				.write_lockfile(&new_lockfile)
				.await
				.context("failed to write lockfile")?;

			anyhow::Ok((new_lockfile, old_graph.unwrap_or_default()))
		})
			.await?;

	let elapsed = start.elapsed();

	if is_root {
		println!();
	}

	print_package_diff(
		&format!("{} {}:", manifest.name, manifest.target),
		&old_graph,
		&new_lockfile.graph,
	);

	println!("done in {:.2}s", elapsed.as_secs_f64());
	println!();

	Ok(())
}

/// Prints the difference between two graphs.
pub fn print_package_diff(prefix: &str, old_graph: &DependencyGraph, new_graph: &DependencyGraph) {
	let mut old_pkg_map = BTreeMap::new();
	let mut old_direct_pkg_map = BTreeMap::new();
	let mut new_pkg_map = BTreeMap::new();
	let mut new_direct_pkg_map = BTreeMap::new();

	for (id, node) in old_graph {
		old_pkg_map.insert(id, node);
		if node.direct.is_some() {
			old_direct_pkg_map.insert(id, node);
		}
	}

	for (id, node) in new_graph {
		new_pkg_map.insert(id, node);
		if node.direct.is_some() {
			new_direct_pkg_map.insert(id, node);
		}
	}

	let added_pkgs = new_pkg_map
		.iter()
		.filter(|(key, _)| !old_pkg_map.contains_key(*key))
		.map(|(key, &node)| (key, node))
		.collect::<Vec<_>>();
	let removed_pkgs = old_pkg_map
		.iter()
		.filter(|(key, _)| !new_pkg_map.contains_key(*key))
		.map(|(key, &node)| (key, node))
		.collect::<Vec<_>>();
	let added_direct_pkgs = new_direct_pkg_map
		.iter()
		.filter(|(key, _)| !old_direct_pkg_map.contains_key(*key))
		.map(|(key, &node)| (key, node))
		.collect::<Vec<_>>();
	let removed_direct_pkgs = old_direct_pkg_map
		.iter()
		.filter(|(key, _)| !new_direct_pkg_map.contains_key(*key))
		.map(|(key, &node)| (key, node))
		.collect::<Vec<_>>();

	let prefix = style(prefix).bold();

	let no_changes = added_pkgs.is_empty()
		&& removed_pkgs.is_empty()
		&& added_direct_pkgs.is_empty()
		&& removed_direct_pkgs.is_empty();

	if no_changes {
		println!("{prefix} already up to date");
	} else {
		let mut change_signs = [
			(!added_pkgs.is_empty()).then(|| {
				ADDED_STYLE
					.apply_to(format!("+{}", added_pkgs.len()))
					.to_string()
			}),
			(!removed_pkgs.is_empty()).then(|| {
				REMOVED_STYLE
					.apply_to(format!("-{}", removed_pkgs.len()))
					.to_string()
			}),
		]
		.into_iter()
		.flatten()
		.collect::<Vec<_>>()
		.join(" ");

		let changes_empty = change_signs.is_empty();
		if changes_empty {
			change_signs = style("(no changes)").dim().to_string();
		}

		println!("{prefix} {change_signs}");

		if !changes_empty {
			println!(
				"{}{}",
				ADDED_STYLE.apply_to("+".repeat(added_pkgs.len())),
				REMOVED_STYLE.apply_to("-".repeat(removed_pkgs.len()))
			);
		}

		let dependency_groups = added_direct_pkgs
			.iter()
			.map(|(key, node)| (true, key, node))
			.chain(
				removed_direct_pkgs
					.iter()
					.map(|(key, node)| (false, key, node)),
			)
			.filter_map(|(added, key, node)| {
				node.direct.as_ref().map(|(_, _, ty)| (added, key, ty))
			})
			.fold(
				BTreeMap::<DependencyType, BTreeSet<_>>::new(),
				|mut map, (added, key, &ty)| {
					map.entry(ty).or_default().insert((key, added));
					map
				},
			);

		for (ty, set) in dependency_groups {
			println!();
			println!(
				"{}",
				style(format!("{}:", dep_type_to_key(ty))).yellow().bold()
			);

			for (id, added) in set {
				println!(
					"{} {} {}",
					if added {
						ADDED_STYLE.apply_to("+")
					} else {
						REMOVED_STYLE.apply_to("-")
					},
					id.name(),
					style(id.version_id()).dim()
				);
			}
		}

		println!();
	}
}
