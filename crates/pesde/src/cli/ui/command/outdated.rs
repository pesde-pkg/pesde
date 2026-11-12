use std::borrow::Cow;
use std::collections::BTreeMap;

use clap::Args;
use console::style;
use futures::TryStreamExt as _;
use itertools::Either;
use pesde::Importer;
use pesde::registry::manifest::Alias;
use pesde::source::DependencySpecifier;
use pesde::source::PackageId;
use pesde::source::PackageIdComparison;
use semver::VersionReq;
use tokio::task::JoinSet;

use crate::cli::install::get_lockfile;
use crate::cli::ui::CommandContext;

/// Checks for outdated dependencies
#[derive(Debug, Args)]
pub struct OutdatedCommand {
	/// Whether to check within version requirements
	#[arg(short, long)]
	strict: bool,
}

impl OutdatedCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let member = ctx.member().await?.clone();
		let project = ctx.project().await?;
		let mut resolution = get_lockfile(project).await?;

		let mut tasks = if member.importer().is_root() {
			Either::Left(
				resolution
					.importers
					.into_iter()
					.map(|(importer, resolution_importer)| {
						(
							member.workspace().clone().member(importer),
							resolution_importer,
						)
					}),
			)
		} else {
			Either::Right(
				resolution
					.importers
					.remove(member.importer())
					.map(|resolution_importer| (member, resolution_importer))
					.into_iter(),
			)
		}
		.flat_map(|(member, importer_data)| {
			importer_data
				.dependencies
				.into_iter()
				// .filter(|(_, (_, spec, _))| !spec.is_local())
				.map(move |(alias, (id, spec, _))| (member.clone(), alias, id, spec))
		})
		.filter_map(|(member, alias, current_id, mut spec)| {
			if !self.strict {
				match &mut spec {
					DependencySpecifier::Pesde(spec) => {
						spec.req = VersionReq::STAR;
					}
					#[expect(deprecated)]
					DependencySpecifier::LegacyPesde(spec) => {
						spec.req = VersionReq::STAR;
					}
					DependencySpecifier::Wally(spec) => {
						spec.req = VersionReq::STAR;
					}
					DependencySpecifier::Git(_) => {}
					DependencySpecifier::Workspace(_) => {
						return None;
					}
				}
			}

			let project = project.clone();

			Some(async move {
				let versions = crate::cli::ui::specifier::resolve(&project, spec).await?;

				let new = versions
					.try_fold(Cow::Borrowed(&current_id), async |acc, (candidate, _)| {
						match candidate.compare(&acc) {
							PackageIdComparison::Comparable(ordering) => {
								if ordering == std::cmp::Ordering::Greater {
									Ok(Cow::Owned(candidate))
								} else {
									Ok(acc)
								}
							}
							PackageIdComparison::Incomparable => {
								// updated Git revision
								Ok(Cow::Owned(candidate))
							}
							PackageIdComparison::NotSameSource => {
								anyhow::bail!(
									"resolved package `{candidate}` is not of the same source as previously resolved package `{acc}`"
								)
							}
						}
					})
					.await?;

				let Cow::Owned(new) = new else {
					tracing::debug!("no versions found for `{current_id}`");
					return Ok(None);
				};

				// Git will return the same revision
				if new == current_id {
					return Ok(None);
				}

				Ok(Some((member, alias, current_id, new)))
			})
		})
		.collect::<JoinSet<anyhow::Result<_>>>();

		let mut importer_updates = BTreeMap::<Importer, BTreeMap<Alias, (String, String)>>::new();

		while let Some(task) = tasks.join_next().await {
			let Some((subproject, alias, current_id, new_id)) = task.unwrap()? else {
				continue;
			};

			let version = |id: &PackageId| match id {
				PackageId::Pesde(_, _, version) => version.to_string(),
				PackageId::LegacyPesde(_, _, version, _) => version.to_string(),
				PackageId::Wally(_, _, version) => version.to_string(),
				PackageId::Git(_, rev, _) => rev.to_string(),
				PackageId::Workspace(_) => unreachable!(),
			};

			importer_updates
				.entry(subproject.importer().clone())
				.or_default()
				.insert(alias, (version(&current_id), version(&new_id)));
		}

		if importer_updates.is_empty() {
			println!("all packages are up to date");
			return Ok(());
		}

		for (importer, updates) in importer_updates {
			println!("{}", style(importer).bold());

			for (alias, (current_version, new_version)) in updates {
				println!("\t{alias} {current_version} → {new_version}");
			}
		}

		Ok(())
	}
}
