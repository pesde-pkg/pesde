use crate::{
	graph::{DependencyGraphNodeWithTarget, DependencyGraphWithTarget},
	linking::generator::get_file_types,
	manifest::{Alias, Manifest},
	scripts::{execute_script, ExecuteScriptHooks, ScriptName},
	source::{
		fs::{cas_path, store_in_cas},
		ids::PackageId,
		traits::PackageRef as _,
	},
	Project, LINK_LIB_NO_FILE_FOUND, PACKAGES_CONTAINER_NAME, SCRIPTS_LINK_FOLDER,
};
use fs_err::tokio as fs;
use std::{
	collections::HashMap,
	ffi::OsStr,
	path::{Path, PathBuf},
};
use tokio::task::{spawn_blocking, JoinSet};
use tracing::{instrument, Instrument as _};

/// Generates linking modules for a project
pub mod generator;
/// Incremental installs
pub mod incremental;

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
		// TODO: investigate why this happens and whether we can avoid it without ignoring all PermissionDenied errors
		#[cfg(windows)]
		Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {}
		Err(e) => return Err(e),
	}

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
		graph: &DependencyGraphWithTarget,
		with_types: bool,
	) -> Result<(), errors::LinkingError> {
		let manifest = self.deser_manifest().await?;
		let manifest_target_kind = manifest.target.kind();

		// step 1. link all non-wally packages (and their dependencies) temporarily without types
		// we do this separately to allow the required tools for the scripts to be installed
		self.link(graph, &manifest, &PackageTypes::default(), false)
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
					let Some(lib_file) = node.target.lib_path() else {
						return Ok((package_id, vec![]));
					};

					let container_folder = node.node.container_folder_from_project(
						&package_id,
						&project,
						manifest_target_kind,
					);

					let types = if lib_file.as_str() == LINK_LIB_NO_FILE_FOUND {
						vec![]
					} else {
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
					};

					if let Some(build_files) = Some(&node.target)
						.filter(|_| !node.node.pkg_ref.is_wally_package())
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
			let (package_id, types) = task.unwrap()?;
			package_types.insert(package_id, types);
		}

		// step 3. link all packages (and their dependencies), this time with types
		self.link(graph, &manifest, &package_types, true).await
	}

	async fn link(
		&self,
		graph: &DependencyGraphWithTarget,
		manifest: &Manifest,
		package_types: &PackageTypes,
		is_complete: bool,
	) -> Result<(), errors::LinkingError> {
		let package_dir_canonical = fs::canonicalize(self.package_dir()).await?;

		let mut tasks = JoinSet::<Result<_, errors::LinkingError>>::new();
		let mut link_files = |base_folder: &Path,
		                      container_folder: &Path,
		                      root_container_folder: &Path,
		                      relative_container_folder: &Path,
		                      node: &DependencyGraphNodeWithTarget,
		                      package_id: &PackageId,
		                      alias: &Alias,
		                      is_root: bool|
		 -> Result<(), errors::LinkingError> {
			static NO_TYPES: Vec<String> = Vec::new();

			if let Some(lib_file) = node.target.lib_path() {
				let destination = base_folder.join(format!("{alias}.luau"));

				let lib_module = generator::generate_lib_linking_module(
					&generator::get_lib_require_path(
						node.target.kind(),
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
				let cas_dir = self.cas_dir().to_path_buf();

				tasks.spawn(async move {
					write_cas(destination, &cas_dir, &lib_module)
						.await
						.map_err(Into::into)
				});
			}

			if let Some(bin_file) = node.target.bin_path() {
				let destination = base_folder.join(format!("{alias}.bin.luau"));

				let bin_module = generator::generate_bin_linking_module(
					container_folder,
					&generator::get_bin_require_path(base_folder, bin_file, container_folder),
				);
				let cas_dir = self.cas_dir().to_path_buf();

				tasks.spawn(async move {
					write_cas(destination, &cas_dir, &bin_module)
						.await
						.map_err(Into::into)
				});
			}

			if let Some(scripts) = node
				.target
				.scripts()
				.filter(|s| !s.is_empty() && node.node.direct.is_some() && is_root)
			{
				let scripts_base = package_dir_canonical
					.join(SCRIPTS_LINK_FOLDER)
					.join(alias.as_str());

				for (script_name, script_path) in scripts {
					let destination = scripts_base.join(format!("{script_name}.luau"));
					let script_module = generator::generate_script_linking_module(
						&generator::get_script_require_path(
							&scripts_base,
							script_path,
							container_folder,
						),
					);
					let cas_dir = self.cas_dir().to_path_buf();

					tasks.spawn(async move {
						fs::create_dir_all(destination.parent().unwrap()).await?;

						write_cas(destination, &cas_dir, &script_module)
							.await
							.map_err(Into::into)
					});
				}
			}

			Ok(())
		};

		let mut node_tasks = graph
			.iter()
			.map(|(id, node)| {
				let base_folder = self.package_dir().join(
					manifest
						.target
						.kind()
						.packages_folder(id.version_id().target()),
				);

				let id = id.clone();
				let node = node.clone();

				async move {
					Ok::<_, errors::LinkingError>((
						id,
						node,
						create_and_canonicalize(base_folder).await?,
					))
				}
			})
			.collect::<JoinSet<_>>();

		let mut dependency_tasks = JoinSet::<Result<_, errors::LinkingError>>::new();

		loop {
			tokio::select! {
				Some(res) = node_tasks.join_next() => {
					let (package_id, node, base_folder) = res.unwrap()?;
					let (node_container_folder, node_packages_folder) = {
						let packages_container_folder = base_folder.join(PACKAGES_CONTAINER_NAME);

						let container_folder =
							packages_container_folder.join(node.node.container_folder(&package_id));

						if let Some((alias, _, _)) = &node.node.direct {
							link_files(
								&base_folder,
								&container_folder,
								&base_folder,
								container_folder.strip_prefix(&base_folder).unwrap(),
								&node,
								&package_id,
								alias,
								true,
							)?;
						}

						(container_folder, base_folder)
					};

					for (dep_id, dep_alias) in &node.node.dependencies {
						let dep_id = dep_id.clone();
						let dep_alias = dep_alias.clone();
						let dep_node = graph.get(&dep_id).cloned();
						let node = node.clone();
						let package_id = package_id.clone();
						let node_container_folder = node_container_folder.clone();
						let node_packages_folder = node_packages_folder.clone();
						let package_dir = self.package_dir().to_path_buf();

						dependency_tasks.spawn(async move {
							let Some(dep_node) = dep_node else {
								return if is_complete {
									Err(errors::LinkingError::DependencyNotFound(
										dep_id.to_string(),
										package_id.to_string(),
									))
								} else {
									Ok(None)
								};
							};

							let base_folder = package_dir.join(
								package_id
									.version_id()
									.target()
									.packages_folder(dep_id.version_id().target()),
							);
							let linker_folder = node_container_folder.join(node.node.dependencies_dir(
								package_id.version_id(),
								dep_id.version_id().target(),
							));

							Ok(Some((
								dep_node.clone(),
								dep_id,
								dep_alias,
								create_and_canonicalize(base_folder).await?,
								create_and_canonicalize(linker_folder).await?,
								node_packages_folder,
							)))
						});
					}
				},
				Some(res) = dependency_tasks.join_next() => {
					let Some((
						dependency_node,
						dependency_id,
						dependency_alias,
						base_folder,
						linker_folder,
						node_packages_folder,
					)) = res.unwrap()?
					else {
						continue;
					};

					let packages_container_folder = base_folder.join(PACKAGES_CONTAINER_NAME);

					let container_folder = packages_container_folder
						.join(dependency_node.node.container_folder(&dependency_id));

					link_files(
						&linker_folder,
						&container_folder,
						&node_packages_folder,
						container_folder.strip_prefix(&base_folder).unwrap(),
						&dependency_node,
						&dependency_id,
						&dependency_alias,
						false,
					)?;
				},
				else => break,
			}
		}

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
