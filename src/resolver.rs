use crate::{
    lockfile::{insert_node, DependencyGraph, DependencyGraphNode},
    manifest::DependencyType,
    names::PackageNames,
    source::{
        pesde::PesdePackageSource,
        specifiers::DependencySpecifiers,
        traits::{PackageRef, PackageSource},
        version_id::VersionId,
        PackageSources,
    },
    Project, DEFAULT_INDEX_NAME,
};
use std::collections::{HashMap, HashSet, VecDeque};

impl Project {
    /// Create a dependency graph from the project's manifest
    pub async fn dependency_graph(
        &self,
        previous_graph: Option<&DependencyGraph>,
        refreshed_sources: &mut HashSet<PackageSources>,
    ) -> Result<DependencyGraph, Box<errors::DependencyGraphError>> {
        let manifest = self
            .deser_manifest()
            .await
            .map_err(|e| Box::new(e.into()))?;

        let mut all_specifiers = manifest
            .all_dependencies()
            .map_err(|e| Box::new(e.into()))?
            .into_iter()
            .map(|(alias, (spec, ty))| ((spec, ty), alias))
            .collect::<HashMap<_, _>>();

        let mut graph = DependencyGraph::default();

        if let Some(previous_graph) = previous_graph {
            for (name, versions) in previous_graph {
                for (version, node) in versions {
                    let Some((_, specifier)) = &node.direct else {
                        // this is not a direct dependency, will be added if it's still being used later
                        continue;
                    };

                    if matches!(specifier, DependencySpecifiers::Workspace(_)) {
                        // workspace dependencies must always be resolved brand new
                        continue;
                    }

                    let Some(alias) = all_specifiers.remove(&(specifier.clone(), node.ty)) else {
                        log::debug!(
                            "dependency {name}@{version} from old dependency graph is no longer in the manifest",
                        );
                        continue;
                    };

                    log::debug!("resolved {}@{} from old dependency graph", name, version);
                    insert_node(
                        &mut graph,
                        name.clone(),
                        version.clone(),
                        DependencyGraphNode {
                            direct: Some((alias.clone(), specifier.clone())),
                            ..node.clone()
                        },
                        true,
                    );

                    let mut queue = node
                        .dependencies
                        .iter()
                        .map(|(name, (version, _))| (name, version, 0usize))
                        .collect::<VecDeque<_>>();

                    while let Some((dep_name, dep_version, depth)) = queue.pop_front() {
                        if let Some(dep_node) = previous_graph
                            .get(dep_name)
                            .and_then(|v| v.get(dep_version))
                        {
                            log::debug!(
                                "{}resolved dependency {}@{} from {}@{}",
                                "\t".repeat(depth),
                                dep_name,
                                dep_version,
                                name,
                                version
                            );
                            insert_node(
                                &mut graph,
                                dep_name.clone(),
                                dep_version.clone(),
                                dep_node.clone(),
                                false,
                            );

                            dep_node
                                .dependencies
                                .iter()
                                .map(|(name, (version, _))| (name, version, depth + 1))
                                .for_each(|dep| queue.push_back(dep));
                        } else {
                            log::warn!(
                                "dependency {}@{} from {}@{} not found in previous graph",
                                dep_name,
                                dep_version,
                                name,
                                version
                            );
                        }
                    }
                }
            }
        }

        let mut queue = all_specifiers
            .into_iter()
            .map(|((spec, ty), alias)| {
                (
                    alias.to_string(),
                    spec,
                    ty,
                    None::<(PackageNames, VersionId)>,
                    vec![alias.to_string()],
                    false,
                    manifest.target.kind(),
                )
            })
            .collect::<VecDeque<_>>();

        while let Some((alias, specifier, ty, dependant, path, overridden, target)) =
            queue.pop_front()
        {
            let depth = path.len() - 1;

            log::debug!(
                "{}resolving {specifier} ({alias}) from {dependant:?}",
                "\t".repeat(depth)
            );
            let source = match &specifier {
                DependencySpecifiers::Pesde(specifier) => {
                    let index_url = if depth == 0 || overridden {
                        let index_name = specifier.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME);

                        manifest
                            .indices
                            .get(index_name)
                            .ok_or(errors::DependencyGraphError::IndexNotFound(
                                index_name.to_string(),
                            ))?
                            .clone()
                    } else {
                        let index_url = specifier.index.clone().unwrap();

                        index_url
                            .clone()
                            .try_into()
                            // specifiers in indices store the index url in this field
                            .unwrap()
                    };

                    PackageSources::Pesde(PesdePackageSource::new(index_url))
                }
                #[cfg(feature = "wally-compat")]
                DependencySpecifiers::Wally(specifier) => {
                    let index_url = if depth == 0 || overridden {
                        let index_name = specifier.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME);

                        manifest
                            .wally_indices
                            .get(index_name)
                            .ok_or(errors::DependencyGraphError::WallyIndexNotFound(
                                index_name.to_string(),
                            ))?
                            .clone()
                    } else {
                        let index_url = specifier.index.clone().unwrap();

                        index_url
                            .clone()
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
            };

            if refreshed_sources.insert(source.clone()) {
                source.refresh(self).await.map_err(|e| Box::new(e.into()))?;
            }

            let (name, resolved) = source
                .resolve(&specifier, self, target)
                .await
                .map_err(|e| Box::new(e.into()))?;

            let Some(target_version_id) = graph
                .get(&name)
                .and_then(|versions| {
                    versions
                        .keys()
                        // only consider versions that are compatible with the specifier
                        .filter(|ver| resolved.contains_key(ver))
                        .max()
                })
                .or_else(|| resolved.last_key_value().map(|(ver, _)| ver))
                .cloned()
            else {
                return Err(Box::new(errors::DependencyGraphError::NoMatchingVersion(
                    format!("{specifier} ({target})"),
                )));
            };

            let ty = if depth == 0 && ty == DependencyType::Peer {
                DependencyType::Standard
            } else {
                ty
            };

            if let Some((dependant_name, dependant_version_id)) = dependant {
                graph
                    .get_mut(&dependant_name)
                    .and_then(|versions| versions.get_mut(&dependant_version_id))
                    .and_then(|node| {
                        node.dependencies
                            .insert(name.clone(), (target_version_id.clone(), alias.clone()))
                    });
            }

            let pkg_ref = &resolved[&target_version_id];

            if let Some(already_resolved) = graph
                .get_mut(&name)
                .and_then(|versions| versions.get_mut(&target_version_id))
            {
                log::debug!(
                    "{}{}@{} already resolved",
                    "\t".repeat(depth),
                    name,
                    target_version_id
                );

                if std::mem::discriminant(&already_resolved.pkg_ref)
                    != std::mem::discriminant(pkg_ref)
                {
                    log::warn!(
                        "resolved package {name}@{target_version_id} has a different source than the previously resolved one, this may cause issues",
                    );
                }

                if already_resolved.ty == DependencyType::Peer && ty == DependencyType::Standard {
                    already_resolved.ty = ty;
                }

                if already_resolved.direct.is_none() && depth == 0 {
                    already_resolved.direct = Some((alias.clone(), specifier.clone()));
                }

                continue;
            }

            let node = DependencyGraphNode {
                direct: if depth == 0 {
                    Some((alias.clone(), specifier.clone()))
                } else {
                    None
                },
                pkg_ref: pkg_ref.clone(),
                dependencies: Default::default(),
                ty,
            };
            insert_node(
                &mut graph,
                name.clone(),
                target_version_id.clone(),
                node.clone(),
                depth == 0,
            );

            log::debug!(
                "{}resolved {}@{} from new dependency graph",
                "\t".repeat(depth),
                name,
                target_version_id
            );

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
                    log::debug!(
                        "{}overridden specifier found for {dependency_alias} ({dependency_spec})",
                        "\t".repeat(depth)
                    );
                }

                queue.push_back((
                    dependency_alias,
                    overridden.cloned().unwrap_or(dependency_spec),
                    dependency_ty,
                    Some((name.clone(), target_version_id.clone())),
                    path.iter()
                        .cloned()
                        .chain(std::iter::once(alias.to_string()))
                        .collect(),
                    overridden.is_some(),
                    *target_version_id.target(),
                ));
            }
        }

        for (name, versions) in &graph {
            for (version_id, node) in versions {
                if node.ty == DependencyType::Peer {
                    log::warn!("peer dependency {name}@{version_id} was not resolved");
                }
            }
        }

        Ok(graph)
    }
}

/// Errors that can occur when resolving dependencies
pub mod errors {
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
    }
}
