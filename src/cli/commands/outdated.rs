use std::collections::BTreeMap;

use crate::cli::{
	style::{ADDED_STYLE, INFO_STYLE, REMOVED_STYLE, SUCCESS_STYLE},
	up_to_date_lockfile,
};
use anyhow::Context as _;
use clap::Args;
use console::style;
use pesde::{
	Project, RefreshedSources,
	manifest::Alias,
	source::{
		git::specifier::GitVersionSpecifier,
		ids::VersionId,
		specifiers::DependencySpecifiers,
		traits::{PackageSource as _, RefreshOptions, ResolveOptions},
	},
};
use semver::VersionReq;
use tokio::task::JoinSet;

#[derive(Debug, Args)]
pub struct OutdatedCommand {
	/// Whether to check within version requirements
	#[arg(short, long)]
	strict: bool,
}

impl OutdatedCommand {
	pub async fn run(self, project: Project) -> anyhow::Result<()> {
		let graph = match up_to_date_lockfile(&project).await? {
			Some(file) => file.graph,
			None => {
				anyhow::bail!(
					"lockfile is out of sync, run `{} install` to update it",
					env!("CARGO_BIN_NAME")
				);
			}
		};

		let manifest = project
			.deser_manifest()
			.await
			.context("failed to read manifest")?;
		let manifest_target_kind = manifest.target.kind();

		let refreshed_sources = RefreshedSources::new();

		let mut tasks = graph
			.importers
			.into_iter()
			.flat_map(|(importer, dependencies)| {
				dependencies
					.into_iter()
					.map(move |(alias, (id, spec, _))| (importer.clone(), alias, id, spec))
			})
			.filter(|(_, _, _, spec)| !spec.is_local())
			.map(|(importer, alias, id, mut spec)| {
				let project = project.clone();
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
							if matches!(spec.version_specifier, GitVersionSpecifier::VersionReq(_))
							{
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
								project: project.clone(),
							},
						)
						.await
						.context("failed to refresh source")?;

					let new_v_id = id
						.source()
						.resolve(
							&spec,
							&ResolveOptions {
								project: project.clone(),
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

					Ok(Some((importer, alias, id.v_id().clone(), new_v_id))
						.filter(|(_, _, current_id, new_id)| current_id != new_id))
				}
			})
			.collect::<JoinSet<Result<_, anyhow::Error>>>();

		let mut importer_updates = BTreeMap::<_, BTreeMap<Alias, (VersionId, VersionId)>>::new();

		while let Some(task) = tasks.join_next().await {
			let Some((importer, alias, current_id, new_id)) = task.unwrap()? else {
				continue;
			};

			importer_updates
				.entry(importer)
				.or_default()
				.insert(alias, (current_id, new_id));
		}

		if importer_updates.is_empty() {
			println!("{}", SUCCESS_STYLE.apply_to("all packages are up to date"));
			return Ok(());
		}

		for (importer, updates) in importer_updates {
			println!(
				"{}",
				style(if importer.as_str().is_empty() {
					"(root)"
				} else {
					importer.as_str()
				})
				.bold()
			);

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
