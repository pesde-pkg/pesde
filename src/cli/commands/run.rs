use crate::cli::{compatible_runtime, get_project_engines, style::WARN_STYLE, up_to_date_lockfile};
use anyhow::Context as _;
use clap::Args;
use fs_err::tokio as fs;
use futures::{StreamExt as _, TryStreamExt as _};
use pesde::{
	engine::runtime::Runtime,
	errors::{ManifestReadError, WorkspaceMembersError},
	linking::generator::{generate_bin_linking_module, get_bin_require_path},
	manifest::{Alias, Manifest},
	names::{PackageName, PackageNames},
	scripts::parse_script,
	source::traits::{GetTargetOptions, PackageRef as _, PackageSource as _, RefreshOptions},
	Project, MANIFEST_FILE_NAME,
};
use relative_path::{RelativePath, RelativePathBuf};
use std::{
	collections::HashSet, env::current_dir, ffi::OsString, io::Write as _, path::Path, sync::Arc,
};

#[derive(Debug, Args)]
pub struct RunCommand {
	/// The package name, script name, or path to a script to run
	#[arg(index = 1)]
	package_or_script: Option<String>,

	/// Arguments to pass to the script
	#[arg(index = 2, last = true)]
	args: Vec<OsString>,
}

impl RunCommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let manifest = project
			.deser_manifest()
			.await
			.context("failed to deserialize manifest")?;

		let engines = Arc::new(get_project_engines(&manifest, &reqwest).await?);

		let run = async |runtime: Runtime, root: &Path, file_path: &Path| -> ! {
			let dir = project.cas_dir().join(".tmp");
			fs::create_dir_all(&dir)
				.await
				.expect("failed to create temporary directory");

			let mut caller =
				tempfile::NamedTempFile::new_in(dir).expect("failed to create tempfile");

			caller
				.write_all(
					generate_bin_linking_module(
						root,
						&get_bin_require_path(
							caller.path(),
							RelativePath::from_path(
								file_path
									.file_name()
									.unwrap()
									.to_str()
									.expect("path contains invalid characters"),
							)
							.unwrap(),
							file_path.parent().unwrap(),
						),
					)
					.as_bytes(),
				)
				.expect("failed to write to tempfile");

			let status = runtime
				.prepare_command(caller.path().as_os_str(), self.args)
				.current_dir(current_dir().expect("failed to get current directory"))
				.status()
				.await
				.expect("failed to run script");

			drop(caller);

			std::process::exit(status.code().unwrap_or(1i32))
		};

		let Some(package_or_script) = self.package_or_script else {
			if let Some(script_path) = manifest.target.bin_path() {
				run(
					compatible_runtime(manifest.target.kind(), &engines)?,
					project.package_dir(),
					&script_path.to_path(project.package_dir()),
				)
				.await;
			}

			anyhow::bail!("no package or script specified, and no bin path found in manifest")
		};

		let mut package_info = None;

		if let Ok(pkg_name) = package_or_script.parse::<PackageName>() {
			let graph = if let Some(lockfile) = up_to_date_lockfile(&project).await? {
				lockfile.graph
			} else {
				anyhow::bail!("outdated lockfile, please run the install command first")
			};

			let pkg_name = PackageNames::Pesde(pkg_name);

			let mut versions = graph
				.into_iter()
				.filter(|(id, node)| *id.name() == pkg_name && node.direct.is_some())
				.collect::<Vec<_>>();

			package_info = Some(match versions.len() {
				0 => anyhow::bail!("package not found"),
				1 => versions.pop().unwrap(),
				_ => anyhow::bail!("multiple versions found. use the package's alias instead."),
			});
		} else if let Ok(alias) = package_or_script.parse::<Alias>() {
			if let Some(lockfile) = up_to_date_lockfile(&project).await? {
				package_info = lockfile
					.graph
					.into_iter()
					.find(|(_, node)| node.direct.as_ref().is_some_and(|(a, _, _)| alias == *a));
			} else {
				eprintln!(
					"{}",
					WARN_STYLE.apply_to(
						"outdated lockfile, please run the install command first to use an alias"
					)
				);
			};
		}

		if let Some((id, node)) = package_info {
			let container_folder = node.container_folder_from_project(
				&id,
				&project,
				project
					.deser_manifest()
					.await
					.context("failed to deserialize manifest")?
					.target
					.kind(),
			);

			let source = node.pkg_ref.source();
			source
				.refresh(&RefreshOptions {
					project: project.clone(),
				})
				.await
				.context("failed to refresh source")?;
			let target = source
				.get_target(
					&node.pkg_ref,
					&GetTargetOptions {
						project: project.clone(),
						path: container_folder.as_path().into(),
						id: id.into(),
						engines: engines.clone(),
					},
				)
				.await?;

			let Some(bin_path) = target.bin_path() else {
				anyhow::bail!("package has no bin path");
			};

			let path = bin_path.to_path(&container_folder);

			run(compatible_runtime(target.kind(), &engines)?, &path, &path).await;
		}

		if let Ok(mut manifest) = project.deser_manifest().await {
			if let Some(script) = manifest.scripts.remove(&package_or_script) {
				let (runtime, script_path) =
					parse_script(script, &engines).context("failed to get script info")?;

				run(
					runtime,
					project.package_dir(),
					&script_path.to_path(project.package_dir()),
				)
				.await;
			}
		}

		let relative_path = RelativePathBuf::from(package_or_script);
		let path = relative_path.to_path(project.package_dir());

		if fs::metadata(&path).await.is_err() {
			anyhow::bail!("path `{}` does not exist", path.display());
		}

		let workspace_dir = project
			.workspace_dir()
			.unwrap_or_else(|| project.package_dir());

		let members = match project.workspace_members(false).await {
			Ok(members) => members.boxed(),
			Err(WorkspaceMembersError::ManifestParse(ManifestReadError::Io(e)))
				if e.kind() == std::io::ErrorKind::NotFound =>
			{
				futures::stream::empty().boxed()
			}
			Err(e) => Err(e).context("failed to get workspace members")?,
		};

		let members = members
			.then(|res| async {
				fs::canonicalize(res.map_err(anyhow::Error::from)?.0)
					.await
					.map_err(anyhow::Error::from)
			})
			.chain(futures::stream::once(async {
				fs::canonicalize(workspace_dir).await.map_err(Into::into)
			}))
			.try_collect::<HashSet<_>>()
			.await
			.context("failed to collect workspace members")?;

		let root = 'finder: {
			let mut current_path = path.clone();
			loop {
				let canonical_path = fs::canonicalize(&current_path)
					.await
					.context("failed to canonicalize parent")?;

				if members.contains(&canonical_path)
					&& fs::metadata(canonical_path.join(MANIFEST_FILE_NAME))
						.await
						.is_ok()
				{
					break 'finder canonical_path;
				}

				if let Some(parent) = current_path.parent() {
					current_path = parent.to_path_buf();
				} else {
					break;
				}
			}

			project.package_dir().to_path_buf()
		};

		let manifest = fs::read_to_string(root.join(MANIFEST_FILE_NAME))
			.await
			.context("failed to read manifest at root")?;
		let manifest = toml::de::from_str::<Manifest>(&manifest)
			.context("failed to deserialize manifest at root")?;

		run(
			compatible_runtime(manifest.target.kind(), &engines)?,
			&root,
			&path,
		)
		.await;
	}
}
