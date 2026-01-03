use crate::cli::{
	dep_type_to_key,
	reporters::{self, CliReporter},
	resolve_overrides,
	style::{ADDED_STYLE, REMOVED_STYLE, WARN_PREFIX},
	up_to_date_lockfile,
};
use anyhow::Context as _;
use console::style;
use fs_err::tokio as fs;
use itertools::{EitherOrBoth, Itertools};
use pesde::{
	Project, RefreshedSources,
	download_and_link::{DownloadAndLinkHooks, DownloadAndLinkOptions, InstallDependenciesMode},
	graph::{DependencyGraph, DependencyTypeGraph},
	lockfile::Lockfile,
	private_dir,
	source::{PackageSources, refs::PackageRefs, traits::RefreshOptions},
};
use relative_path::RelativePath;
use std::{
	cmp::Ordering,
	collections::{BTreeMap, BTreeSet},
	fmt::Display,
	num::NonZeroUsize,
	path::Path,
	sync::Arc,
	time::Instant,
};
use tokio::task::JoinSet;

pub struct InstallHooks {
	pub project: Project,
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct InstallHooksError(#[from] anyhow::Error);

impl DownloadAndLinkHooks for InstallHooks {
	type Error = InstallHooksError;

	async fn on_bins_downloaded<'a>(
		&self,
		importer: &RelativePath,
		aliases: impl Iterator<Item = &'a str>,
	) -> Result<(), Self::Error> {
		let dir = private_dir(&self.project, importer);
		let bin_dir = dir.join("bin");

		let curr_exe: Arc<Path> = std::env::current_exe()
			.context("failed to get current executable path")?
			.as_path()
			.into();

		let mut tasks = aliases
			.into_iter()
			.map(|alias| {
				let bin_exec_file = bin_dir
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
				if lockfile.overrides == resolve_overrides(&manifest)? {
					Some(lockfile)
				} else {
					tracing::debug!("overrides are different");
					has_irrecoverable_changes = true;
					None
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

			root_progress.reset();
			root_progress.set_message("resolve");

			let old_graph = lockfile.map(|lockfile| lockfile.graph);

			let (graph, type_graph) = project
				.dependency_graph(
					old_graph.clone().filter(|_| options.use_lockfile),
					refreshed_sources.clone(),
					false,
				)
				.await
				.context("failed to build dependency graph")?;

			if let Some(type_graph) = type_graph {
				check_peers_satisfied(&type_graph);
			}

			#[expect(deprecated)]
			let mut tasks = graph
				.nodes
				.keys()
				.filter_map(|id| {
					let PackageSources::Pesde(source) = id.source() else {
						return None;
					};
					let PackageRefs::Pesde(pkg_ref) = id.pkg_ref() else {
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

				let hooks = InstallHooks {
					project: project.clone(),
				};

				#[allow(unused_variables)]
				project
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
					use pesde::{engine::EngineKind, source::refs::PackageRefs, version_matches};

					let mut tasks = graph
						.nodes
						.keys()
						.map(|id| {
							let id = id.clone();
							let project = project.clone();
							let refreshed_sources = refreshed_sources.clone();

							async move {
								let engines = match id.pkg_ref() {
									PackageRefs::Pesde(pkg_ref) => {
										let PackageSources::Pesde(source) = id.source() else {
											return Ok::<_, anyhow::Error>((
												id,
												Default::default(),
											));
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
									// _ => {
									// 	let path =

									// 	match fs::read_to_string(path.join(MANIFEST_FILE_NAME))
									// 		.await
									// 	{
									// 		Ok(manifest) => {
									// 			match toml::from_str::<PesdeVersionedManifest>(
									// 				&manifest,
									// 			) {
									// 				Ok(manifest) => {
									// 					manifest.into_manifest().engines
									// 				}
									// 				Err(e) => {
									// 					return Err(e).context(
									// 						"failed to read package manifest",
									// 					);
									// 				}
									// 			}
									// 		}
									// 		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
									// 			Default::default()
									// 		}
									// 		Err(e) => {
									// 			return Err(e)
									// 				.context("failed to read package manifest");
									// 		}
									// 	}
									// }
									_ => {
										// TODO
										return Ok::<_, anyhow::Error>((id, Default::default()));
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

			let new_lockfile = Lockfile { overrides, graph };

			project
				.write_lockfile(&new_lockfile)
				.await
				.context("failed to write lockfile")?;

			anyhow::Ok((new_lockfile, old_graph))
		})
		.await?;

	let elapsed = start.elapsed();

	print_install_summary(old_graph, new_lockfile.graph);

	println!("done in {:.2}s", elapsed.as_secs_f64());

	Ok(())
}

pub fn print_install_summary(old_graph: Option<DependencyGraph>, new_graph: DependencyGraph) {
	let old_importers = old_graph
		.map_or(BTreeMap::new(), |old_graph| old_graph.importers)
		.into_iter();
	let new_importers = new_graph.importers.into_iter();

	let importer_pairs = old_importers
		.merge_join_by(new_importers, |(old_importer, _), (new_importer, _)| {
			old_importer.cmp(new_importer)
		})
		.map(|m| match m {
			EitherOrBoth::Both((importer, old), (_, new)) => (importer, old, new),
			EitherOrBoth::Left((importer, old)) => (importer, old, BTreeMap::new()),
			EitherOrBoth::Right((importer, new)) => (importer, BTreeMap::new(), new),
		});

	for (importer, old, new) in importer_pairs {
		// TODO: populate ts
		let peer_warnings: Vec<String> = vec![];

		enum Change {
			Added,
			Removed,
		}

		let groups = new
			.into_iter()
			.merge_join_by(
				old.into_iter(),
				|(new_alias, (new_id, _, _)), (old_alias, (old_id, _, _))| {
					new_alias.cmp(old_alias).then(if new_id == old_id {
						Ordering::Equal
					} else {
						Ordering::Less
					})
				},
			)
			.filter_map(|m| match m {
				EitherOrBoth::Both(_, _) => None,
				EitherOrBoth::Left((alias, (id, _, ty))) => Some((ty, (Change::Added, alias, id))),
				EitherOrBoth::Right((alias, (id, _, ty))) => {
					Some((ty, (Change::Removed, alias, id)))
				}
			})
			.into_group_map();

		if groups.is_empty() && peer_warnings.is_empty() {
			continue;
		}

		println!(
			"{}",
			style(if importer.as_str().is_empty() {
				"(root)"
			} else {
				importer.as_str()
			})
			.bold()
		);

		for (ty, changes) in groups {
			println!("  {}", style(dep_type_to_key(ty)).yellow().bold());

			for (change, alias, id) in changes {
				let version = if let PackageSources::Path(_) = id.source() {
					format_args!("")
				} else {
					format_args!(" v{}", id.v_id().version())
				};

				let sign = match change {
					Change::Added => ADDED_STYLE.apply_to("+"),
					Change::Removed => REMOVED_STYLE.apply_to("-"),
				};

				println!(
					"    {sign} {alias}{} {}",
					style(version).cyan(),
					style(&id).dim()
				)
			}
		}

		println!();

		for msg in peer_warnings {
			println!("{msg}")
		}
	}
}

pub fn check_peers_satisfied(graph: &DependencyTypeGraph) {
	// TODO
	return;
	// for (id, node) in graph {
	// 	let Some(alias) = &node.direct else {
	// 		continue;
	// 	};

	// 	let mut queue = node
	// 		.dependencies
	// 		.iter()
	// 		.map(|(dep_alias, (dep_id, dep_ty))| (vec![(id, alias)], (dep_id, dep_alias), *dep_ty))
	// 		.collect::<Vec<_>>();

	// 	while let Some((path, (dep_id, dep_alias), dep_ty)) = queue.pop() {
	// 		if dep_ty == DependencyType::Peer {
	// 			let mut iter = path
	// 				.iter()
	// 				.map(|(id, _)| id)
	// 				.rev()
	// 				// skip our parent since we're always going to be descendants of it
	// 				.skip(1)
	// 				.take(2);

	// 			let satisfied = if iter.len() > 0 {
	// 				iter.any(|id| graph[id].dependencies.values().any(|(id, _)| id == dep_id))
	// 			} else {
	// 				graph.get(dep_id).is_some_and(|node| node.direct.is_some())
	// 			};

	// 			if !satisfied {
	// 				eprintln!(
	// 					"{WARN_PREFIX}: peer dependency {}>{dep_alias} is not satisfied",
	// 					path.iter()
	// 						.map(|(_, alias)| alias.as_str())
	// 						.collect::<Vec<_>>()
	// 						.join(">"),
	// 				);
	// 			}
	// 		}

	// 		queue.extend(graph[dep_id].dependencies.iter().map(
	// 			|(inner_dep_alias, (inner_dep_id, inner_dep_ty))| {
	// 				(
	// 					path.iter()
	// 						.copied()
	// 						.chain(std::iter::once((dep_id, dep_alias)))
	// 						.collect(),
	// 					(inner_dep_id, inner_dep_alias),
	// 					*inner_dep_ty,
	// 				)
	// 			},
	// 		));
	// 	}
	// }
}
