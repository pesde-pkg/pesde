use crate::{
    linking::generator::get_file_types,
    lockfile::{DownloadedDependencyGraphNode, DownloadedGraph},
    manifest::Manifest,
    names::PackageNames,
    scripts::{execute_script, ScriptName},
    source::{
        fs::{cas_path, store_in_cas},
        traits::PackageRef,
        version_id::VersionId,
    },
    Project, LINK_LIB_NO_FILE_FOUND, PACKAGES_CONTAINER_NAME, SCRIPTS_LINK_FOLDER,
};
use fs_err::tokio as fs;
use futures::future::try_join_all;
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::task::spawn_blocking;
use tracing::{instrument, Instrument};

/// Generates linking modules for a project
pub mod generator;

async fn create_and_canonicalize<P: AsRef<Path>>(path: P) -> std::io::Result<PathBuf> {
    let p = path.as_ref();
    fs::create_dir_all(p).await?;
    p.canonicalize()
}

async fn write_cas(destination: PathBuf, cas_dir: &Path, contents: &str) -> std::io::Result<()> {
    let hash = store_in_cas(cas_dir, contents.as_bytes(), |_| async { Ok(()) }).await?;

    match fs::remove_file(&destination).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    };

    fs::hard_link(cas_path(&hash, cas_dir), destination).await
}

impl Project {
    /// Links the dependencies of the project
    #[instrument(skip(self, graph), level = "debug")]
    pub async fn link_dependencies(
        &self,
        graph: &DownloadedGraph,
        with_types: bool,
    ) -> Result<(), errors::LinkingError> {
        let manifest = self.deser_manifest().await?;
        let manifest_target_kind = manifest.target.kind();
        let manifest = Arc::new(manifest);

        // step 1. link all non-wally packages (and their dependencies) temporarily without types
        // we do this separately to allow the required tools for the scripts to be installed
        self.link(graph, &manifest, &Arc::new(Default::default()), false)
            .await?;

        if !with_types {
            return Ok(());
        }

        // step 2. extract the types from libraries, prepare Roblox packages for syncing
        let roblox_sync_config_gen_script = manifest
            .scripts
            .get(&ScriptName::RobloxSyncConfigGenerator.to_string());

        let package_types = try_join_all(graph.iter().map(|(name, versions)| async move {
            Ok::<_, errors::LinkingError>((
                name,
                try_join_all(versions.iter().map(|(version_id, node)| async move {
                    let Some(lib_file) = node.target.lib_path() else {
                        return Ok((version_id, vec![]));
                    };

                    let container_folder = node.node.container_folder(
                        &self
                            .package_dir()
                            .join(manifest_target_kind.packages_folder(version_id.target()))
                            .join(PACKAGES_CONTAINER_NAME),
                        name,
                        version_id.version(),
                    );

                    let types = if lib_file.as_str() != LINK_LIB_NO_FILE_FOUND {
                        let lib_file = lib_file.to_path(&container_folder);

                        let contents = match fs::read_to_string(&lib_file).await {
                            Ok(contents) => contents,
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                return Err(errors::LinkingError::LibFileNotFound(
                                    lib_file.display().to_string(),
                                ));
                            }
                            Err(e) => return Err(e.into()),
                        };

                        let types = spawn_blocking(move || get_file_types(&contents))
                            .await
                            .unwrap();

                        tracing::debug!("contains {} exported types", types.len());

                        types
                    } else {
                        vec![]
                    };

                    if let Some(build_files) = Some(&node.target)
                        .filter(|_| !node.node.pkg_ref.like_wally())
                        .and_then(|t| t.build_files())
                    {
                        let Some(script_path) = roblox_sync_config_gen_script else {
                            tracing::warn!("not having a `{}` script in the manifest might cause issues with Roblox linking", ScriptName::RobloxSyncConfigGenerator);
                            return Ok((version_id, types));
                        };

                        execute_script(
                            ScriptName::RobloxSyncConfigGenerator,
                            &script_path.to_path(self.package_dir()),
                            std::iter::once(container_folder.as_os_str())
                                .chain(build_files.iter().map(OsStr::new)),
                            self,
                            false,
                        ).await
                            .map_err(|e| {
                                errors::LinkingError::GenerateRobloxSyncConfig(
                                    container_folder.display().to_string(),
                                    e,
                                )
                            })?;
                    }

                    Ok((version_id, types))
                }.instrument(tracing::info_span!("extract types", name = name.to_string(), version_id = version_id.to_string()))))
                    .await?
                    .into_iter()
                    .collect::<HashMap<_, _>>(),
            ))
        }))
            .await?
            .into_iter()
            .collect::<HashMap<_, _>>();

        // step 3. link all packages (and their dependencies), this time with types
        self.link(graph, &manifest, &Arc::new(package_types), true)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn link_files(
        &self,
        base_folder: &Path,
        container_folder: &Path,
        root_container_folder: &Path,
        relative_container_folder: &Path,
        node: &DownloadedDependencyGraphNode,
        name: &PackageNames,
        version_id: &VersionId,
        alias: &str,
        package_types: &HashMap<&PackageNames, HashMap<&VersionId, Vec<String>>>,
        manifest: &Manifest,
    ) -> Result<(), errors::LinkingError> {
        static NO_TYPES: Vec<String> = Vec::new();

        if let Some(lib_file) = node.target.lib_path() {
            let lib_module = generator::generate_lib_linking_module(
                &generator::get_lib_require_path(
                    &node.target.kind(),
                    base_folder,
                    lib_file,
                    container_folder,
                    node.node.pkg_ref.use_new_structure(),
                    root_container_folder,
                    relative_container_folder,
                    manifest,
                )?,
                package_types
                    .get(name)
                    .and_then(|v| v.get(version_id))
                    .unwrap_or(&NO_TYPES),
            );

            write_cas(
                base_folder.join(format!("{alias}.luau")),
                self.cas_dir(),
                &lib_module,
            )
            .await?;
        }

        if let Some(bin_file) = node.target.bin_path() {
            let bin_module = generator::generate_bin_linking_module(
                container_folder,
                &generator::get_bin_require_path(base_folder, bin_file, container_folder),
            );

            write_cas(
                base_folder.join(format!("{alias}.bin.luau")),
                self.cas_dir(),
                &bin_module,
            )
            .await?;
        }

        if let Some(scripts) = node.target.scripts().filter(|s| !s.is_empty()) {
            let scripts_base =
                create_and_canonicalize(self.package_dir().join(SCRIPTS_LINK_FOLDER).join(alias))
                    .await?;

            for (script_name, script_path) in scripts {
                let script_module =
                    generator::generate_script_linking_module(&generator::get_script_require_path(
                        &scripts_base,
                        script_path,
                        container_folder,
                    ));

                write_cas(
                    scripts_base.join(format!("{script_name}.luau")),
                    self.cas_dir(),
                    &script_module,
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn link(
        &self,
        graph: &DownloadedGraph,
        manifest: &Arc<Manifest>,
        package_types: &Arc<HashMap<&PackageNames, HashMap<&VersionId, Vec<String>>>>,
        is_complete: bool,
    ) -> Result<(), errors::LinkingError> {
        try_join_all(graph.iter().flat_map(|(name, versions)| {
            versions.iter().map(|(version_id, node)| {
                let name = name.clone();
                let manifest = manifest.clone();
                let package_types = package_types.clone();

                let span = tracing::info_span!(
                    "link",
                    name = name.to_string(),
                    version_id = version_id.to_string()
                );

                async move {
                    let (node_container_folder, node_packages_folder) = {
                        let base_folder = create_and_canonicalize(
                            self.package_dir()
                                .join(manifest.target.kind().packages_folder(version_id.target())),
                        )
                        .await?;
                        let packages_container_folder = base_folder.join(PACKAGES_CONTAINER_NAME);

                        let container_folder = node.node.container_folder(
                            &packages_container_folder,
                            &name,
                            version_id.version(),
                        );

                        if let Some((alias, _, _)) = &node.node.direct {
                            self.link_files(
                                &base_folder,
                                &container_folder,
                                &base_folder,
                                container_folder.strip_prefix(&base_folder).unwrap(),
                                node,
                                &name,
                                version_id,
                                alias,
                                &package_types,
                                &manifest,
                            )
                            .await?;
                        }

                        (container_folder, base_folder)
                    };

                    for (dependency_name, (dependency_version_id, dependency_alias)) in
                        &node.node.dependencies
                    {
                        let Some(dependency_node) = graph
                            .get(dependency_name)
                            .and_then(|v| v.get(dependency_version_id))
                        else {
                            if is_complete {
                                return Err(errors::LinkingError::DependencyNotFound(
                                    format!("{dependency_name}@{dependency_version_id}"),
                                    format!("{name}@{version_id}"),
                                ));
                            }

                            continue;
                        };

                        let base_folder = create_and_canonicalize(
                            self.package_dir().join(
                                version_id
                                    .target()
                                    .packages_folder(dependency_version_id.target()),
                            ),
                        )
                        .await?;
                        let packages_container_folder = base_folder.join(PACKAGES_CONTAINER_NAME);

                        let container_folder = dependency_node.node.container_folder(
                            &packages_container_folder,
                            dependency_name,
                            dependency_version_id.version(),
                        );

                        let linker_folder = create_and_canonicalize(
                            node_container_folder.join(
                                node.node
                                    .base_folder(version_id, dependency_node.target.kind()),
                            ),
                        )
                        .await?;

                        self.link_files(
                            &linker_folder,
                            &container_folder,
                            &node_packages_folder,
                            container_folder.strip_prefix(&base_folder).unwrap(),
                            dependency_node,
                            dependency_name,
                            dependency_version_id,
                            dependency_alias,
                            &package_types,
                            &manifest,
                        )
                        .await?;
                    }

                    Ok(())
                }
                .instrument(span)
            })
        }))
        .await
        .map(|_| ())
    }
}

/// Errors that can occur while linking dependencies
pub mod errors {
    use thiserror::Error;

    /// Errors that can occur while linking dependencies
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum LinkingError {
        /// An error occurred while deserializing the project manifest
        #[error("error deserializing project manifest")]
        Manifest(#[from] crate::errors::ManifestReadError),

        /// An error occurred while interacting with the filesystem
        #[error("error interacting with filesystem")]
        Io(#[from] std::io::Error),

        /// A dependency was not found
        #[error("dependency `{0}` of `{1}` not found")]
        DependencyNotFound(String, String),

        /// The library file was not found
        #[error("library file at {0} not found")]
        LibFileNotFound(String),

        /// An error occurred while generating a Roblox sync config
        #[error("error generating roblox sync config for {0}")]
        GenerateRobloxSyncConfig(String, #[source] std::io::Error),

        /// An error occurred while getting the require path for a library
        #[error("error getting require path for library")]
        GetLibRequirePath(#[from] super::generator::errors::GetLibRequirePath),
    }
}
