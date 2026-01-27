use crate::cli::ExecReplace as _;
use crate::cli::compatible_runtime;
use crate::cli::get_project_engines;
use crate::cli::install::get_graph_strict;
use anyhow::Context as _;
use clap::Args;
use fs_err::tokio as fs;
use pesde::RefreshedSources;
use pesde::Subproject;
use pesde::engine::runtime::Runtime;
use pesde::graph::DependencyGraphNode;
use pesde::linking::generator::generate_bin_linking_module;
use pesde::linking::generator::get_bin_require_path;
use pesde::manifest::Alias;
use pesde::source::traits::GetTargetOptions;
use pesde::source::traits::PackageSource as _;
use pesde::source::traits::RefreshOptions;
use relative_path::RelativePath;
use relative_path::RelativePathBuf;
use std::env::current_dir;
use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Args)]
pub struct RunCommand {
	/// The package name, script name, or path to a script to run
	#[arg(index = 1)]
	package_or_script: String,

	/// Arguments to pass to the script
	#[arg(index = 2)]
	args: Vec<OsString>,
}

impl RunCommand {
	pub async fn run(self, subproject: Subproject, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let manifest = subproject
			.deser_manifest()
			.await
			.context("failed to deserialize manifest")?;

		let engines = Arc::new(
			get_project_engines(&manifest, &reqwest, subproject.project().auth_config()).await?,
		);

		let run = async |runtime: Runtime, root: &Path, file_path: &Path| -> ! {
			let tempdir = subproject.project().cas_dir().join(".tmp");
			fs::create_dir_all(&tempdir)
				.await
				.expect("failed to create temporary directory");

			let mut caller = tempfile::Builder::new()
				.suffix(".luau")
				.tempfile_in(&tempdir)
				.expect("failed to create tempfile");

			caller
				.write_all(
					generate_bin_linking_module(
						root,
						&get_bin_require_path(
							&tempdir,
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

			let mut command = runtime.prepare_command(caller.path().as_os_str(), self.args);
			command.current_dir(current_dir().expect("failed to get current directory"));
			command.exec_replace()
		};

		#[allow(clippy::useless_let_if_seq)]
		let mut package_info = None;

		if let Ok(alias) = self.package_or_script.parse::<Alias>() {
			let refreshed_sources = RefreshedSources::new();
			let mut graph = get_graph_strict(subproject.project(), &refreshed_sources).await?;
			package_info = graph
				.importers
				.remove(subproject.importer())
				.context("failed to get importer from lockfile")?
				.dependencies
				.remove(&alias)
				.map(|(id, _, _)| id);
		}

		if let Some(id) = package_info {
			let dir = subproject.private_dir();
			let container_dir = dir
				.join("dependencies")
				.join(DependencyGraphNode::container_dir_top_level(&id));

			let source = id.source();
			source
				.refresh(&RefreshOptions {
					project: subproject.project().clone(),
				})
				.await
				.context("failed to refresh source")?;
			let target = source
				.get_target(
					id.pkg_ref(),
					&GetTargetOptions {
						project: subproject.project().clone(),
						path: container_dir.as_path().into(),
						version_id: id.v_id(),
						engines: engines.clone(),
					},
				)
				.await?;

			let Some(bin_path) = target.bin_path() else {
				anyhow::bail!("package has no bin path");
			};

			let path = bin_path.to_path(&container_dir);

			run(compatible_runtime(target.kind(), &engines)?, &path, &path).await;
		}

		if let Ok(manifest) = subproject.deser_manifest().await
			&& let Some(_script) = manifest.scripts.get(&self.package_or_script)
		{
			// let (runtime, script_path) =
			// 	parse_script(script, &engines).context("failed to get script info")?;

			// run(
			// 	runtime,
			// 	project.package_dir(),
			// 	&script_path.to_path(project.package_dir()),
			// )
			// .await;
		}

		let relative_path = RelativePathBuf::from(self.package_or_script);
		let path = relative_path.to_path(subproject.dir());

		if fs::metadata(&path).await.is_err() {
			anyhow::bail!("path `{}` does not exist", path.display());
		}

		unimplemented!();

		// let workspace_dir = project
		// 	.workspace_dir()
		// 	.unwrap_or_else(|| project.package_dir());

		// let members = match project.workspace_members().await {
		// 	Ok(members) => members.boxed(),
		// 	Err(WorkspaceMembersError::ManifestParse(ManifestReadError::Io(e)))
		// 		if e.kind() == std::io::ErrorKind::NotFound =>
		// 	{
		// 		futures::stream::empty().boxed()
		// 	}
		// 	Err(e) => Err(e).context("failed to get workspace members")?,
		// };
		//
		// let members = members
		// 	.then(|res| async {
		// 		fs::canonicalize(res.map_err(anyhow::Error::from)?.0)
		// 			.await
		// 			.map_err(anyhow::Error::from)
		// 	})
		// 	.chain(futures::stream::once(async {
		// 		fs::canonicalize(workspace_dir).await.map_err(Into::into)
		// 	}))
		// 	.try_collect::<HashSet<_>>()
		// 	.await
		// 	.context("failed to collect workspace members")?;

		// let root = 'finder: {
		// 	let mut current_path = path.clone();
		// 	loop {
		// 		let canonical_path = fs::canonicalize(&current_path)
		// 			.await
		// 			.context("failed to canonicalize parent")?;

		// 		if members.contains(&canonical_path)
		// 			&& fs::metadata(canonical_path.join(MANIFEST_FILE_NAME))
		// 				.await
		// 				.is_ok()
		// 		{
		// 			break 'finder canonical_path;
		// 		}

		// 		if let Some(parent) = current_path.parent() {
		// 			current_path = parent.to_path_buf();
		// 		} else {
		// 			break;
		// 		}
		// 	}

		// 	project.package_dir().to_path_buf()
		// };

		// let manifest = fs::read_to_string(root.join(MANIFEST_FILE_NAME))
		// 	.await
		// 	.context("failed to read manifest at root")?;
		// let manifest = toml::de::from_str::<Manifest>(&manifest)
		// 	.context("failed to deserialize manifest at root")?;

		// run(
		// 	compatible_runtime(manifest.target.kind(), &engines)?,
		// 	&root,
		// 	&path,
		// )
		// .await;
	}
}
