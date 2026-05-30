use std::collections::BTreeMap;

use crate::cli::install::get_lockfile;
use crate::cli::style::ADDED_STYLE;
use crate::cli::style::INFO_STYLE;
use crate::cli::style::REMOVED_STYLE;
use crate::cli::style::SUCCESS_STYLE;
use anyhow::Context as _;
use clap::Args;
use console::style;
use itertools::Either;
use pesde::Importer;
use pesde::RefreshedSources;
use pesde::Subproject;
use pesde::manifest::Alias;
use pesde::source::DependencySpecifiers;
use pesde::source::PackageSource as _;
use semver::Version;
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
		let mut lockfile = get_lockfile(subproject.project(), &refreshed_sources).await?;

		let refreshed_sources = RefreshedSources::new();

		let mut tasks = if subproject.importer().is_root() {
			Either::Left(
				lockfile
					.graph
					.importers
					.into_iter()
					.map(|(importer, deps)| {
						(subproject.project().clone().subproject(importer), deps)
					}),
			)
		} else {
			Either::Right(
				lockfile
					.graph
					.importers
					.remove(subproject.importer())
					.map(|deps| (subproject, deps))
					.into_iter(),
			)
		}
		.flat_map(|(subproject, importer_data)| {
			importer_data
				.dependencies
				.into_iter()
				.filter(|(_, (_, spec, _))| !spec.is_local())
				.map(move |(alias, (id, spec, _))| (subproject.clone(), alias, id, spec))
		})
		.map(|(subproject, alias, id, mut spec)| {
			let refreshed_sources = refreshed_sources.clone();
			if !self.strict {
				match &mut spec {
					DependencySpecifiers::Pesde(spec) => {
						spec.version = VersionReq::STAR;
					}
					#[expect(deprecated)]
					DependencySpecifiers::LegacyPesde(spec) => {
						spec.version = VersionReq::STAR;
					}
					DependencySpecifiers::Wally(spec) => {
						spec.version = VersionReq::STAR;
					}
					DependencySpecifiers::Git(_) => {}
					DependencySpecifiers::Path(_) => {}
				}
			}

			let old_state = lockfile.source_states.get(id.source()).cloned();

			async move {
				let Some(old_state) = old_state else {
					anyhow::bail!("source state not found for {}", id.source());
				};

				let source_state = refreshed_sources
					.refresh(id.source(), subproject.project(), Some(&old_state))
					.await
					.context("failed to refresh source")?;

				let new_version = id
					.source()
					.resolve(&subproject, &source_state, &spec, &refreshed_sources)
					.await
					.context("failed to resolve package versions")?
					.versions
					.pop_last()
					.map(|(version, _)| version)
					.with_context(|| format!("no versions of {spec} found"))?;

				if id.version() == &new_version {
					return Ok(None);
				}

				Ok(Some((subproject, alias, id.version().clone(), new_version)))
			}
		})
		.collect::<JoinSet<Result<_, anyhow::Error>>>();

		let mut importer_updates = BTreeMap::<Importer, BTreeMap<Alias, (Version, Version)>>::new();

		while let Some(task) = tasks.join_next().await {
			let Some((subproject, alias, current_version, new_version)) = task.unwrap()? else {
				continue;
			};

			importer_updates
				.entry(subproject.importer().clone())
				.or_default()
				.insert(alias, (current_version, new_version));
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
