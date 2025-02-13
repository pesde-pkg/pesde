use crate::cli::{
	style::{ADDED_STYLE, INFO_STYLE, REMOVED_STYLE, SUCCESS_STYLE},
	up_to_date_lockfile,
};
use anyhow::Context;
use clap::Args;
use pesde::{
	source::{
		specifiers::DependencySpecifiers,
		traits::{PackageRef, PackageSource, RefreshOptions, ResolveOptions},
	},
	Project, RefreshedSources,
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
			.into_iter()
			.map(|(current_id, node)| {
				let project = project.clone();
				let refreshed_sources = refreshed_sources.clone();
				async move {
					let Some((alias, mut specifier, _)) = node.direct else {
						return Ok::<_, anyhow::Error>(None);
					};

					if matches!(
						specifier,
						DependencySpecifiers::Git(_)
							| DependencySpecifiers::Workspace(_)
							| DependencySpecifiers::Path(_)
					) {
						return Ok(None);
					}

					let source = node.pkg_ref.source();
					refreshed_sources
						.refresh(
							&source,
							&RefreshOptions {
								project: project.clone(),
							},
						)
						.await?;

					if !self.strict {
						match &mut specifier {
							DependencySpecifiers::Pesde(spec) => {
								spec.version = VersionReq::STAR;
							}
							#[cfg(feature = "wally-compat")]
							DependencySpecifiers::Wally(spec) => {
								spec.version = VersionReq::STAR;
							}
							DependencySpecifiers::Git(_) => {}
							DependencySpecifiers::Workspace(_) => {}
							DependencySpecifiers::Path(_) => {}
						};
					}

					let new_id = source
						.resolve(
							&specifier,
							&ResolveOptions {
								project: project.clone(),
								target: manifest_target_kind,
								refreshed_sources: refreshed_sources.clone(),
							},
						)
						.await
						.context("failed to resolve package versions")?
						.1
						.pop_last()
						.map(|(v_id, _)| v_id)
						.with_context(|| format!("no versions of {specifier} found"))?;

					Ok(Some((alias, current_id, new_id))
						.filter(|(_, current_id, new_id)| current_id.version_id() != new_id))
				}
			})
			.collect::<JoinSet<_>>();

		let mut all_up_to_date = true;

		while let Some(task) = tasks.join_next().await {
			let Some((alias, current_id, new_id)) = task.unwrap()? else {
				continue;
			};

			all_up_to_date = false;

			println!(
				"{} ({}) {} → {}",
				current_id.name(),
				INFO_STYLE.apply_to(alias),
				REMOVED_STYLE.apply_to(current_id.version_id()),
				ADDED_STYLE.apply_to(new_id),
			);
		}

		if all_up_to_date {
			println!("{}", SUCCESS_STYLE.apply_to("all packages are up to date"));
		}

		Ok(())
	}
}
