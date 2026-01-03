use crate::graph::{DependencyTypeGraph, DependencyTypeGraphNode};
#[expect(deprecated)]
use crate::{
	GixUrl, Project, RefreshedSources,
	graph::{DependencyGraph, DependencyGraphNode},
	manifest::{Alias, DependencyType, overrides::OverrideSpecifier},
	source::{
		PackageSources,
		ids::PackageId,
		pesde::PesdePackageSource,
		specifiers::DependencySpecifiers,
		traits::{PackageSource as _, RefreshOptions, ResolveOptions},
	},
};
use futures::StreamExt as _;
use relative_path::RelativePath;
use std::{
	collections::{HashMap, VecDeque},
	sync::Arc,
};
use tokio::pin;
use tracing::{Instrument as _, instrument};

impl Project {
	/// Create a dependency graph from the project's manifest
	#[instrument(
		skip(self, previous_graph, refreshed_sources),
		ret(level = "trace"),
		level = "debug"
	)]
	pub async fn dependency_graph(
		&self,
		previous_graph: Option<DependencyGraph>,
		refreshed_sources: RefreshedSources,
		// used by `x` command - if true, specifier indices are expected to be URLs
		is_published_package: bool,
	) -> Result<(DependencyGraph, Option<DependencyTypeGraph>), errors::DependencyGraphError> {
		// TODO: always recompute the type graph
		let mut graph = DependencyGraph {
			importers: Default::default(),
			nodes: Default::default(),
		};

		let mut queue = VecDeque::new();

		let members = self.workspace_members().await?;
		pin!(members);
		while let Some((importer, manifest)) = members.next().await.transpose()? {
			let importer: Arc<RelativePath> = importer.into();
			let manifest = Arc::new(manifest);
			let importer_entry = graph.importers.entry(importer.clone()).or_default();
			let all_current_dependencies = manifest.all_dependencies()?;

			let mut all_specifiers = all_current_dependencies
				.clone()
				.into_iter()
				.map(|(alias, (spec, ty))| ((spec, ty), alias))
				.collect::<HashMap<_, _>>();

			if let Some(previous_graph) = &previous_graph
				&& let Some(previous_importer) = previous_graph.importers.get(&importer)
			{
				for (old_alias, (package_id, specifier, source_ty)) in previous_importer {
					if specifier.is_local() {
						// local dependencies must always be resolved fresh in case their FS changes
						continue;
					}

					let Some(alias) = all_specifiers.remove(&(specifier.clone(), *source_ty))
					else {
						tracing::debug!(
							"dependency {package_id} (old alias {old_alias}) from old dependency graph is no longer in the manifest",
						);
						continue;
					};

					let span =
						tracing::info_span!("resolve from old graph", alias = alias.as_str());
					let _guard = span.enter();

					let mut queue = previous_graph.nodes[package_id]
						.dependencies
						.iter()
						.map(|(dep_alias, id)| (id, vec![alias.to_string(), dep_alias.to_string()]))
						.collect::<VecDeque<_>>();

					tracing::debug!("resolved {package_id} from old dependency graph");

					importer_entry
						.insert(alias, (package_id.clone(), specifier.clone(), *source_ty));

					graph
						.nodes
						.insert(package_id.clone(), previous_graph.nodes[package_id].clone());

					while let Some((dep_id, path)) = queue.pop_front() {
						let inner_span =
							tracing::info_span!("resolve dependency", path = path.join(">"));
						let _inner_guard = inner_span.enter();

						tracing::debug!("resolved sub-dependency {dep_id}");
						if graph.nodes.contains_key(dep_id) {
							tracing::debug!(
								"sub-dependency {dep_id} already resolved in new graph",
							);
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
										.chain(std::iter::once(alias.to_string()))
										.collect(),
								)
							})
							.for_each(|dep| queue.push_back(dep));
					}
				}
			}

			let all_current_dependencies = Arc::new(all_current_dependencies);

			queue.extend(all_specifiers.into_iter().map(|((spec, ty), alias)| {
				(
					Some(importer.clone()),
					manifest.clone(),
					all_current_dependencies.clone(),
					spec,
					ty,
					None::<PackageId>,
					vec![alias],
					false,
					manifest.target.kind(),
				)
			}));
		}

		let refresh_options = RefreshOptions {
			project: self.clone(),
		};

		let mut type_graph = None::<DependencyTypeGraph>;

		while let Some((
			importer,
			manifest,
			all_current_dependencies,
			specifier,
			ty,
			dependant,
			path,
			overridden,
			target,
		)) = queue.pop_front()
		{
			let type_graph = type_graph.get_or_insert_with(|| DependencyTypeGraph {
				importers: Default::default(),
				nodes: Default::default(),
			});

			async {
				let alias = path.last().unwrap();
				let depth = path.len() - 1;

				tracing::debug!("resolving {specifier} ({ty:?})");
				let source = match &specifier {
					#[expect(deprecated)]
					DependencySpecifiers::Pesde(specifier) => {
						let index_url = if !is_published_package && (depth == 0 || overridden) {
							manifest
								.indices
								.get(&specifier.index)
								.ok_or_else(|| {
									errors::DependencyGraphError::IndexNotFound(
										specifier.index.clone(),
									)
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
					#[cfg(feature = "wally-compat")]
					DependencySpecifiers::Wally(specifier) => {
						let index_url = if !is_published_package && (depth == 0 || overridden) {
							manifest
								.wally_indices
								.get(&specifier.index)
								.ok_or_else(|| {
									errors::DependencyGraphError::WallyIndexNotFound(
										specifier.index.clone(),
									)
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

						PackageSources::Wally(crate::source::wally::WallyPackageSource::new(
							index_url,
						))
					}
					DependencySpecifiers::Git(specifier) => PackageSources::Git(
						crate::source::git::GitPackageSource::new(specifier.repo.clone()),
					),
					DependencySpecifiers::Path(_) => {
						PackageSources::Path(crate::source::path::PathPackageSource)
					}
				};

				refreshed_sources.refresh(&source, &refresh_options).await?;

				let (source, pkg_ref, mut versions, suggestions) = source
					.resolve(
						&specifier,
						&ResolveOptions {
							project: self.clone(),
							target,
							refreshed_sources: refreshed_sources.clone(),
							loose_target: false,
						},
					)
					.await?;

				let Some((package_id, dependencies)) = graph
					.nodes
					.keys()
					.filter(|package_id| {
						*package_id.source() == source
							&& *package_id.pkg_ref() == pkg_ref
							&& versions.contains_key(package_id.v_id())
					})
					.max()
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
					return Err(errors::DependencyGraphError::NoMatchingVersion(format!(
						"{specifier} {target}{}",
						if suggestions.is_empty() {
							"".into()
						} else {
							format!(
								" available targets: {}",
								suggestions
									.into_iter()
									.map(|t| t.to_string())
									.collect::<Vec<_>>()
									.join(", ")
							)
						}
					)));
				};

				if let Some(importer) = &importer {
					graph
						.importers
						.entry(importer.clone())
						.or_default()
						.insert(alias.clone(), (package_id.clone(), specifier.clone(), ty));
					type_graph
						.importers
						.entry(importer.clone())
						.or_default()
						.insert(alias.clone(), package_id.clone());
				}

				if let Some(dependant_id) = dependant {
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
						.insert(alias.clone(), (package_id.clone(), ty));
				}

				if graph.nodes.get_mut(&package_id).is_some() {
					tracing::debug!("{package_id} already resolved");

					return Ok(());
				}

				graph.nodes.insert(
					package_id.clone(),
					DependencyGraphNode {
						dependencies: Default::default(),
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

					let overridden = manifest.overrides.iter().find_map(|(key, spec)| {
						key.0.iter().find_map(|override_path| {
							// if the path up until the last element is the same as the current path,
							// and the last element in the path is the dependency alias,
							// then the specifier is to be overridden
							(path.len() == override_path.len() - 1
								&& path == override_path[..override_path.len() - 1]
								&& override_path.last() == Some(&dependency_alias))
							.then_some(spec)
						})
					});

					if overridden.is_some() {
						tracing::debug!(
							"overridden specifier found for {} ({dependency_spec})",
							path.iter()
								.map(Alias::as_str)
								.chain(std::iter::once(dependency_alias.as_str()))
								.collect::<Vec<_>>()
								.join(">"),
						);
					}

					queue.push_back((
						None,
						manifest.clone(),
						all_current_dependencies.clone(),
						match overridden {
							Some(OverrideSpecifier::Specifier(spec)) => spec.clone(),
							Some(OverrideSpecifier::Alias(alias)) => all_current_dependencies
								.get(alias)
								.map(|(spec, _)| spec)
								.ok_or_else(|| {
									errors::DependencyGraphError::AliasNotFound(alias.clone())
								})?
								.clone(),
							None => dependency_spec,
						},
						dependency_ty,
						Some(package_id.clone()),
						path.iter()
							.cloned()
							.chain(std::iter::once(dependency_alias))
							.collect(),
						overridden.is_some(),
						package_id.v_id().target(),
					));
				}

				Ok(())
			}
			.instrument(tracing::info_span!(
				"resolve new/changed",
				path = path.iter().map(Alias::as_str).collect::<Vec<_>>().join(">")
			))
			.await?;
		}

		Ok((graph, type_graph))
	}
}

/// Errors that can occur when resolving dependencies
pub mod errors {
	use crate::manifest::Alias;
	use thiserror::Error;

	/// Errors that can occur when creating a dependency graph
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DependencyGraphError {
		/// An error occurred while accessing the workspace members
		#[error("error accessing workspace members")]
		WorkspaceMembers(#[from] crate::errors::WorkspaceMembersError),

		/// An error occurred while reading all dependencies from the manifest
		#[error("error getting all project dependencies")]
		AllDependencies(#[from] crate::manifest::errors::AllDependenciesError),

		/// An index was not found in the manifest
		#[error("index named `{0}` not found in manifest")]
		IndexNotFound(String),

		/// A Wally index was not found in the manifest
		#[cfg(feature = "wally-compat")]
		#[error("wally index named `{0}` not found in manifest")]
		WallyIndexNotFound(String),

		/// An error occurred while refreshing a package source
		#[error("error refreshing package source")]
		Refresh(#[from] crate::source::errors::RefreshError),

		/// An error occurred while resolving a package
		#[error("error resolving package")]
		Resolve(#[from] crate::source::errors::ResolveError),

		/// No matching version was found for a specifier
		#[error("no matching version found for {0}")]
		NoMatchingVersion(String),

		/// An alias for an override was not found in the manifest
		#[error("alias `{0}` not found in manifest")]
		AliasNotFound(Alias),
	}
}
