use std::{
	collections::{BTreeMap, BTreeSet, HashMap},
	num::NonZeroUsize,
	sync::Arc,
	time::Instant,
};

use super::files::make_executable;
use crate::cli::{
	bin_dir,
	reporters::{self, CliReporter},
	resolve_overrides, run_on_workspace_members, up_to_date_lockfile,
};
use anyhow::Context;
use colored::Colorize;
use fs_err::tokio as fs;
use pesde::{
	download_and_link::{DownloadAndLinkHooks, DownloadAndLinkOptions},
	graph::{DependencyGraph, DependencyGraphWithTarget},
	lockfile::Lockfile,
	manifest::{target::TargetKind, DependencyType},
	Project, RefreshedSources, LOCKFILE_FILE_NAME, MANIFEST_FILE_NAME,
};
use tokio::task::JoinSet;

fn bin_link_file(alias: &str) -> String {
	let mut all_combinations = BTreeSet::new();

	for a in TargetKind::VARIANTS {
		for b in TargetKind::VARIANTS {
			all_combinations.insert((a, b));
		}
	}

	let all_folders = all_combinations
		.into_iter()
		.map(|(a, b)| format!("{:?}", a.packages_folder(b)))
		.collect::<BTreeSet<_>>()
		.into_iter()
		.collect::<Vec<_>>()
		.join(", ");

	format!(
		include_str!("bin_link.luau"),
		alias = alias,
		all_folders = all_folders,
		MANIFEST_FILE_NAME = MANIFEST_FILE_NAME,
		LOCKFILE_FILE_NAME = LOCKFILE_FILE_NAME
	)
}

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
		let mut tasks = graph
			.values()
			.filter(|node| node.target.bin_path().is_some())
			.filter_map(|node| node.node.direct.as_ref())
			.map(|(alias, _, _)| alias)
			.filter(|alias| {
				if *alias == env!("CARGO_BIN_NAME") {
					tracing::warn!(
						"package {alias} has the same name as the CLI, skipping bin link"
					);
					return false;
				}
				true
			})
			.map(|alias| {
				let bin_folder = self.bin_folder.clone();
				let alias = alias.clone();

				async move {
					let bin_exec_file = bin_folder
						.join(&alias)
						.with_extension(std::env::consts::EXE_EXTENSION);

					let impl_folder = bin_folder.join(".impl");
					fs::create_dir_all(&impl_folder)
						.await
						.context("failed to create bin link folder")?;

					let bin_file = impl_folder.join(&alias).with_extension("luau");
					fs::write(&bin_file, bin_link_file(&alias))
						.await
						.context("failed to write bin link file")?;

					#[cfg(windows)]
					match fs::symlink_file(
						std::env::current_exe().context("failed to get current executable path")?,
						&bin_exec_file,
					)
					.await
					{
						Ok(_) => {}
						Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
						e => e.context("failed to copy bin link file")?,
					}

					#[cfg(not(windows))]
					fs::write(
						&bin_exec_file,
						format!(
							r#"#!/bin/sh
exec lune run "$(dirname "$0")/.impl/{alias}.luau" -- "$@""#
						),
					)
					.await
					.context("failed to link bin link file")?;

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
	pub prod: bool,
	pub write: bool,
	pub use_lockfile: bool,
	pub network_concurrency: NonZeroUsize,
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
					None
				} else if lockfile.target != manifest.target.kind() {
					tracing::debug!("target kind is different");
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

	let overrides = resolve_overrides(&manifest)?;

	let (new_lockfile, old_graph) =
		reporters::run_with_reporter(|_, root_progress, reporter| async {
			let root_progress = root_progress;

			root_progress.set_prefix(format!("{} {}: ", manifest.name, manifest.target));
			#[cfg(feature = "version-management")]
			{
				root_progress.set_message("update engine linkers");

				let mut tasks = manifest
					.engines
					.keys()
					.map(|engine| crate::cli::version::make_linker_if_needed(*engine))
					.collect::<JoinSet<_>>();

				while let Some(task) = tasks.join_next().await {
					task.unwrap()?;
				}
			}

			root_progress.set_message("clean");

			if options.write {
				let mut deleted_folders = HashMap::new();

				for target_kind in TargetKind::VARIANTS {
					let folder = manifest.target.kind().packages_folder(target_kind);
					let package_dir = project.package_dir().to_path_buf();

					deleted_folders
						.entry(folder.to_string())
						.or_insert_with(|| async move {
							tracing::debug!("deleting the {folder} folder");

							if let Some(e) = fs::remove_dir_all(package_dir.join(&folder))
								.await
								.err()
								.filter(|e| e.kind() != std::io::ErrorKind::NotFound)
							{
								return Err(e)
									.context(format!("failed to remove the {folder} folder"));
							};

							Ok(())
						});
				}

				let mut tasks = deleted_folders.into_values().collect::<JoinSet<_>>();
				while let Some(task) = tasks.join_next().await {
					task.unwrap()?;
				}
			}

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
			let graph = Arc::new(graph);

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
							.reporter(
								#[cfg(feature = "patches")]
								reporter.clone(),
								#[cfg(not(feature = "patches"))]
								reporter,
							)
							.hooks(hooks)
							.refreshed_sources(refreshed_sources)
							.prod(options.prod)
							.network_concurrency(options.network_concurrency),
					)
					.await
					.context("failed to download and link dependencies")?;

				#[cfg(feature = "patches")]
				{
					use pesde::graph::ConvertableGraph;
					root_progress.reset();
					root_progress.set_length(0);
					root_progress.set_message("patch");

					project
						.apply_patches(&downloaded_graph.convert(), reporter)
						.await?;
				}
			}

			root_progress.set_message("finish");

			let new_lockfile = Lockfile {
				name: manifest.name.clone(),
				version: manifest.version,
				target: manifest.target.kind(),
				overrides,

				graph: Arc::into_inner(graph).unwrap(),

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
		old_graph,
		new_lockfile.graph,
	);

	println!("done in {:.2}s", elapsed.as_secs_f64());
	println!();

	Ok(())
}

/// Prints the difference between two graphs.
pub fn print_package_diff(prefix: &str, old_graph: DependencyGraph, new_graph: DependencyGraph) {
	let mut old_pkg_map = BTreeMap::new();
	let mut old_direct_pkg_map = BTreeMap::new();
	let mut new_pkg_map = BTreeMap::new();
	let mut new_direct_pkg_map = BTreeMap::new();

	for (id, node) in &old_graph {
		old_pkg_map.insert(id, node);
		if node.direct.is_some() {
			old_direct_pkg_map.insert(id, node);
		}
	}

	for (id, node) in &new_graph {
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

	let prefix = prefix.bold();

	let no_changes = added_pkgs.is_empty()
		&& removed_pkgs.is_empty()
		&& added_direct_pkgs.is_empty()
		&& removed_direct_pkgs.is_empty();

	if no_changes {
		println!("{prefix} already up to date");
	} else {
		let mut change_signs = [
			(!added_pkgs.is_empty()).then(|| format!("+{}", added_pkgs.len()).green().to_string()),
			(!removed_pkgs.is_empty())
				.then(|| format!("-{}", removed_pkgs.len()).red().to_string()),
		]
		.into_iter()
		.flatten()
		.collect::<Vec<_>>()
		.join(" ");

		let changes_empty = change_signs.is_empty();
		if changes_empty {
			change_signs = "(no changes)".dimmed().to_string();
		}

		println!("{prefix} {change_signs}");

		if !changes_empty {
			println!(
				"{}{}",
				"+".repeat(added_pkgs.len()).green(),
				"-".repeat(removed_pkgs.len()).red()
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

			let ty_name = match ty {
				DependencyType::Standard => "dependencies",
				DependencyType::Peer => "peer_dependencies",
				DependencyType::Dev => "dev_dependencies",
			};
			println!("{}", format!("{ty_name}:").yellow().bold());

			for (id, added) in set {
				println!(
					"{} {} {}",
					if added { "+".green() } else { "-".red() },
					id.name(),
					id.version_id().to_string().dimmed()
				);
			}
		}

		println!();
	}
}
