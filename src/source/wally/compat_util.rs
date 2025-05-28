use std::path::Path;

use relative_path::RelativePathBuf;
use serde::Deserialize;

use crate::{
	engine::runtime::Engines,
	manifest::target::Target,
	scripts::{execute_script, ExecuteScriptHooks, ScriptName},
	source::{
		traits::GetTargetOptions,
		wally::manifest::{Realm, WallyManifest},
	},
	Project, LINK_LIB_NO_FILE_FOUND,
};
use fs_err::tokio as fs;
use tracing::instrument;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourcemapNode {
	#[serde(default)]
	file_paths: Vec<RelativePathBuf>,
}

#[derive(Debug, Clone, Copy)]
struct CompatExecuteScriptHooks;

impl ExecuteScriptHooks for CompatExecuteScriptHooks {
	fn not_found(&self, script: ScriptName) {
		tracing::warn!("no {script} found in project. wally types will not be generated");
	}
}

async fn find_lib_path(
	project: &Project,
	engines: &Engines,
	package_dir: &Path,
) -> Result<Option<RelativePathBuf>, errors::GetTargetError> {
	let Some(result) = execute_script(
		ScriptName::SourcemapGenerator,
		project,
		engines,
		CompatExecuteScriptHooks,
		[package_dir],
		true,
	)
	.await?
	.into_output()
	.filter(|result| !result.is_empty()) else {
		return Ok(None);
	};

	let node: SourcemapNode = serde_json::from_str(&result)?;
	Ok(node.file_paths.into_iter().find(|path| {
		path.extension()
			.is_some_and(|ext| ext == "lua" || ext == "luau")
	}))
}

pub(crate) const WALLY_MANIFEST_FILE_NAME: &str = "wally.toml";

#[instrument(skip_all, level = "debug")]
pub(crate) async fn get_target(
	options: &GetTargetOptions,
) -> Result<Target, errors::GetTargetError> {
	let GetTargetOptions {
		project,
		path,
		engines,
		..
	} = options;

	let lib = find_lib_path(project, engines, path)
		.await?
		.or_else(|| Some(RelativePathBuf::from(LINK_LIB_NO_FILE_FOUND)));
	let build_files = Default::default();

	let manifest = path.join(WALLY_MANIFEST_FILE_NAME);
	let manifest = fs::read_to_string(&manifest).await?;
	let manifest: WallyManifest = toml::from_str(&manifest)?;

	Ok(if matches!(manifest.package.realm, Realm::Shared) {
		Target::Roblox { lib, build_files }
	} else {
		Target::RobloxServer { lib, build_files }
	})
}

pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when finding the lib path
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum GetTargetError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred while executing a script
		#[error("error executing script")]
		ExecuteScript(#[from] crate::scripts::errors::ExecuteScriptError),

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
