use crate::{
	graph::{DependencyGraph, DependencyGraphNode},
	manifest::{overrides::OverrideSpecifier, Alias, DependencyType},
	source::{
		ids::PackageId,
		pesde::PesdePackageSource,
		specifiers::DependencySpecifiers,
		traits::{PackageRef, PackageSource, RefreshOptions, ResolveOptions},
		PackageSources,
	},
	Project, RefreshedSources, DEFAULT_INDEX_NAME,
};
use std::collections::{btree_map::Entry, HashMap, VecDeque};
use tracing::{instrument, Instrument};

fn insert_node(
	graph: &mut DependencyGraph,
	package_id: &PackageId,
	mut node: DependencyGraphNode,
	is_top_level: bool,
) {
	if !is_top_level && node.direct.take().is_some() {
		tracing::debug!(
			"tried to insert {package_id} as direct dependency from a non top-level context",
		);
	}

	match graph.entry(package_id.clone()) {
		Entry::Vacant(entry) => {
			entry.insert(node);
		}
		Entry::Occupied(existing) => {
			let current_node = existing.into_mut();

			match (&current_node.direct, &node.direct) {
				(Some(_), Some(_)) => {
					tracing::warn!("duplicate direct dependency for {package_id}");
				}

				(None, Some(_)) => {
					current_node.direct = node.direct;
				}

				(_, _) => {}
			}
		}
	}
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
		refreshed_sources: RefreshedSources,
		// used by `x` command - if true, specifier indices are expected to be URLs. will not do peer dependency checks
		is_published_package: bool,
	) -> Result<DependencyGraph, Box<errors::DependencyGraphError>> {
		let manifest = self
			.deser_manifest()
			.await
			.map_err(|e| Box::new(e.into()))?;

		let all_current_dependencies = manifest
			.all_dependencies()
			.map_err(|e| Box::new(e.into()))?;

		let mut all_specifiers = all_current_dependencies
			.clone()
			.into_iter()
			.map(|(alias, (spec, ty))| ((spec, ty), alias))
			.collect::<HashMap<_, _>>();

		let mut graph = DependencyGraph::default();

		if let Some(previous_graph) = previous_graph {
			for (package_id, node) in previous_graph {
				let Some((old_alias, specifier, source_ty)) = &node.direct else {
					// this is not a direct dependency, will be added if it's still being used later
					continue;
				};

				if matches!(specifier, DependencySpecifiers::Workspace(_)) {
					// workspace dependencies must always be resolved brand new
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

				tracing::debug!("resolved {package_id} from old dependency graph");
				insert_node(
					&mut graph,
					package_id,
					DependencyGraphNode {
						direct: Some((alias.clone(), specifier.clone(), *source_ty)),
						..node.clone()
					},
					true,
				);

				let mut queue = node
					.dependencies
					.iter()
					.map(|(id, dep_alias)| (id, vec![alias.to_string(), dep_alias.to_string()]))
					.collect::<VecDeque<_>>();

				while let Some((dep_id, path)) = queue.pop_front() {
					let inner_span =
						tracing::info_span!("resolve dependency", path = path.join(">"));
					let _inner_guard = inner_span.enter();

					if let Some(dep_node) = previous_graph.get(dep_id) {
						tracing::debug!("resolved sub-dependency {dep_id}");
						if graph.contains_key(dep_id) {
							tracing::debug!(
								"sub-dependency {dep_id} already resolved in new graph",
							);
							continue;
						}
						insert_node(&mut graph, dep_id, dep_node.clone(), false);

						dep_node
							.dependencies
							.iter()
							.map(|(id, alias)| {
								(
									id,
									path.iter()
										.cloned()
										.chain(std::iter::once(alias.to_string()))
										.collect(),
								)
							})
							.for_each(|dep| queue.push_back(dep));
					} else {
						tracing::warn!("dependency {dep_id} not found in previous graph");
					}
				}
			}
		}

		let mut queue = all_specifiers
			.into_iter()
			.map(|((spec, ty), alias)| {
				(
					spec,
					ty,
					None::<PackageId>,
					vec![alias],
					false,
					manifest.target.kind(),
				)
			})
			.collect::<VecDeque<_>>();

		let refresh_options = RefreshOptions {
			project: self.clone(),
		};

		while let Some((specifier, ty, dependant, path, overridden, target)) = queue.pop_front() {
			async {
                let alias = path.last().unwrap();
                let depth = path.len() - 1;

                tracing::debug!("resolving {specifier} ({ty:?})");
                let source = match &specifier {
                    DependencySpecifiers::Pesde(specifier) => {
                        let index_url = if !is_published_package && (depth == 0 || overridden) {
                            let index_name = specifier.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME);

                            manifest
                                .indices
                                .get(index_name)
                                .ok_or(errors::DependencyGraphError::IndexNotFound(
                                    index_name.to_string(),
                                ))?
                                .clone()
                        } else {
                            specifier.index.as_deref().unwrap()
                                .try_into()
                                // specifiers in indices store the index url in this field
                                .unwrap()
                        };

                        PackageSources::Pesde(PesdePackageSource::new(index_url))
                    }
                    #[cfg(feature = "wally-compat")]
                    DependencySpecifiers::Wally(specifier) => {
                        let index_url = if !is_published_package && (depth == 0 || overridden) {
                            let index_name = specifier.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME);

                            manifest
                                .wally_indices
                                .get(index_name)
                                .ok_or(errors::DependencyGraphError::WallyIndexNotFound(
                                    index_name.to_string(),
                                ))?
                                .clone()
                        } else {
                            specifier.index.as_deref().unwrap()
                                .try_into()
                                // specifiers in indices store the index url in this field
                                .unwrap()
                        };

                        PackageSources::Wally(crate::source::wally::WallyPackageSource::new(index_url))
                    }
                    DependencySpecifiers::Git(specifier) => PackageSources::Git(
                        crate::source::git::GitPackageSource::new(specifier.repo.clone()),
                    ),
                    DependencySpecifiers::Workspace(_) => {
                        PackageSources::Workspace(crate::source::workspace::WorkspacePackageSource)
                    }
                    DependencySpecifiers::Path(_) => {
                        PackageSources::Path(crate::source::path::PathPackageSource)
                    }
                };

                refreshed_sources.refresh(
                    &source,
                    &refresh_options,
                )
                .await
                .map_err(|e| Box::new(e.into()))?;

                let (name, resolved) = source
                    .resolve(&specifier, &ResolveOptions {
                        project: self.clone(),
                        target,
                        refreshed_sources: refreshed_sources.clone(),
                    })
                    .await
                    .map_err(|e| Box::new(e.into()))?;

                let Some(package_id) = graph
                    .keys()
                    .filter(|id| *id.name() == name && resolved.contains_key(id.version_id()))
                    .max()
                    .cloned()
                    .or_else(|| resolved.last_key_value().map(|(ver, _)| PackageId::new(name, ver.clone())))
                else {
                    return Err(Box::new(errors::DependencyGraphError::NoMatchingVersion(
                        format!("{specifier} ({target})"),
                    )));
                };

                let resolved_ty = if (is_published_package || depth == 0) && ty == DependencyType::Peer
                {
                    DependencyType::Standard
                } else {
                    ty
                };

                if let Some(dependant_id) = dependant {
                    graph
                        .get_mut(&dependant_id)
                        .expect("dependant package not found in graph")
                        .dependencies
                        .insert(package_id.clone(), alias.clone());
                }

                let pkg_ref = &resolved[package_id.version_id()];

                if let Some(already_resolved) = graph.get_mut(&package_id) {
                    tracing::debug!("{package_id} already resolved");

                    if std::mem::discriminant(&already_resolved.pkg_ref) != std::mem::discriminant(pkg_ref) {
                        tracing::warn!(
                            "resolved package {package_id} has a different source than previously resolved one, this may cause issues",
                        );
                    }

                    if already_resolved.resolved_ty == DependencyType::Peer {
                        already_resolved.resolved_ty = resolved_ty;
                    }

                    if ty == DependencyType::Peer && depth == 0 {
                        already_resolved.is_peer = true;
                    }

                    if already_resolved.direct.is_none() && depth == 0 {
                        already_resolved.direct = Some((alias.clone(), specifier.clone(), ty));
                    }

                    return Ok(());
                }

                let node = DependencyGraphNode {
                    direct: if depth == 0 {
                        Some((alias.clone(), specifier.clone(), ty))
                    } else {
                        None
                    },
                    pkg_ref: pkg_ref.clone(),
                    dependencies: Default::default(),
                    resolved_ty,
                    is_peer: if depth == 0 {
                        false
                    } else {
                        ty == DependencyType::Peer
                    },
                };
                insert_node(
                    &mut graph,
                    &package_id,
                    node,
                    depth == 0,
                );

                tracing::debug!("resolved {package_id} from new dependency graph");

                for (dependency_alias, (dependency_spec, dependency_ty)) in
                    pkg_ref.dependencies().clone()
                {
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
                        match overridden {
                            Some(OverrideSpecifier::Specifier(spec)) => spec.clone(),
                            Some(OverrideSpecifier::Alias(alias)) => all_current_dependencies.get(alias)
                                .map(|(spec, _)| spec)
                                .ok_or_else(|| errors::DependencyGraphError::AliasNotFound(alias.clone()))?
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
                        package_id.version_id().target(),
                    ));
                }

                Ok(())
            }
                .instrument(tracing::info_span!("resolve new/changed", path = path.iter().map(Alias::as_str).collect::<Vec<_>>().join(">")))
                .await?;
		}

		for (id, node) in &mut graph {
			if node.is_peer && node.direct.is_none() {
				node.resolved_ty = DependencyType::Peer;
			}

			if node.resolved_ty == DependencyType::Peer {
				tracing::warn!("peer dependency {id} was not resolved");
			}
		}

		Ok(graph)
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
		/// An error occurred while deserializing the manifest
		#[error("failed to deserialize manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

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
