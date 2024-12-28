use crate::{
    lockfile::{DependencyGraph, DependencyGraphNode},
    manifest::DependencyType,
    names::PackageNames,
    source::{
        pesde::PesdePackageSource,
        specifiers::DependencySpecifiers,
        traits::{PackageRef, PackageSource, RefreshOptions, ResolveOptions},
        version_id::VersionId,
        PackageSources,
    },
    Project, RefreshedSources, DEFAULT_INDEX_NAME,
};
use std::collections::{btree_map::Entry, HashMap, VecDeque};
use tracing::{instrument, Instrument};

fn insert_node(
    graph: &mut DependencyGraph,
    name: &PackageNames,
    version: &VersionId,
    mut node: DependencyGraphNode,
    is_top_level: bool,
) {
    if !is_top_level && node.direct.take().is_some() {
        tracing::debug!(
            "tried to insert {name}@{version} as direct dependency from a non top-level context",
        );
    }

    match graph
        .entry(name.clone())
        .or_default()
        .entry(version.clone())
    {
        Entry::Vacant(entry) => {
            entry.insert(node);
        }
        Entry::Occupied(existing) => {
            let current_node = existing.into_mut();

            match (&current_node.direct, &node.direct) {
                (Some(_), Some(_)) => {
                    tracing::warn!("duplicate direct dependency for {name}@{version}");
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
                    let Some((old_alias, specifier, source_ty)) = &node.direct else {
                        // this is not a direct dependency, will be added if it's still being used later
                        continue;
                    };

                    if matches!(specifier, DependencySpecifiers::Workspace(_)) {
                        // workspace dependencies must always be resolved brand new
                        continue;
                    }

                    let Some(alias) = all_specifiers.remove(&(specifier.clone(), *source_ty))
                    else {
                        tracing::debug!(
                            "dependency {name}@{version} (old alias {old_alias}) from old dependency graph is no longer in the manifest",
                        );
                        continue;
                    };

                    let span = tracing::info_span!("resolve from old graph", alias);
                    let _guard = span.enter();

                    tracing::debug!("resolved {}@{} from old dependency graph", name, version);
                    insert_node(
                        &mut graph,
                        name,
                        version,
                        DependencyGraphNode {
                            direct: Some((alias.clone(), specifier.clone(), *source_ty)),
                            ..node.clone()
                        },
                        true,
                    );

                    let mut queue = node
                        .dependencies
                        .iter()
                        .map(|(name, (version, dep_alias))| {
                            (
                                name,
                                version,
                                vec![alias.to_string(), dep_alias.to_string()],
                            )
                        })
                        .collect::<VecDeque<_>>();

                    while let Some((dep_name, dep_version, path)) = queue.pop_front() {
                        let inner_span =
                            tracing::info_span!("resolve dependency", path = path.join(">"));
                        let _inner_guard = inner_span.enter();
                        if let Some(dep_node) = previous_graph
                            .get(dep_name)
                            .and_then(|v| v.get(dep_version))
                        {
                            tracing::debug!("resolved sub-dependency {dep_name}@{dep_version}");
                            insert_node(&mut graph, dep_name, dep_version, dep_node.clone(), false);

                            dep_node
                                .dependencies
                                .iter()
                                .map(|(name, (version, alias))| {
                                    (
                                        name,
                                        version,
                                        path.iter()
                                            .cloned()
                                            .chain(std::iter::once(alias.to_string()))
                                            .collect(),
                                    )
                                })
                                .for_each(|dep| queue.push_back(dep));
                        } else {
                            tracing::warn!(
                                "dependency {dep_name}@{dep_version} not found in previous graph"
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
                    spec,
                    ty,
                    None::<(PackageNames, VersionId)>,
                    vec![alias.to_string()],
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

                let resolved_ty = if (is_published_package || depth == 0) && ty == DependencyType::Peer
                {
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
                    tracing::debug!(
                        "{}@{} already resolved",
                        name,
                        target_version_id
                    );

                    if std::mem::discriminant(&already_resolved.pkg_ref)
                        != std::mem::discriminant(pkg_ref)
                    {
                        tracing::warn!(
                            "resolved package {name}@{target_version_id} has a different source than previously resolved one, this may cause issues",
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
                    &name,
                    &target_version_id,
                    node,
                    depth == 0,
                );

                tracing::debug!(
                    "resolved {}@{} from new dependency graph",
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
                        tracing::debug!(
                            "overridden specifier found for {} ({dependency_spec})",
                            path.iter()
                                .map(|s| s.as_str())
                                .chain(std::iter::once(dependency_alias.as_str()))
                                .collect::<Vec<_>>()
                                .join(">"),
                        );
                    }

                    queue.push_back((
                        overridden.cloned().unwrap_or(dependency_spec),
                        dependency_ty,
                        Some((name.clone(), target_version_id.clone())),
                        path.iter()
                            .cloned()
                            .chain(std::iter::once(dependency_alias))
                            .collect(),
                        overridden.is_some(),
                        *target_version_id.target(),
                    ));
                }

                Ok(())
            }
                .instrument(tracing::info_span!("resolve new/changed", path = path.join(">")))
                .await?;
        }

        for (name, versions) in &mut graph {
            for (version_id, node) in versions {
                if node.is_peer && node.direct.is_none() {
                    node.resolved_ty = DependencyType::Peer;
                }

                if node.resolved_ty == DependencyType::Peer {
                    tracing::warn!("peer dependency {name}@{version_id} was not resolved");
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
