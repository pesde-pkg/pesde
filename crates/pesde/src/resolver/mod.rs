use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

use crate::registry::manifest::Alias;
use crate::registry::manifest::DependencyType;
use crate::source::InspectResult;
use crate::source::PackageIdComparison;
use futures::TryStreamExt as _;
use itertools::Itertools as _;
use tracing::Instrument as _;
use tracing::instrument;

use crate::Error;
use crate::Importer;
use crate::Project;
use crate::Result;
use crate::link::PackageStructure;
use crate::source::DependencySpecifier;
use crate::source::PackageId;
use crate::source::ResolveResult;

mod state;
pub use state::*;

struct ResolveEntry {
	specifier: DependencySpecifier,
	ty: DependencyType,
	user: ResolveEntryUser,
	path: Vec<Vec<Alias>>,
}

enum ResolveEntryUser {
	Importer(Importer),
	Dependant(PackageId),
}

#[instrument(skip_all, level = "debug")]
async fn prepare_queue(
	project: &Project,
	resolution: &mut Resolution,
	previous_resolution: Option<&Resolution>,
) -> Result<VecDeque<ResolveEntry>> {
	// overrides can affect the graph at any level, so it's much easier and safer to ignore the previous graph if there are differences in overrides
	let previous_resolution =
		previous_resolution.filter(|previous| previous.overrides == resolution.overrides);

	let mut members = project.workspace().members_with_manifests().await;

	let mut queue = VecDeque::<ResolveEntry>::new();

	let mut all_specifiers = HashMap::<_, Vec<Alias>>::new();

	while let Some((subproject, manifest)) =
		members.join_next().await.transpose().unwrap().transpose()?
	{
		let importer_entry = resolution
			.importers
			.entry(subproject.importer().clone())
			.or_insert_with(|| ResolutionImporter {
				dependencies: Default::default(),
			});

		for (alias, (spec, ty)) in manifest.dependencies.iter() {
			all_specifiers
				.entry((spec.clone(), *ty))
				.or_default()
				.push(alias.clone());
		}

		if let Some(previous_resolution) = &previous_resolution
			&& let Some(previous_importer) =
				previous_resolution.importers.get(subproject.importer())
		{
			for (old_alias, (package_id, specifier, source_ty)) in &previous_importer.dependencies {
				// if specifier.is_local() {
				// 	// local dependencies must always be resolved fresh in case their FS changes
				// 	continue;
				// }

				let Some(aliases) = all_specifiers.remove(&(specifier.clone(), *source_ty)) else {
					tracing::debug!(
						"dependency {package_id} (old alias {old_alias}) from old dependency graph is no longer in the manifest",
					);
					continue;
				};

				let span = tracing::info_span!(
					"resolve from old graph",
					aliases = aliases.iter().join(", ")
				);
				let _guard = span.enter();

				for alias in &aliases {
					importer_entry.dependencies.insert(
						alias.clone(),
						(package_id.clone(), specifier.clone(), *source_ty),
					);
				}

				let node = &previous_resolution.graph[package_id];
				if resolution
					.graph
					.insert(package_id.clone(), node.clone())
					.is_some()
				{
					tracing::debug!("dependency {package_id} already resolved in new graph");
					continue;
				}

				let mut queue = node
					.dependencies
					.iter()
					.map(|(dep_alias, (dep_id, _))| {
						(dep_id, vec![aliases.clone(), vec![dep_alias.clone()]])
					})
					.collect::<VecDeque<_>>();

				tracing::debug!("resolved {package_id} from old dependency graph");

				while let Some((dep_id, path)) = queue.pop_front() {
					let inner_span = tracing::info_span!(
						"resolve dependency",
						path = path.iter().map(|p| p.iter().format(", ")).join(">")
					);
					let _inner_guard = inner_span.enter();

					tracing::debug!("resolved sub-dependency {dep_id}");
					if resolution.graph.contains_key(dep_id) {
						tracing::debug!("sub-dependency {dep_id} already resolved in new graph");
						continue;
					}
					resolution
						.graph
						.insert(dep_id.clone(), previous_resolution.graph[dep_id].clone());

					previous_resolution.graph[dep_id]
						.dependencies
						.iter()
						.map(|(alias, (dep_id, _))| {
							(
								dep_id,
								path.iter()
									.cloned()
									.chain(std::iter::once(vec![alias.clone()]))
									.collect(),
							)
						})
						.for_each(|dep| queue.push_back(dep));
				}
			}
		}

		queue.extend(
			all_specifiers
				.drain()
				.map(|((specifier, ty), aliases)| ResolveEntry {
					user: ResolveEntryUser::Importer(subproject.importer().clone()),
					specifier,
					ty,
					path: vec![aliases],
				}),
		);
	}

	Ok(queue)
}

#[instrument(skip_all, level = "debug")]
async fn resolve_version(
	project: &Project,
	resolution: &Resolution,
	specifier: &DependencySpecifier,
) -> Result<(
	PackageId,
	PackageStructure,
	BTreeMap<Alias, (DependencySpecifier, DependencyType)>,
)> {
	let mut visited = HashSet::new();
	// most sources won't return Elsewhere
	let mut source_ids = VecDeque::with_capacity(1);

	let mut inner = async |specifier: &DependencySpecifier| {
		visited.clear();
		source_ids.clear();
		source_ids.push_back(specifier.source_id());

		let (mut candidates, source) = 'finder: {
			while let Some(source_id) = source_ids.pop_front() {
				if !visited.insert(source_id.clone()) {
					return Err(
						ResolveError::SourceRedirectionLoop(Box::new(specifier.clone())).into(),
					);
				}

				let source = project
					.source_provider()
					.get(&source_id)
					.await
					.map_err(Error::source)?;
				match source.resolve(specifier).await.map_err(Error::source)? {
					ResolveResult::Found(candidates) => break 'finder (candidates, source),
					ResolveResult::Elsewhere(new_source_ids) => {
						source_ids.extend(new_source_ids);
					}
				}
			}

			return Err(ResolveError::NoVersionsFound(Box::new(specifier.clone())).into());
		};

		let mut chosen_candidate = None::<(PackageId, Option<InspectResult>)>;
		let mut chosen_already_in_graph = false;
		while let Some(candidate) = candidates.try_next().await.map_err(Error::source)? {
			let set = if let Some((previous_max, _)) = &chosen_candidate {
				let newer = match candidate.0.compare(previous_max) {
					PackageIdComparison::Comparable(ordering) => {
						ordering == std::cmp::Ordering::Greater
					}
					PackageIdComparison::Incomparable => false,
					PackageIdComparison::NotSameSource => {
						return Err(ResolveError::InvalidPackageType(
							Box::new(specifier.clone()),
							Box::new(previous_max.clone()),
							Box::new(candidate.0),
						)
						.into());
					}
				};

				// if both are already in the graph, we prefer the newer one
				// if only the current candidate is already in the graph, we prefer it to reuse as many of the existing nodes as possible
				if resolution.graph.contains_key(&candidate.0) {
					if chosen_already_in_graph { newer } else { true }
				} else {
					newer && !chosen_already_in_graph
				}
			} else {
				true
			};

			if set {
				chosen_already_in_graph = resolution.graph.contains_key(&candidate.0);
				chosen_candidate = Some(candidate);
			}
		}

		let Some((package_id, inspect_result)) = chosen_candidate else {
			return Err(ResolveError::NoVersionsFound(Box::new(specifier.clone())).into());
		};

		Ok::<_, Error>((source, package_id, inspect_result))
	};

	let mut result = inner(specifier).await?;
	if let Some(specifier) = resolution.overrides.get(&result.1) {
		result = inner(specifier).await?;
	}

	let (source, package_id, inspect_result) = result;

	let inspect = match inspect_result {
		Some(inspect_result) => inspect_result,
		None => source.inspect(&package_id).await.map_err(Error::source)?,
	};

	Ok((package_id, inspect.structure, inspect.dependencies))
}

impl Project {
	/// Solves the project's dependency requirements and creates a new [Resolution]
	#[instrument(skip(self, previous_resolution), ret(level = "trace"), level = "debug")]
	pub async fn solve(
		&self,
		previous_resolution: Option<&Resolution>,
	) -> Result<(Resolution, bool)> {
		let root_manifest = self.member(Importer::root()).deser_manifest().await?;

		let mut resolution = Resolution {
			importers: Default::default(),
			overrides: root_manifest.workspace.overrides.clone(),
			graph: Default::default(),
		};

		let mut queue = prepare_queue(self, &mut resolution, previous_resolution).await?;
		if queue.is_empty() {
			tracing::debug!("dependency graph is up to date");
			return Ok((resolution, false));
		}

		while let Some(entry) = queue.pop_front() {
			async {
				let aliases = entry.path.last().unwrap();

				tracing::debug!("resolving {} ({:?})", entry.specifier, entry.ty);

				let (package_id, structure, dependencies) =
					resolve_version(self, &resolution, &entry.specifier).await?;

				match entry.user {
					ResolveEntryUser::Importer(importer) => {
						let dependencies = &mut resolution
							.importers
							.get_mut(&importer)
							.unwrap()
							.dependencies;

						for alias in aliases {
							dependencies.insert(
								alias.clone(),
								(package_id.clone(), entry.specifier.clone(), entry.ty),
							);
						}
					}
					ResolveEntryUser::Dependant(dependant_id) => {
						let dependencies = &mut resolution
							.graph
							.get_mut(&dependant_id)
							.expect("dependant package not found in graph")
							.dependencies;

						for alias in aliases {
							dependencies.insert(alias.clone(), (package_id.clone(), entry.ty));
						}
					}
				}

				if resolution.graph.contains_key(&package_id) {
					tracing::debug!("{package_id} already resolved");

					return Ok(());
				}

				resolution.graph.insert(
					package_id.clone(),
					DependencyGraphNode {
						dependencies: Default::default(),
						structure: Some(structure),
					},
				);

				tracing::debug!("resolved {package_id} from new dependency graph");

				for (dependency_alias, (dependency_spec, dependency_ty)) in dependencies {
					if dependency_ty == DependencyType::Dev {
						// dev dependencies of dependencies are to be ignored
						continue;
					}

					queue.push_back(ResolveEntry {
						specifier: dependency_spec,
						ty: dependency_ty,
						user: ResolveEntryUser::Dependant(package_id.clone()),
						path: entry
							.path
							.iter()
							.cloned()
							.chain(std::iter::once(vec![dependency_alias]))
							.collect(),
					});
				}

				Ok::<_, Error>(())
			}
			.instrument(tracing::info_span!(
				"resolve new/changed",
				path = entry.path.iter().map(|p| p.iter().format(", ")).join(">")
			))
			.await?;
		}

		Ok((resolution, true))
	}
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ResolveError {
	#[error("source redirection loop detected while resolving `{0}`")]
	SourceRedirectionLoop(Box<DependencySpecifier>),

	#[error("no versions found for specifier `{0}`")]
	NoVersionsFound(Box<DependencySpecifier>),

	#[error("invalid package type resolved for `{0}`, got both {1} and {2}")]
	InvalidPackageType(Box<DependencySpecifier>, Box<PackageId>, Box<PackageId>),
}
