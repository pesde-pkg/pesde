use std::collections::BTreeMap;

use crate::cli::install::get_graph_loose;
use crate::cli::style::ADDED_STYLE;
use crate::cli::style::INFO_STYLE;
use crate::cli::style::REMOVED_STYLE;
use crate::cli::style::SUCCESS_STYLE;
use anyhow::Context as _;
use clap::Args;
use console::style;
use itertools::Either;
use pesde::RefreshedSources;
use pesde::Subproject;
use pesde::manifest::Alias;
use pesde::source::git::specifier::GitVersionSpecifier;
use pesde::source::ids::VersionId;
use pesde::source::specifiers::DependencySpecifiers;
use pesde::source::traits::PackageSource as _;
use pesde::source::traits::RefreshOptions;
use pesde::source::traits::ResolveOptions;
use semver::VersionReq;
use tokio::task::JoinSet;

#[derive(Debug, Args)]
pub struct OutdatedCommand {
	/// Whether to check within version requirements
	#[arg(short, long)]
	strict: bool,
}

impl OutdatedCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let refreshed_sources = RefreshedSources::new();
		let mut graph = get_graph_loose(subproject.project(), &refreshed_sources).await?;

		let refreshed_sources = RefreshedSources::new();

		let mut tasks =
			if subproject.importer().is_root() {
				Either::Left(graph.importers.into_iter().map(|(importer, deps)| {
					(subproject.project().clone().subproject(importer), deps)
				}))
			} else {
				Either::Right(
					graph
						.importers
						.remove(subproject.importer())
						.map(|deps| (subproject, deps))
						.into_iter(),
				)
			}
			.map(|(subproject, importer_data)| {
				let refreshed_sources = refreshed_sources.clone();
				async move {
					let manifest = subproject
						.deser_manifest()
						.await
						.context("failed to read manifest")?;
					let manifest_target_kind = manifest.target.kind();

					let mut tasks = importer_data
						.dependencies
						.into_iter()
						.filter(|(_, (_, spec, _))| !spec.is_local())
						.map(|(alias, (id, mut spec, _))| {
							let subproject = subproject.clone();
							let refreshed_sources = refreshed_sources.clone();
							if !self.strict {
								match &mut spec {
									#[expect(deprecated)]
									DependencySpecifiers::Pesde(spec) => {
										spec.version = VersionReq::STAR;
									}
									#[cfg(feature = "wally-compat")]
									DependencySpecifiers::Wally(spec) => {
										spec.version = VersionReq::STAR;
									}
									DependencySpecifiers::Git(spec) => {
										if matches!(
											spec.version_specifier,
											GitVersionSpecifier::VersionReq(_)
										) {
											spec.version_specifier =
												GitVersionSpecifier::VersionReq(VersionReq::STAR);
										}
									}
									DependencySpecifiers::Path(_) => {}
								}
							}
							async move {
								refreshed_sources
									.refresh(
										id.source(),
										&RefreshOptions {
											project: subproject.project().clone(),
										},
									)
									.await
									.context("failed to refresh source")?;

								let new_v_id = id
									.source()
									.resolve(
										&spec,
										&ResolveOptions {
											subproject: subproject.clone(),
											target: manifest_target_kind,
											refreshed_sources: refreshed_sources.clone(),
											loose_target: false,
										},
									)
									.await
									.context("failed to resolve package versions")?
									.2
									.pop_last()
									.map(|(v_id, _)| v_id)
									.with_context(|| format!("no versions of {spec} found"))?;

								Ok(Some((alias, id.v_id().clone(), new_v_id))
									.filter(|(_, current_id, new_id)| current_id != new_id))
							}
						})
						.collect::<JoinSet<Result<_, anyhow::Error>>>();

					let mut updates = BTreeMap::<Alias, (VersionId, VersionId)>::new();
					while let Some(task) = tasks.join_next().await {
						let Some((alias, current_id, new_id)) = task.unwrap()? else {
							continue;
						};
						updates.insert(alias, (current_id, new_id));
					}

					Ok::<_, anyhow::Error>((subproject.importer().clone(), updates))
				}
			})
			.collect::<JoinSet<Result<_, anyhow::Error>>>();

		let mut importer_updates = BTreeMap::new();

		while let Some(task) = tasks.join_next().await {
			let (importer, updates) = task.unwrap()?;
			if updates.is_empty() {
				continue;
			}

			importer_updates.insert(importer, updates);
		}

		if importer_updates.is_empty() {
			println!("{}", SUCCESS_STYLE.apply_to("all packages are up to date"));
			return Ok(());
		}

		for (importer, updates) in importer_updates {
			println!("{}", style(importer).bold());

			for (alias, (current_id, new_id)) in updates {
				println!(
					"  {} {} → {}",
					INFO_STYLE.apply_to(alias),
					REMOVED_STYLE.apply_to(current_id),
					ADDED_STYLE.apply_to(new_id),
				);
			}
		}

		Ok(())
	}
}
