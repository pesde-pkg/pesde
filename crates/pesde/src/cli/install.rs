use anyhow::Context as _;
use console::style;
use itertools::EitherOrBoth;
use itertools::Itertools as _;
use pesde::Project;
use pesde::Resolution;
use pesde::install::InstallDependenciesMode;
use pesde::install::InstallOptions;
use pesde::registry::manifest::DependencyType;
use pesde::source::PackageId;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::time::Instant;

use crate::cli::reporters;
use crate::cli::reporters::CliReporter;
use crate::cli::ui::dependency_type_to_key;

#[derive(Debug, Clone, Copy)]
pub struct CliInstallOptions {
	pub locked: bool,
	pub install_dependencies_mode: InstallDependenciesMode,
	pub write: bool,
	pub use_lockfile: bool,
	pub force: bool,
}

async fn get_lockfile_internal(
	project: &Project,
	locked: bool,
	use_lockfile: bool,
) -> anyhow::Result<(Option<Resolution>, Resolution)> {
	let old_lockfile = if use_lockfile {
		match project.workspace().deser_lockfile().await {
			Ok(lockfile) => Some(lockfile),
			Err(e)
				if let pesde::ErrorKind::Vfs(_, e) = e.inner()
					&& e.kind() == std::io::ErrorKind::NotFound =>
			{
				None
			}
			Err(e) => {
				return Err(e.into());
			}
		}
	} else {
		None
	};

	let (new_lockfile, updated) = project
		.solve(old_lockfile.as_ref())
		.await
		.context("failed to build dependency graph")?;

	if updated && locked {
		anyhow::bail!(
			"lockfile is out of sync, run `{} install` without --locked to update it",
			env!("CARGO_BIN_NAME")
		);
	}

	Ok((old_lockfile, new_lockfile))
}

pub async fn get_lockfile(project: &Project) -> anyhow::Result<Resolution> {
	let (_, lockfile) = get_lockfile_internal(project, false, true).await?;

	Ok(lockfile)
}

// pub async fn get_lockfile_locked(project: &Project) -> anyhow::Result<Resolution> {
// 	let (_, lockfile) = get_lockfile_internal(project, true, true).await?;

// 	Ok(lockfile)
// }

pub async fn install(options: &CliInstallOptions, project: &Project) -> anyhow::Result<()> {
	let start = Instant::now();

	let (new_lockfile, old_lockfile) =
		reporters::run_with_reporter(async |_, root_progress, reporter| {
			let root_progress = root_progress;
			let reporter = reporter;

			root_progress.reset();
			root_progress.set_message("resolve");

			let (old_lockfile, new_lockfile) =
				get_lockfile_internal(project, options.locked, options.use_lockfile).await?;

			// #[expect(deprecated)]
			// let mut tasks = new_lockfile
			// 	.graph
			// 	.keys()
			// 	.filter_map(|id| {
			// 		let PackageSources::LegacyPesde(source) = id.source() else {
			// 			return None;
			// 		};
			// 		let PackageRefs::LegacyPesde(pkg_ref) = id.pkg_ref() else {
			// 			return None;
			// 		};
			// 		let source = source.clone();
			// 		let name = pkg_ref.name.clone();
			// 		let project = project.clone();
			// 		let refreshed_sources = refreshed_sources.clone();

			// 		Some(async move {
			// 			refreshed_sources
			// 				.refresh(&PackageSources::LegacyPesde(source.clone()), &project)
			// 				.await
			// 				.context("failed to refresh source")?;

			// 			let file = source
			// 				.repo()
			// 				.read_index_file(&project, name.clone())
			// 				.await
			// 				.context("failed to read package index file")?
			// 				.context("package not found in index")?;

			// 			Ok::<_, anyhow::Error>(if file.meta.deprecated.is_empty() {
			// 				None
			// 			} else {
			// 				Some((name, file.meta.deprecated))
			// 			})
			// 		})
			// 	})
			// 	.collect::<JoinSet<_>>();

			// while let Some(task) = tasks.join_next().await {
			// 	let Some((name, reason)) = task.unwrap()? else {
			// 		continue;
			// 	};

			// 	multi.suspend(|| {
			// 		println!("{WARN_PREFIX}: package {name} is deprecated: {reason}");
			// 	});
			// }

			if options.write {
				root_progress.reset();
				root_progress.set_length(0);
				root_progress.set_message("download");
				root_progress.set_style(reporters::root_progress_style_with_progress());

				project
					.install(
						&new_lockfile,
						InstallOptions::<CliReporter>::new()
							.reporter(reporter)
							.install_dependencies_mode(options.install_dependencies_mode)
							.force(options.force),
					)
					.await
					.context("failed to install dependencies")?;
			}

			root_progress.reset();
			root_progress.set_message("finish");

			project
				.workspace()
				.write_lockfile(&new_lockfile)
				.await
				.context("failed to write lockfile")?;

			anyhow::Ok((new_lockfile, old_lockfile))
		})
		.await?;

	let elapsed = start.elapsed();

	print_install_summary(old_lockfile, new_lockfile);

	println!("done in {:.2}s", elapsed.as_secs_f64());

	Ok(())
}

pub fn print_install_summary(old_resolution: Option<Resolution>, new_resolution: Resolution) {
	let old_importers = old_resolution
		.map_or(BTreeMap::new(), |old_resolution| old_resolution.importers)
		.into_iter();
	let new_importers = new_resolution.importers.into_iter();

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

		for (alias, (id, _, _)) in &new {
			let Some(node) = new_resolution.graph.get(id) else {
				continue;
			};

			let mut queue = node
				.dependencies
				.iter()
				.map(|(dep_alias, (dep_id, dep_ty))| {
					(vec![(id, alias)], (dep_id, dep_alias), dep_ty)
				})
				.collect::<Vec<_>>();

			while let Some((path, (dep_id, dep_alias), dep_ty)) = queue.pop() {
				if *dep_ty == DependencyType::Peer {
					let mut iter = path
						.iter()
						.map(|(id, _)| id)
						.rev()
						// skip our parent since we're always going to be descendants of it
						.skip(1)
						.take(2);

					let satisfied = if iter.len() > 0 {
						iter.any(|id| {
							new_resolution.graph[id]
								.dependencies
								.values()
								.any(|(inner_dep_id, _)| inner_dep_id == dep_id)
						})
					} else {
						new.iter().any(|(_, (node_id, _, _))| node_id == dep_id)
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

				if let Some(dep_node) = new_resolution.graph.get(dep_id) {
					queue.extend(dep_node.dependencies.iter().map(
						|(inner_dep_alias, (inner_dep_id, inner_dep_ty))| {
							(
								path.iter()
									.copied()
									.chain(std::iter::once((dep_id, dep_alias)))
									.collect(),
								(inner_dep_id, inner_dep_alias),
								inner_dep_ty,
							)
						},
					));
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
				old,
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

		let mut stdout = std::io::stdout().lock();

		writeln!(stdout, "{}", style(importer).bold()).unwrap();

		for (ty, changes) in groups {
			writeln!(
				stdout,
				"  {}",
				style(dependency_type_to_key(ty)).yellow().bold()
			)
			.unwrap();

			for (change, alias, id) in changes {
				let version = match &id {
					PackageId::Pesde(_, _, version) => format_args!(" v{}", *version),
					PackageId::Wally(_, _, version) => format_args!(" v{}", *version),
					PackageId::LegacyPesde(_, _, version, target) => {
						format_args!(" v{} {}", *version, *target)
					}
					PackageId::Git(_, revision, path) => {
						if path.is_empty() {
							format_args!(" {}", *revision)
						} else {
							format_args!(" {} ({})", *revision, *path)
						}
					}
					PackageId::Workspace(_) => format_args!(""),
				};

				let sign = match change {
					Change::Added =>
					/*ADDED_STYLE.apply_to(*/
					{
						"+"
					}
					Change::Removed =>
					/*REMOVED_STYLE.apply_to(*/
					{
						"-"
					}
				};

				writeln!(
					stdout,
					"    {sign} {alias}{} {}",
					style(version).cyan(),
					style(&id).dim()
				)
				.unwrap();
			}
		}

		if !peer_warnings.is_empty() {
			for msg in peer_warnings {
				writeln!(stdout, "\t{msg}").unwrap();
			}
		}

		writeln!(stdout).unwrap();
	}
}
