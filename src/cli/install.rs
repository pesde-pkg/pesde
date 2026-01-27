use crate::cli::dep_type_to_key;
use crate::cli::reporters;
use crate::cli::reporters::CliReporter;
use crate::cli::style::ADDED_STYLE;
use crate::cli::style::REMOVED_STYLE;
use crate::cli::style::WARN_PREFIX;
use anyhow::Context as _;
use console::style;
use fs_err::tokio as fs;
use itertools::EitherOrBoth;
use itertools::Itertools as _;
use pesde::Importer;
use pesde::Project;
use pesde::RefreshedSources;
use pesde::download_and_link::DownloadAndLinkHooks;
use pesde::download_and_link::DownloadAndLinkOptions;
use pesde::download_and_link::InstallDependenciesMode;
use pesde::graph::DependencyGraph;
use pesde::graph::DependencyTypeGraph;
use pesde::lockfile::Lockfile;
use pesde::manifest::DependencyType;
use pesde::source::PackageRefs;
use pesde::source::PackageSources;
use pesde::source::traits::RefreshOptions;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::task::JoinSet;

pub struct InstallHooks {
	pub project: Project,
	pub global_binaries: bool,
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct InstallHooksError(#[from] anyhow::Error);

impl DownloadAndLinkHooks for InstallHooks {
	type Error = InstallHooksError;

	async fn on_bins_downloaded<'a>(
		&self,
		importer: &Importer,
		aliases: impl IntoIterator<Item = &'a str>,
	) -> Result<(), Self::Error> {
		if !self.global_binaries {
			return Ok(());
		}

		let dir = self.project.clone().subproject(importer.clone()).dir();
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

async fn get_graph_internal(
	project: &Project,
	refreshed_sources: &RefreshedSources,
	locked: bool,
	use_lockfile: bool,
) -> anyhow::Result<(
	Option<DependencyGraph>,
	DependencyGraph,
	Option<DependencyTypeGraph>,
)> {
	let lockfile = if use_lockfile {
		match project.deser_lockfile().await {
			Ok(lockfile) => Some(lockfile),
			Err(e) => {
				if let pesde::errors::LockfileReadErrorKind::Io(e) = e.inner()
					&& e.kind() == std::io::ErrorKind::NotFound
				{
					None
				} else {
					return Err(e.into());
				}
			}
		}
	} else {
		None
	};

	let old_graph = lockfile.map(|lockfile| lockfile.graph);

	let (graph, type_graph, updated) = project
		.dependency_graph(old_graph.as_ref(), refreshed_sources, false)
		.await
		.context("failed to build dependency graph")?;

	if updated && locked {
		anyhow::bail!(
			"lockfile is out of sync, run `{} install` without --locked to update it",
			env!("CARGO_BIN_NAME")
		);
	}

	Ok((old_graph, graph, type_graph))
}

/// Loose means that it doesn't have to be linked, only need it for the data
pub async fn get_graph_loose(
	project: &Project,
	refreshed_sources: &RefreshedSources,
) -> anyhow::Result<DependencyGraph> {
	let (_, graph, _) = get_graph_internal(project, refreshed_sources, false, true).await?;

	Ok(graph)
}

/// Strict means that it has to be unchanged (and so linked)
pub async fn get_graph_strict(
	project: &Project,
	refreshed_sources: &RefreshedSources,
) -> anyhow::Result<DependencyGraph> {
	let (_, graph, _) = get_graph_internal(project, refreshed_sources, true, true).await?;

	Ok(graph)
}

pub async fn install(
	options: &InstallOptions,
	project: &Project,
	reqwest: reqwest::Client,
) -> anyhow::Result<()> {
	let start = Instant::now();

	let refreshed_sources = RefreshedSources::new();

	let manifest = project
		.clone()
		.subproject(Importer::root())
		.deser_manifest()
		.await
		.context("failed to read manifest")?;

	let resolved_engine_versions =
		Arc::new(super::get_project_engines(&manifest, &reqwest, project.auth_config()).await?);

	let (new_lockfile, old_graph, type_graph) =
		reporters::run_with_reporter(|multi, root_progress, reporter| async {
			let multi = multi;
			let root_progress = root_progress;

			root_progress.reset();
			root_progress.set_message("resolve");

			let (old_graph, mut graph, type_graph) = get_graph_internal(
				project,
				&refreshed_sources,
				options.locked,
				options.use_lockfile,
			)
			.await?;

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
							.read_index_file(name.clone(), &project)
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

				project
					.download_and_link(
						&mut graph,
						DownloadAndLinkOptions::<CliReporter, InstallHooks>::new(reqwest.clone())
							.reporter(reporter)
							.hooks(InstallHooks {
								project: project.clone(),
								#[cfg(feature = "global-binaries")]
								global_binaries: crate::cli::config::read_config()
									.await?
									.global_binaries,
								#[cfg(not(feature = "global-binaries"))]
								global_binaries: false,
							})
							.refreshed_sources(refreshed_sources.clone())
							.install_dependencies_mode(options.install_dependencies_mode)
							.network_concurrency(options.network_concurrency)
							.force(options.force)
							.engines(resolved_engine_versions.clone()),
					)
					.await
					.context("failed to download and link dependencies")?;

				#[cfg(feature = "version-management")]
				#[expect(deprecated)]
				{
					use pesde::engine::EngineKind;
					use pesde::source::PackageRefs;
					use pesde::version_matches;

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
											.read_index_file(pkg_ref.name.clone(), &project)
											.await
											.context("failed to read package index file")?
											.context("package not found in index")?;

										file.entries
											.remove(id.v_id())
											.context("package version not found in index")?
											.engines
									}
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

			let new_lockfile = Lockfile { graph };

			project
				.write_lockfile(&new_lockfile)
				.await
				.context("failed to write lockfile")?;

			anyhow::Ok((new_lockfile, old_graph, type_graph))
		})
		.await?;

	let elapsed = start.elapsed();

	print_install_summary(old_graph, new_lockfile.graph, type_graph.as_ref());

	println!("done in {:.2}s", elapsed.as_secs_f64());

	Ok(())
}

pub fn print_install_summary(
	old_graph: Option<DependencyGraph>,
	new_graph: DependencyGraph,
	type_graph: Option<&DependencyTypeGraph>,
) {
	let old_importers = old_graph
		.map_or(BTreeMap::new(), |old_graph| old_graph.importers)
		.into_iter();
	let new_importers = new_graph.importers.into_iter();

	let importer_pairs = old_importers
		.merge_join_by(new_importers, |(old_importer, _), (new_importer, _)| {
			old_importer.cmp(new_importer)
		})
		.map(|m| match m {
			EitherOrBoth::Both((importer, old), (_, new)) => {
				(importer, old.dependencies, new.dependencies)
			}
			EitherOrBoth::Left((importer, old)) => (importer, old.dependencies, BTreeMap::new()),
			EitherOrBoth::Right((importer, new)) => (importer, BTreeMap::new(), new.dependencies),
		});

	for (importer, old, new) in importer_pairs {
		let mut peer_warnings = vec![];

		if let Some((type_graph, dependencies)) = &type_graph.and_then(|type_graph| {
			type_graph
				.importers
				.get(&importer)
				.map(|dependencies| (type_graph, dependencies))
		}) {
			for (alias, id) in *dependencies {
				let Some(node) = type_graph.nodes.get(id) else {
					continue;
				};

				let mut queue = node
					.dependencies
					.iter()
					.map(|(dep_alias, (dep_id, dep_ty))| {
						(vec![(id, alias)], (dep_id, dep_alias), *dep_ty)
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
								new_graph.nodes[id]
									.dependencies
									.values()
									.any(|id| id == dep_id)
							})
						} else {
							dependencies.iter().any(|(_, node_id)| node_id == dep_id)
						};

						if !satisfied {
							peer_warnings.push(
								style(format!(
									"missing peer {}>{dep_alias}",
									path.iter().map(|(_, alias)| alias.as_str()).format(">"),
								))
								.red(),
							);
						}
					}

					if let Some(dep_node) = type_graph.nodes.get(dep_id) {
						queue.extend(dep_node.dependencies.iter().map(
							|(inner_dep_alias, (inner_dep_id, inner_dep_ty))| {
								(
									path.iter()
										.copied()
										.chain(std::iter::once((dep_id, dep_alias)))
										.collect(),
									(inner_dep_id, inner_dep_alias),
									*inner_dep_ty,
								)
							},
						));
					}
				}
			}
		}

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

		println!("{}", style(importer).bold());

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
				);
			}
		}

		if !peer_warnings.is_empty() {
			for msg in peer_warnings {
				println!("  {msg}");
			}
		}

		println!();
	}
}
