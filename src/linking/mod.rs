use crate::{
    graph::{DownloadedDependencyGraphNode, DownloadedGraph},
    linking::generator::get_file_types,
    manifest::Manifest,
    scripts::{execute_script, ExecuteScriptHooks, ScriptName},
    source::{
        fs::{cas_path, store_in_cas},
        ids::PackageId,
        traits::PackageRef,
    },
    Project, LINK_LIB_NO_FILE_FOUND, PACKAGES_CONTAINER_NAME, SCRIPTS_LINK_FOLDER,
};
use fs_err::tokio as fs;
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::task::{spawn_blocking, JoinSet};
use tracing::{instrument, Instrument};

/// Generates linking modules for a project
pub mod generator;

async fn create_and_canonicalize<P: AsRef<Path>>(path: P) -> std::io::Result<PathBuf> {
    let p = path.as_ref();
    fs::create_dir_all(p).await?;
    p.canonicalize()
}

async fn write_cas(destination: PathBuf, cas_dir: &Path, contents: &str) -> std::io::Result<()> {
    let hash = store_in_cas(cas_dir, contents.as_bytes()).await?;

    match fs::remove_file(&destination).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    };

    fs::hard_link(cas_path(&hash, cas_dir), destination).await
}

#[derive(Debug, Clone, Copy)]
struct LinkingExecuteScriptHooks;

impl ExecuteScriptHooks for LinkingExecuteScriptHooks {
    fn not_found(&self, script: ScriptName) {
        tracing::warn!(
            "not having a `{script}` script in the manifest might cause issues with linking"
        );
    }
}

type PackageTypes = HashMap<PackageId, Vec<String>>;

impl Project {
    /// Links the dependencies of the project
    #[instrument(skip(self, graph), level = "debug")]
    pub(crate) async fn link_dependencies(
        &self,
        graph: Arc<DownloadedGraph>,
        with_types: bool,
    ) -> Result<(), errors::LinkingError> {
        let manifest = self.deser_manifest().await?;
        let manifest_target_kind = manifest.target.kind();
        let manifest = Arc::new(manifest);

        // step 1. link all non-wally packages (and their dependencies) temporarily without types
        // we do this separately to allow the required tools for the scripts to be installed
        self.link(&graph, &manifest, &Arc::new(PackageTypes::default()), false)
            .await?;

        if !with_types {
            return Ok(());
        }

        // step 2. extract the types from libraries, prepare Roblox packages for syncing
        let mut tasks = graph
            .iter()
            .map(|(package_id, node)| {
                let span =
                    tracing::info_span!("extract types", package_id = package_id.to_string());

                let package_id = package_id.clone();
                let node = node.clone();
                let project = self.clone();

                async move {
                    let Some(lib_file) = node.target.as_ref().and_then(|t| t.lib_path()) else {
                        return Ok((package_id, vec![]));
                    };

                    let container_folder = project
                        .package_dir()
                        .join(
                            manifest_target_kind.packages_folder(package_id.version_id().target()),
                        )
                        .join(PACKAGES_CONTAINER_NAME)
                        .join(node.node.container_folder(&package_id));

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

                    if let Some(build_files) = node
                        .target
                        .as_ref()
                        .filter(|_| !node.node.pkg_ref.like_wally())
                        .and_then(|t| t.build_files())
                    {
                        execute_script(
                            ScriptName::RobloxSyncConfigGenerator,
                            &project,
                            LinkingExecuteScriptHooks,
                            std::iter::once(container_folder.as_os_str())
                                .chain(build_files.iter().map(OsStr::new)),
                            false,
                        )
                        .await
                        .map_err(errors::LinkingError::ExecuteScript)?;
                    }

                    Ok((package_id, types))
                }
                .instrument(span)
            })
            .collect::<JoinSet<_>>();

        let mut package_types = PackageTypes::new();
        while let Some(task) = tasks.join_next().await {
            let (version_id, types) = task.unwrap()?;
            package_types.insert(version_id, types);
        }

        // step 3. link all packages (and their dependencies), this time with types
        self.link(&graph, &manifest, &Arc::new(package_types), true)
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
        package_id: &PackageId,
        alias: &str,
        package_types: &PackageTypes,
        manifest: &Manifest,
    ) -> Result<(), errors::LinkingError> {
        static NO_TYPES: Vec<String> = Vec::new();

        let Some(target) = &node.target else {
            return Ok(());
        };

        if let Some(lib_file) = target.lib_path() {
            let lib_module = generator::generate_lib_linking_module(
                &generator::get_lib_require_path(
                    &target.kind(),
                    base_folder,
                    lib_file,
                    container_folder,
                    node.node.pkg_ref.use_new_structure(),
                    root_container_folder,
                    relative_container_folder,
                    manifest,
                )?,
                package_types.get(package_id).unwrap_or(&NO_TYPES),
            );

            write_cas(
                base_folder.join(format!("{alias}.luau")),
                self.cas_dir(),
                &lib_module,
            )
            .await?;
        }

        if let Some(bin_file) = target.bin_path() {
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

        if let Some(scripts) = target.scripts().filter(|s| !s.is_empty()) {
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
        graph: &Arc<DownloadedGraph>,
        manifest: &Arc<Manifest>,
        package_types: &Arc<PackageTypes>,
        is_complete: bool,
    ) -> Result<(), errors::LinkingError> {
        let mut tasks = graph
            .iter()
            .map(|(package_id, node)| {
                let graph = graph.clone();
                let manifest = manifest.clone();
                let package_types = package_types.clone();

                let span = tracing::info_span!("link", package_id = package_id.to_string());
                let package_id = package_id.clone();
                let node = node.clone();
                let project = self.clone();

                async move {
                    let (node_container_folder, node_packages_folder) = {
                        let base_folder = create_and_canonicalize(
                            project.package_dir().join(
                                manifest
                                    .target
                                    .kind()
                                    .packages_folder(package_id.version_id().target()),
                            ),
                        )
                        .await?;
                        let packages_container_folder = base_folder.join(PACKAGES_CONTAINER_NAME);

                        let container_folder =
                            packages_container_folder.join(node.node.container_folder(&package_id));

                        if let Some((alias, _, _)) = &node.node.direct {
                            project
                                .link_files(
                                    &base_folder,
                                    &container_folder,
                                    &base_folder,
                                    container_folder.strip_prefix(&base_folder).unwrap(),
                                    &node,
                                    &package_id,
                                    alias,
                                    &package_types,
                                    &manifest,
                                )
                                .await?;
                        }

                        (container_folder, base_folder)
                    };

                    for (dependency_id, dependency_alias) in &node.node.dependencies {
                        let Some(dependency_node) = graph.get(dependency_id) else {
                            if is_complete {
                                return Err(errors::LinkingError::DependencyNotFound(
                                    dependency_id.to_string(),
                                    package_id.to_string(),
                                ));
                            }

                            continue;
                        };

                        let base_folder = create_and_canonicalize(
                            project.package_dir().join(
                                package_id
                                    .version_id()
                                    .target()
                                    .packages_folder(dependency_id.version_id().target()),
                            ),
                        )
                        .await?;
                        let packages_container_folder = base_folder.join(PACKAGES_CONTAINER_NAME);

                        let container_folder = packages_container_folder
                            .join(dependency_node.node.container_folder(dependency_id));

                        let linker_folder = create_and_canonicalize(node_container_folder.join(
                            node.node.base_folder(
                                package_id.version_id(),
                                match &dependency_node.target {
                                    Some(t) => t.kind(),
                                    None => continue,
                                },
                            ),
                        ))
                        .await?;

                        project
                            .link_files(
                                &linker_folder,
                                &container_folder,
                                &node_packages_folder,
                                container_folder.strip_prefix(&base_folder).unwrap(),
                                dependency_node,
                                dependency_id,
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
            .collect::<JoinSet<_>>();

        while let Some(task) = tasks.join_next().await {
            task.unwrap()?;
        }

        Ok(())
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

        /// Executing a script failed
        #[error("error executing script")]
        ExecuteScript(#[from] crate::scripts::errors::ExecuteScriptError),

        /// An error occurred while getting the require path for a library
        #[error("error getting require path for library")]
        GetLibRequirePath(#[from] super::generator::errors::GetLibRequirePath),
    }
}
