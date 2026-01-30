use crate::GixUrl;
use crate::Importer;
use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::graph::DependencyGraph;
use crate::graph::DependencyGraphImporter;
use crate::graph::DependencyGraphNode;
use crate::graph::DependencyTypeGraph;
use crate::graph::DependencyTypeGraphNode;
use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::manifest::ManifestIndices;
use crate::manifest::OverrideSpecifier;
use crate::manifest::target::TargetKind;
use crate::matching_globs;
use crate::source::Dependencies;
use crate::source::DependencyProvider as _;
use crate::source::DependencySpecifiers;
use crate::source::PackageSources;
use crate::source::ids::PackageId;
#[expect(deprecated)]
use crate::source::pesde::PesdePackageSource;
use crate::source::traits::PackageSource as _;
use crate::source::traits::RefreshOptions;
use crate::source::traits::ResolveOptions;
use itertools::Itertools as _;
use relative_path::RelativePathBuf;
use std::collections::HashMap;
use std::collections::VecDeque;
use tokio::task::JoinSet;
use tracing::Instrument as _;
use tracing::instrument;

fn specifier_to_source(
	indices: Option<&ManifestIndices>,
	specifier: &DependencySpecifiers,
) -> Result<PackageSources, errors::DependencyGraphError> {
	let source = match &specifier {
		#[expect(deprecated)]
		DependencySpecifiers::Pesde(specifier) => {
			let index_url = if let Some(indices) = indices {
				indices
					.pesde
					.get(&specifier.index)
					.ok_or_else(|| {
						errors::DependencyGraphErrorKind::IndexNotFound(specifier.index.clone())
					})?
					.clone()
			} else {
				specifier
					.index
					.as_str()
					.try_into()
					.map(GixUrl::new)
					// specifiers in indices store the index url in this field
					.unwrap()
			};

			PackageSources::Pesde(PesdePackageSource::new(index_url))
		}
		DependencySpecifiers::Wally(specifier) => {
			let index_url = if let Some(indices) = indices {
				indices
					.wally
					.get(&specifier.index)
					.ok_or_else(|| {
						errors::DependencyGraphErrorKind::WallyIndexNotFound(
							specifier.index.clone(),
						)
					})?
					.clone()
			} else {
				specifier
					.index
					.as_str()
					.parse()
					// specifiers in indices store the index url in this field
					.unwrap()
			};

			PackageSources::Wally(crate::source::wally::WallyPackageSource::new(index_url))
		}
		DependencySpecifiers::Git(specifier) => PackageSources::Git(
			crate::source::git::GitPackageSource::new(specifier.repo.clone()),
		),
		DependencySpecifiers::Path(_) => {
			PackageSources::Path(crate::source::path::PathPackageSource)
		}
	};

	Ok(source)
}

struct ResolveEntry {
	subproject: Subproject,
	specifier: DependencySpecifiers,
	ty: DependencyType,
	dependant: Option<PackageId>,
	path: Vec<Alias>,
	target: TargetKind,
}

#[instrument(skip_all, level = "debug")]
async fn prepare_queue(
	project: &Project,
	graph: &mut DependencyGraph,
	previous_graph: Option<&DependencyGraph>,
) -> Result<VecDeque<ResolveEntry>, errors::DependencyGraphError> {
	let root_subproject = project.clone().subproject(Importer::root());
	let root_manifest = root_subproject.deser_manifest().await?;
	let root_dependencies = root_manifest.all_dependencies()?;

	graph.overrides = root_manifest
		.workspace
		.overrides
		.iter()
		.map(|(id, spec)| {
			Ok((
				id.clone(),
				match spec {
					OverrideSpecifier::Alias(alias) => root_dependencies
						.get(alias)
						.ok_or_else(|| {
							errors::DependencyGraphErrorKind::AliasNotFound(alias.clone())
						})?
						.0
						.clone(),
					OverrideSpecifier::Specifier(spec) => spec.clone(),
				},
			))
		})
		.collect::<Result<_, errors::DependencyGraphError>>()?;

	let members = matching_globs(
		project.dir(),
		root_manifest.workspace.members.iter().map(String::as_str),
	)
	.await?;

	let mut members = members
		.into_iter()
		.map(|path| {
			project.clone().subproject(Importer::new(
				RelativePathBuf::from_path(path.strip_prefix(project.dir()).unwrap()).unwrap(),
			))
		})
		.chain(std::iter::once(root_subproject.clone()))
		.map(|subproject| async move {
			let manifest = subproject.deser_manifest().await?;
			Ok((subproject, manifest))
		})
		.collect::<JoinSet<Result<_, errors::DependencyGraphError>>>();

	// TODO: handle this more efficiently
	let previous_graph = previous_graph.filter(|previous| previous.overrides == graph.overrides);

	let mut queue = VecDeque::<ResolveEntry>::new();

	while let Some(res) = members.join_next().await {
		let (subproject, manifest) = res.unwrap()?;
		let all_current_dependencies = manifest.all_dependencies()?;

		let importer_entry = graph
			.importers
			.entry(subproject.importer().clone())
			.or_insert_with(|| DependencyGraphImporter {
				dependencies: Default::default(),
			});

		let mut all_specifiers = all_current_dependencies
			.clone()
			.into_iter()
			.map(|(alias, (spec, ty))| ((spec, ty), alias))
			.collect::<HashMap<_, _>>();

		if let Some(previous_graph) = &previous_graph
			&& let Some(previous_importer) = previous_graph.importers.get(subproject.importer())
		{
			for (old_alias, (package_id, specifier, source_ty)) in &previous_importer.dependencies {
				if specifier.is_local() {
					// local dependencies must always be resolved fresh in case their FS changes
					continue;
				}

				let Some(alias) = all_specifiers.remove(&(specifier.clone(), *source_ty)) else {
					tracing::debug!(
						"dependency {package_id} (old alias {old_alias}) from old dependency graph is no longer in the manifest",
					);
					continue;
				};

				let span = tracing::info_span!("resolve from old graph", alias = alias.as_str());
				let _guard = span.enter();

				let mut queue = previous_graph.nodes[package_id]
					.dependencies
					.iter()
					.map(|(dep_alias, id)| (id, vec![alias.clone(), dep_alias.clone()]))
					.collect::<VecDeque<_>>();

				tracing::debug!("resolved {package_id} from old dependency graph");

				importer_entry
					.dependencies
					.insert(alias, (package_id.clone(), specifier.clone(), *source_ty));

				graph
					.nodes
					.insert(package_id.clone(), previous_graph.nodes[package_id].clone());

				while let Some((dep_id, path)) = queue.pop_front() {
					let inner_span =
						tracing::info_span!("resolve dependency", path = path.iter().join(">"));
					let _inner_guard = inner_span.enter();

					tracing::debug!("resolved sub-dependency {dep_id}");
					if graph.nodes.contains_key(dep_id) {
						tracing::debug!("sub-dependency {dep_id} already resolved in new graph",);
						continue;
					}
					graph
						.nodes
						.insert(dep_id.clone(), previous_graph.nodes[dep_id].clone());

					previous_graph.nodes[dep_id]
						.dependencies
						.iter()
						.map(|(alias, id)| {
							(
								id,
								path.iter()
									.cloned()
									.chain(std::iter::once(alias.clone()))
									.collect(),
							)
						})
						.for_each(|dep| queue.push_back(dep));
				}
			}
		}

		queue.extend(
			all_specifiers
				.into_iter()
				.map(|((spec, ty), alias)| ResolveEntry {
					subproject: subproject.clone(),
					specifier: spec,
					ty,
					dependant: None,
					path: vec![alias],
					target: manifest.target.kind(),
				}),
		);
	}

	Ok(queue)
}

#[instrument(skip_all, level = "debug")]
async fn resolve_version(
	subproject: Subproject,
	graph: &DependencyGraph,
	refreshed_sources: &RefreshedSources,
	pass_indices: bool,
	specifier: &DependencySpecifiers,
	target: TargetKind,
) -> Result<(PackageId, Dependencies), errors::DependencyGraphError> {
	let mut manifest = None;

	let mut inner = async |specifier: &DependencySpecifiers, pass_indices: bool| {
		if pass_indices && manifest.is_none() {
			manifest = Some(subproject.deser_manifest().await?);
		}
		let source = specifier_to_source(manifest.as_ref().map(|m| &m.indices), specifier)?;

		refreshed_sources
			.refresh(
				&source,
				&RefreshOptions {
					project: subproject.project().clone(),
				},
			)
			.await?;

		let (source, pkg_ref, mut versions) = source
			.resolve(
				specifier,
				&ResolveOptions {
					subproject: subproject.clone(),
					target,
					refreshed_sources: refreshed_sources.clone(),
					loose_target: false,
				},
			)
			.await?;

		let Some((package_id, dependencies)) = graph
			.nodes
			.keys()
			.rfind(|package_id| {
				*package_id.source() == source
					&& *package_id.pkg_ref() == pkg_ref
					&& versions.contains_key(package_id.v_id())
			})
			.map(|package_id| {
				(
					package_id.clone(),
					versions.remove(package_id.v_id()).unwrap(),
				)
			})
			.or_else(|| {
				versions.pop_last().map(|(v_id, dependencies)| {
					(PackageId::new(source, pkg_ref, v_id), dependencies)
				})
			})
		else {
			return Err(errors::DependencyGraphErrorKind::NoMatchingVersion(
				specifier.clone(),
				target,
			)
			.into());
		};

		Ok::<_, errors::DependencyGraphError>((package_id, dependencies))
	};

	let mut package = inner(specifier, pass_indices).await?;
	if let Some(specifier) = graph.overrides.get(&package.0) {
		package = inner(specifier, true).await?;
	}

	let dependencies = package.1.dependencies(package.0.pkg_ref()).await?;
	Ok((package.0, dependencies))
}

impl Project {
	/// Create a dependency graph from the project's manifest
	#[instrument(
		skip(self, previous_graph, refreshed_sources),
		ret(level = "trace"),
		level = "debug"
	)]
	pub async fn dependency_graph(
		&self,
		previous_graph: Option<&DependencyGraph>,
		refreshed_sources: &RefreshedSources,
		// used by `x` command - if true, specifier indices are expected to be URLs
		is_published_package: bool,
	) -> Result<(DependencyGraph, Option<DependencyTypeGraph>, bool), errors::DependencyGraphError>
	{
		// TODO: always recompute the type graph
		let mut graph = DependencyGraph {
			importers: Default::default(),
			overrides: Default::default(),
			nodes: Default::default(),
		};

		let mut queue = prepare_queue(self, &mut graph, previous_graph).await?;
		if queue.is_empty() {
			tracing::debug!("dependency graph is up to date");
			return Ok((graph, None, false));
		}

		let mut type_graph = DependencyTypeGraph {
			importers: Default::default(),
			nodes: Default::default(),
		};

		while let Some(entry) = queue.pop_front() {
			async {
				let alias = entry.path.last().unwrap();
				let depth = entry.path.len() - 1;

				tracing::debug!("resolving {} ({:?})", entry.specifier, entry.ty);

				let (package_id, dependencies) = resolve_version(
					entry.subproject.clone(),
					&graph,
					refreshed_sources,
					!is_published_package && depth == 0,
					&entry.specifier,
					entry.target,
				)
				.await?;

				if depth == 0 {
					graph
						.importers
						.get_mut(entry.subproject.importer())
						.unwrap()
						.dependencies
						.insert(
							alias.clone(),
							(package_id.clone(), entry.specifier.clone(), entry.ty),
						);
					type_graph
						.importers
						.entry(entry.subproject.importer().clone())
						.or_default()
						.insert(alias.clone(), package_id.clone());
				}

				if let Some(dependant_id) = entry.dependant {
					graph
						.nodes
						.get_mut(&dependant_id)
						.expect("dependant package not found in graph")
						.dependencies
						.insert(alias.clone(), package_id.clone());
					type_graph
						.nodes
						.get_mut(&dependant_id)
						.expect("dependant package not found in type graph")
						.dependencies
						.insert(alias.clone(), (package_id.clone(), entry.ty));
				}

				if graph.nodes.contains_key(&package_id) {
					tracing::debug!("{package_id} already resolved");

					return Ok(());
				}

				graph.nodes.insert(
					package_id.clone(),
					DependencyGraphNode {
						dependencies: Default::default(),
						// will be filled out later, we don't have this information at this step
						checksum: Default::default(),
					},
				);
				type_graph.nodes.insert(
					package_id.clone(),
					DependencyTypeGraphNode {
						dependencies: Default::default(),
					},
				);

				tracing::debug!("resolved {package_id} from new dependency graph");

				for (dependency_alias, (dependency_spec, dependency_ty)) in dependencies {
					if dependency_ty == DependencyType::Dev {
						// dev dependencies of dependencies are to be ignored
						continue;
					}

					queue.push_back(ResolveEntry {
						subproject: entry.subproject.clone(),
						specifier: dependency_spec,
						ty: dependency_ty,
						dependant: Some(package_id.clone()),
						path: entry
							.path
							.iter()
							.cloned()
							.chain(std::iter::once(dependency_alias))
							.collect(),
						target: package_id.v_id().target(),
					});
				}

				Ok::<_, errors::DependencyGraphError>(())
			}
			.instrument(tracing::info_span!(
				"resolve new/changed",
				path = entry.path.iter().map(Alias::as_str).join(">")
			))
			.await?;
		}

		Ok((graph, Some(type_graph), true))
	}
}

/// Errors that can occur when resolving dependencies
pub mod errors {
	use crate::errors::MatchingGlobsError;
	use crate::manifest::Alias;
	use crate::manifest::target::TargetKind;
	use crate::source::DependencySpecifiers;
	use thiserror::Error;

	/// Errors that can occur when creating a dependency graph
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DependencyGraphError))]
	#[non_exhaustive]
	pub enum DependencyGraphErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred while globbing
		#[error("error globbing")]
		Globbing(#[from] MatchingGlobsError),

		/// An error occurred while reading all dependencies from the manifest
		#[error("error getting all project dependencies")]
		AllDependencies(#[from] crate::manifest::errors::AllDependenciesError),

		/// An index was not found in the manifest
		#[error("index named `{0}` not found in manifest")]
		IndexNotFound(String),

		/// A Wally index was not found in the manifest
		#[error("wally index named `{0}` not found in manifest")]
		WallyIndexNotFound(String),

		/// An error occurred while refreshing a package source
		#[error("error refreshing package source")]
		Refresh(#[from] crate::source::errors::RefreshError),

		/// An error occurred while resolving a package
		#[error("error resolving package")]
		Resolve(#[from] crate::source::errors::ResolveError),

		/// No matching version was found for a specifier
		#[error("no matching version found for {0} {1}")]
		NoMatchingVersion(DependencySpecifiers, TargetKind),

		/// Querying the dependencies
		#[error("error querying dependencies")]
		PackageDependencies(#[from] crate::source::errors::PackageDependenciesError),

		/// An alias for an override was not found in the manifest
		#[error("alias `{0}` not found in manifest")]
		AliasNotFound(Alias),
	}
}
