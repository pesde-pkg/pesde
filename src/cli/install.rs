use crate::cli::{
	dep_type_to_key,
	reporters::{self, CliReporter},
	resolve_overrides, run_on_workspace_members,
	style::{ADDED_STYLE, REMOVED_STYLE, WARN_PREFIX},
	up_to_date_lockfile,
};
use anyhow::Context as _;
use console::style;
use fs_err::tokio as fs;
use pesde::{
	Project, RefreshedSources,
	download_and_link::{DownloadAndLinkHooks, DownloadAndLinkOptions, InstallDependenciesMode},
	graph::DependencyGraph,
	lockfile::Lockfile,
	manifest::DependencyType,
	source::{PackageSources, refs::PackageRefs, traits::RefreshOptions},
};
use std::{
	collections::{BTreeMap, BTreeSet},
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

	async fn on_bins_downloaded(&self, aliases: BTreeSet<&str>) -> Result<(), Self::Error> {
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
					match fs::hard_link(curr_exe, bin_exec_file).await {
						Ok(_) => {}
						Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
						e => e.context("failed to hard link bin link file")?,
					}

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

	let resolved_engine_versions =
		Arc::new(super::get_project_engines(&manifest, &reqwest, project.auth_config()).await?);

	let overrides = resolve_overrides(&manifest)?;

	let (new_lockfile, old_graph) =
		reporters::run_with_reporter(|multi, root_progress, reporter| async {
			let multi = multi;
			let root_progress = root_progress;

			root_progress.set_prefix(format!("{}: ", manifest.target));
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

			check_peers_satisfied(&graph);

			#[expect(deprecated)]
			let mut tasks = graph
				.iter()
				.filter_map(|(id, node)| {
					let PackageSources::Pesde(source) = id.source() else {
						return None;
					};
					let PackageRefs::Pesde(pkg_ref) = &node.resolved.pkg_ref else {
						return None;
					};
					let source = source.clone();
					let name = pkg_ref.name.clone();
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

						let file = source
							.read_index_file(&name, &project)
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

				let bin_dir = project.bin_dir();
				match fs::remove_dir_all(bin_dir).await {
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
					e => e.context("failed to remove bin directory")?,
				}
				fs::create_dir_all(bin_dir)
					.await
					.context("failed to create bin directory")?;

				let hooks = InstallHooks {
					bin_folder: bin_dir.to_path_buf(),
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
							.force(options.force || has_irrecoverable_changes)
							.engines(resolved_engine_versions.clone()),
					)
					.await
					.context("failed to download and link dependencies")?;

				#[cfg(feature = "version-management")]
				#[expect(deprecated)]
				{
					use pesde::{
						MANIFEST_FILE_NAME, engine::EngineKind, manifest::Manifest,
						source::refs::PackageRefs, version_matches,
					};

					let manifest_target_kind = manifest.target.kind();
					let mut tasks = downloaded_graph
						.iter()
						.map(|(id, node)| {
							let id = id.clone();
							let node = node.clone();
							let project = project.clone();
							let refreshed_sources = refreshed_sources.clone();

							async move {
								let engines = match &node.node.resolved.pkg_ref {
									PackageRefs::Pesde(pkg_ref) => {
										let PackageSources::Pesde(source) = id.source() else {
											return Ok((id, Default::default()));
										};

										refreshed_sources
											.refresh(
												id.source(),
												&RefreshOptions {
													project: project.clone(),
												},
											)
											.await
											.context("failed to refresh source")?;

										let mut file = source
											.read_index_file(&pkg_ref.name, &project)
											.await
											.context("failed to read package index file")?
											.context("package not found in index")?;

										file.entries
											.remove(id.v_id())
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

										match fs::read_to_string(path.join(MANIFEST_FILE_NAME))
											.await
										{
											Ok(manifest) => {
												match toml::from_str::<Manifest>(&manifest) {
													Ok(manifest) => manifest.engines,
													Err(e) => {
														return Err(e).context(
															"failed to read package manifest",
														);
													}
												}
											}
											Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
												Default::default()
											}
											Err(e) => {
												return Err(e)
													.context("failed to read package manifest");
											}
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
								tracing::debug!(
									"package {id} requires {engine} {req}, but it is not installed"
								);
								continue;
							};

							if !version_matches(&req, version) {
								multi.suspend(|| {
									println!(
										"{WARN_PREFIX}: package {id} requires {engine} {req}, but {version} is installed"
									);
								});
							}
						}
					}
				}
			}

			root_progress.reset();
			root_progress.set_message("finish");

			let new_lockfile = Lockfile {
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
		&format!("{}:", manifest.target),
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
					"{} {}",
					if added {
						ADDED_STYLE.apply_to("+")
					} else {
						REMOVED_STYLE.apply_to("-")
					},
					style(id.v_id()).dim()
				);
			}
		}

		println!();
	}
}

pub fn check_peers_satisfied(graph: &DependencyGraph) {
	for (id, node) in graph {
		let Some((alias, _, _)) = &node.direct else {
			continue;
		};

		let mut queue = node
			.resolved_dependencies
			.iter()
			.map(|(dep_alias, dep_id)| {
				(
					vec![(id, alias)],
					(dep_id, dep_alias),
					node.resolved.dependencies[dep_alias].1,
				)
			})
			.collect::<Vec<_>>();

		while let Some((path, (dep_id, dep_alias), dep_ty)) = queue.pop() {
			if dep_ty == DependencyType::Peer {
				let mut iter = path
					.iter()
					.map(|(id, _)| id)
					.rev()
					// skip our parent since we're always going to be descendants of it
					.skip(1)
					.take(2);

				let satisfied = if iter.len() > 0 {
					iter.any(|id| {
						graph[id]
							.resolved_dependencies
							.values()
							.any(|id| id == dep_id)
					})
				} else {
					graph.get(dep_id).is_some_and(|node| node.direct.is_some())
				};

				if !satisfied {
					eprintln!(
						"{WARN_PREFIX}: peer dependency {}>{dep_alias} is not satisfied",
						path.iter()
							.map(|(_, alias)| alias.as_str())
							.collect::<Vec<_>>()
							.join(">"),
					);
				}
			}

			queue.extend(graph[dep_id].resolved_dependencies.iter().map(
				|(inner_dep_alias, inner_dep_id)| {
					(
						path.iter()
							.copied()
							.chain(std::iter::once((dep_id, dep_alias)))
							.collect(),
						(inner_dep_id, inner_dep_alias),
						graph[dep_id].resolved.dependencies[inner_dep_alias].1,
					)
				},
			));
		}
	}
}
