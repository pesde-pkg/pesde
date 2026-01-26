use std::{
	io::{BufRead as _, BufReader, PipeReader, Read as _},
	path::PathBuf,
};

use relative_path::RelativePathBuf;
use serde::Deserialize;

use crate::{
	Importer, LINK_LIB_NO_FILE_FOUND, Project,
	manifest::target::Target,
	scripts::{ExecuteScriptHooks, SOURCEMAP_GENERATOR, execute_script},
	source::{
		traits::GetTargetOptions,
		wally::manifest::{Realm, WallyManifest},
	},
};
use fs_err::tokio as fs;
use tracing::instrument;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourcemapNode {
	#[serde(default)]
	file_paths: Vec<RelativePathBuf>,
}

#[derive(Debug, Default)]
pub struct SourcemapGeneratorHooks {
	stdout: String,
	stdout_reader: Option<PipeReader>,
	stderr_reader: Option<PipeReader>,
}

impl ExecuteScriptHooks for SourcemapGeneratorHooks {
	type Error = std::io::Error;

	fn stdio(
		&mut self,
	) -> (
		croshet::ShellPipeWriter,
		croshet::ShellPipeWriter,
		croshet::ShellPipeReader,
	) {
		let (stdout_reader, stdout_writer) = std::io::pipe().unwrap();
		self.stdout_reader = Some(stdout_reader);
		let (stderr_reader, stderr_writer) = std::io::pipe().unwrap();
		self.stderr_reader = Some(stderr_reader);
		let stdin_reader = std::io::pipe().unwrap().0;

		(
			croshet::ShellPipeWriter::OsPipe(stdout_writer),
			croshet::ShellPipeWriter::OsPipe(stderr_writer),
			croshet::ShellPipeReader::OsPipe(stdin_reader),
		)
	}

	async fn run(&mut self) -> Result<(), Self::Error> {
		let mut stdout_reader = self.stdout_reader.take().unwrap();
		let stderr_reader = self.stderr_reader.take().unwrap();

		let (stdout, stderr) = tokio::join!(
			tokio::task::spawn_blocking(move || {
				let mut string = String::new();
				stdout_reader.read_to_string(&mut string).map(|_| string)
			}),
			tokio::task::spawn_blocking(move || {
				let stderr_reader = BufReader::new(stderr_reader);
				for line in stderr_reader.lines() {
					match line {
						Ok(line) => {
							tracing::error!("[{SOURCEMAP_GENERATOR}]: {line}");
						}
						Err(e) => {
							tracing::error!(
								"ERROR IN READING STDERR OF {SOURCEMAP_GENERATOR}: {e}"
							);
						}
					}
				}
			})
		);
		self.stdout = stdout.unwrap()?;
		stderr.unwrap();

		Ok(())
	}
}

async fn find_lib_path(
	project: Project,
	package_dir: PathBuf,
) -> Result<Option<RelativePathBuf>, errors::GetTargetError> {
	let mut hooks = SourcemapGeneratorHooks::default();
	let ran = execute_script(
		SOURCEMAP_GENERATOR,
		&project.clone().subproject(Importer::root()),
		&mut hooks,
		vec![package_dir.into_os_string()],
	)
	.await?;
	if !ran {
		tracing::warn!(
			"no `{SOURCEMAP_GENERATOR}` found in project. wally types will not be generated"
		);
		return Ok(None);
	}

	let node: SourcemapNode = serde_json::from_str(&hooks.stdout)?;
	Ok(node.file_paths.into_iter().find(|path| {
		path.extension()
			.is_some_and(|ext| ext == "lua" || ext == "luau")
	}))
}

pub(crate) const WALLY_MANIFEST_FILE_NAME: &str = "wally.toml";

#[instrument(skip_all, level = "debug")]
pub(crate) async fn get_target(
	options: &GetTargetOptions<'_>,
) -> Result<Target, errors::GetTargetError> {
	let GetTargetOptions { project, path, .. } = options;

	let lib = find_lib_path(project.clone(), path.to_path_buf())
		.await?
		.or_else(|| Some(RelativePathBuf::from(LINK_LIB_NO_FILE_FOUND)));

	let manifest = path.join(WALLY_MANIFEST_FILE_NAME);
	let manifest = fs::read_to_string(&manifest).await?;
	let manifest: WallyManifest = toml::from_str(&manifest)?;

	Ok(if matches!(manifest.package.realm, Realm::Shared) {
		Target::Roblox { lib }
	} else {
		Target::RobloxServer { lib }
	})
}

pub mod errors {
	use thiserror::Error;

	use super::SourcemapGeneratorHooks;

	/// Errors that can occur when finding the lib path
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetTargetError))]
	#[non_exhaustive]
	pub enum GetTargetErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred while executing a script
		#[error("error executing script")]
		ExecuteScript(#[from] crate::scripts::errors::ExecuteScriptError<SourcemapGeneratorHooks>),

		/// An error occurred while deserializing the sourcemap result
		#[error("error deserializing sourcemap result")]
		Serde(#[from] serde_json::Error),

		/// An error occurred while deserializing the wally manifest
		#[error("error deserializing wally manifest")]
		WallyManifest(#[from] toml::de::Error),

		/// IO error
		#[error("io error")]
		Io(#[from] std::io::Error),
	}
}
