//! Resolving packages
use crate::Importer;
use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::graph::DependencyGraphImporter;
use crate::graph::DependencyGraphNode;
use crate::graph::DependencyGraphNodeDependency;
use crate::lockfile::Lockfile;
use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::manifest::ManifestUrls;
use crate::manifest::OverrideSpecifier;
use crate::matching_globs;
use crate::source::DependencySpecifier as _;
use crate::source::DependencySpecifiers;
use crate::source::PackageSource as _;
use crate::source::PackageSources;
use crate::source::ResolveResult;
use crate::source::StructureKind;
use crate::source::ids::PackageId;
#[expect(deprecated)]
use crate::source::legacy_pesde::LegacyPesdePackageSource;
use crate::source::pesde::PesdePackageSource;
use itertools::Itertools as _;
use relative_path::RelativePathBuf;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::task::JoinSet;
use tracing::Instrument as _;
use tracing::instrument;

fn specifier_to_source(
	urls: Option<&ManifestUrls>,
	specifier: &DependencySpecifiers,
) -> Result<PackageSources, errors::DependencyGraphError> {
	let source = match &specifier {
		DependencySpecifiers::Pesde(specifier) => {
			let url = if let Some(indices) = urls {
				indices
					.pesde_registries
					.get(&specifier.registry)
					.ok_or_else(|| {
						errors::DependencyGraphErrorKind::RegistryNotFound(
							specifier.registry.clone(),
						)
					})?
					.clone()
			} else {
				specifier
					.registry
					.as_str()
					.parse()
					.map(Arc::new)
					// specifiers in indices store the index url in this field
					.unwrap()
			};

			PackageSources::Pesde(PesdePackageSource::from_url(url))
		}
		#[expect(deprecated)]
		DependencySpecifiers::LegacyPesde(specifier) => {
			let index_url = if let Some(indices) = urls {
				indices
					.pesde_indices
					.get(&specifier.index)
					.ok_or_else(|| {
						errors::DependencyGraphErrorKind::IndexNotFound(specifier.index.clone())
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

			PackageSources::LegacyPesde(LegacyPesdePackageSource::from_url(index_url))
		}
		DependencySpecifiers::Wally(specifier) => {
			let index_url = if let Some(indices) = urls {
				indices
					.wally_indices
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

			PackageSources::Wally(crate::source::wally::WallyPackageSource::from_url(
				index_url,
			))
		}
		DependencySpecifiers::Git(specifier) => PackageSources::Git(
			crate::source::git::GitPackageSource::from_url(specifier.repo.clone()),
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
}

#[instrument(skip_all, level = "debug")]
async fn prepare_queue(
	project: &Project,
	lockfile: &mut Lockfile,
	previous_lockfile: Option<&Lockfile>,
) -> Result<VecDeque<ResolveEntry>, errors::DependencyGraphError> {
	let root_subproject = project.clone().subproject(Importer::root());
	let root_manifest = root_subproject.deser_manifest().await?;
	let root_dependencies = root_manifest.all_dependencies()?;

	lockfile.graph.overrides = root_manifest
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

	// overrides can affect the graph at any level, so it's much easier and safer to ignore the previous graph if there are differences in overrides
	let previous_graph = previous_lockfile
		.filter(|previous| previous.graph.overrides == lockfile.graph.overrides)
		.map(|lockfile| &lockfile.graph);

	let mut queue = VecDeque::<ResolveEntry>::new();

	while let Some(res) = members.join_next().await {
		let (subproject, manifest) = res.unwrap()?;
		let all_current_dependencies = manifest.all_dependencies()?;

		let importer_entry = lockfile
			.graph
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
					.map(|(dep_alias, dep)| (&dep.id, vec![alias.clone(), dep_alias.clone()]))
					.collect::<VecDeque<_>>();

				tracing::debug!("resolved {package_id} from old dependency graph");

				importer_entry
					.dependencies
					.insert(alias, (package_id.clone(), specifier.clone(), *source_ty));

				lockfile
					.graph
					.nodes
					.insert(package_id.clone(), previous_graph.nodes[package_id].clone());

				while let Some((dep_id, path)) = queue.pop_front() {
					let inner_span =
						tracing::info_span!("resolve dependency", path = path.iter().join(">"));
					let _inner_guard = inner_span.enter();

					tracing::debug!("resolved sub-dependency {dep_id}");
					if lockfile.graph.nodes.contains_key(dep_id) {
						tracing::debug!("sub-dependency {dep_id} already resolved in new graph");
						continue;
					}
					lockfile
						.graph
						.nodes
						.insert(dep_id.clone(), previous_graph.nodes[dep_id].clone());

					previous_graph.nodes[dep_id]
						.dependencies
						.iter()
						.map(|(alias, dep)| {
							(
								&dep.id,
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
				}),
		);
	}

	Ok(queue)
}

#[instrument(skip_all, level = "debug")]
async fn resolve_version(
	subproject: Subproject,
	lockfile: &mut Lockfile,
	previous_lockfile: Option<&Lockfile>,
	refreshed_sources: &RefreshedSources,
	pass_indices: bool,
	specifier: &DependencySpecifiers,
) -> Result<
	(
		PackageId,
		StructureKind,
		BTreeMap<Alias, (DependencySpecifiers, DependencyType)>,
	),
	errors::DependencyGraphError,
> {
	let mut manifest = None;

	let mut inner = async |specifier: &DependencySpecifiers, pass_indices: bool| {
		if pass_indices && manifest.is_none() {
			manifest = Some(subproject.deser_manifest().await?);
		}
		let source = specifier_to_source(manifest.as_ref().map(|m| &m.urls), specifier)?;

		refreshed_sources
			.refresh_index(&source, subproject.project())
			.await?;

		let current_state = lockfile.source_states.entry(source.clone());
		let current_state =
			if let std::collections::btree_map::Entry::Occupied(entry) = &current_state {
				entry.get()
			} else {
				let new_state = source
					.refresh_state(
						subproject.project(),
						previous_lockfile.and_then(|l| l.source_states.get(&source)),
					)
					.await?;

				current_state.or_insert(new_state)
			};

		let ResolveResult {
			source,
			pkg_ref,
			structure_kind,
			mut versions,
		} = source
			.resolve(&subproject, current_state, specifier, refreshed_sources)
			.await?;

		let Some((package_id, dependencies)) = lockfile
			.graph
			.nodes
			.keys()
			.rfind(|package_id| {
				*package_id.source() == source
					&& *package_id.pkg_ref() == pkg_ref
					&& versions.contains_key(package_id.version())
			})
			.map(|package_id| {
				(
					package_id.clone(),
					versions.remove(package_id.version()).unwrap(),
				)
			})
			.or_else(|| {
				versions.pop_last().map(|(version, dependencies)| {
					(PackageId::new(source, pkg_ref, version), dependencies)
				})
			})
		else {
			return Err(
				errors::DependencyGraphErrorKind::NoMatchingVersion(specifier.clone()).into(),
			);
		};

		Ok::<_, errors::DependencyGraphError>((package_id, structure_kind, dependencies))
	};

	let (id, structure_kind, deps) = inner(specifier, pass_indices).await?;
	if let Some(specifier) = lockfile.graph.overrides.get(&id) {
		return inner(specifier, true).await;
	}
	Ok((id, structure_kind, deps))
}

impl Project {
	/// Solves the project's dependency requirements and creates a new [Lockfile]
	#[instrument(
		skip(self, previous_lockfile, refreshed_sources),
		ret(level = "trace"),
		level = "debug"
	)]
	pub async fn solve(
		&self,
		previous_lockfile: Option<&Lockfile>,
		refreshed_sources: &RefreshedSources,
		// used by `x` command - if true, specifier indices are expected to be URLs
		is_published_package: bool,
	) -> Result<(Lockfile, bool), errors::DependencyGraphError> {
		let mut lockfile = Lockfile {
			graph: Default::default(),
			source_states: previous_lockfile
				.map(|l| l.source_states.clone())
				.unwrap_or_default(),
		};

		let mut queue = prepare_queue(self, &mut lockfile, previous_lockfile).await?;
		if queue.is_empty() {
			tracing::debug!("dependency graph is up to date");
			return Ok((lockfile, false));
		}

		while let Some(entry) = queue.pop_front() {
			async {
				let alias = entry.path.last().unwrap();
				let depth = entry.path.len() - 1;

				tracing::debug!("resolving {} ({:?})", entry.specifier, entry.ty);

				let (package_id, structure_kind, dependencies) = resolve_version(
					entry.subproject.clone(),
					&mut lockfile,
					previous_lockfile,
					refreshed_sources,
					!is_published_package && depth == 0,
					&entry.specifier,
				)
				.await?;

				if depth == 0 {
					lockfile
						.graph
						.importers
						.get_mut(entry.subproject.importer())
						.unwrap()
						.dependencies
						.insert(
							alias.clone(),
							(package_id.clone(), entry.specifier.clone(), entry.ty),
						);
				}

				if let Some(dependant_id) = entry.dependant {
					lockfile
						.graph
						.nodes
						.get_mut(&dependant_id)
						.expect("dependant package not found in graph")
						.dependencies
						.insert(
							alias.clone(),
							DependencyGraphNodeDependency {
								id: package_id.clone(),
								ty: entry.ty,
								realm: entry.specifier.realm(),
							},
						);
				}

				if lockfile.graph.nodes.contains_key(&package_id) {
					tracing::debug!("{package_id} already resolved");

					return Ok(());
				}

				lockfile.graph.nodes.insert(
					package_id.clone(),
					DependencyGraphNode {
						dependencies: Default::default(),
						// will be filled out later, we don't have this information at this step
						checksum: Default::default(),
						structure_kind,
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

		Ok((lockfile, true))
	}
}

/// Errors that can occur when resolving dependencies
pub mod errors {
	use crate::errors::MatchingGlobsError;
	use crate::manifest::Alias;
	use crate::source::DependencySpecifiers;
	use thiserror::Error;

	/// Errors that can occur when solving dependencies
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

		/// A registry was not found in the manifest
		#[error("registry `{0}` not found in manifest")]
		RegistryNotFound(String),

		/// An index was not found in the manifest
		#[error("index named `{0}` not found in manifest")]
		IndexNotFound(String),

		/// A Wally index was not found in the manifest
		#[error("wally index named `{0}` not found in manifest")]
		WallyIndexNotFound(String),

		/// An error occurred while refreshing a package source index
		#[error("error refreshing package source index")]
		RefreshIndex(#[from] crate::source::errors::RefreshIndexError),

		/// An error occurred while resolving a package
		#[error("error resolving package")]
		Resolve(#[from] crate::source::errors::ResolveError),

		/// An error occurred while refreshing source state
		#[error("error refreshing source state")]
		RefreshState(#[from] crate::source::errors::RefreshStateError),

		/// No matching version was found for a specifier
		#[error("no matching version found for {0}")]
		NoMatchingVersion(DependencySpecifiers),

		/// An alias for an override was not found in the manifest
		#[error("alias `{0}` not found in manifest")]
		AliasNotFound(Alias),
	}
}
