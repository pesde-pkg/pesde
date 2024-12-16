use std::path::Path;

use relative_path::RelativePathBuf;
use serde::Deserialize;
use tempfile::TempDir;

use crate::{
    manifest::target::Target,
    scripts::{execute_script, ScriptName},
    source::wally::manifest::{Realm, WallyManifest},
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

#[instrument(skip(project, package_dir), level = "debug")]
async fn find_lib_path(
    project: &Project,
    package_dir: &Path,
) -> Result<Option<RelativePathBuf>, errors::FindLibPathError> {
    let manifest = project.deser_manifest().await?;

    let Some(script_path) = manifest
        .scripts
        .get(&ScriptName::SourcemapGenerator.to_string())
    else {
        tracing::warn!("no sourcemap generator script found in manifest");
        return Ok(None);
    };

    let result = execute_script(
        ScriptName::SourcemapGenerator,
        &script_path.to_path(&project.package_dir),
        [package_dir],
        project,
        true,
    )
    .await?;

    if let Some(result) = result.filter(|result| !result.is_empty()) {
        let node: SourcemapNode = serde_json::from_str(&result)?;
        Ok(node.file_paths.into_iter().find(|path| {
            path.extension()
                .is_some_and(|ext| ext == "lua" || ext == "luau")
        }))
    } else {
        Ok(None)
    }
}

pub(crate) const WALLY_MANIFEST_FILE_NAME: &str = "wally.toml";

#[instrument(skip(project, tempdir), level = "debug")]
pub(crate) async fn get_target(
    project: &Project,
    tempdir: &TempDir,
) -> Result<Target, errors::FindLibPathError> {
    let lib = find_lib_path(project, tempdir.path())
        .await?
        .or_else(|| Some(RelativePathBuf::from(LINK_LIB_NO_FILE_FOUND)));
    let build_files = Default::default();

    let manifest = tempdir.path().join(WALLY_MANIFEST_FILE_NAME);
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
    pub enum FindLibPathError {
        /// An error occurred deserializing the project manifest
        #[error("error deserializing manifest")]
        Manifest(#[from] crate::errors::ManifestReadError),

        /// An error occurred while executing the sourcemap generator script
        #[error("error executing sourcemap generator script")]
        Script(#[from] std::io::Error),

        /// An error occurred while deserializing the sourcemap result
        #[error("error deserializing sourcemap result")]
        Serde(#[from] serde_json::Error),

        /// An error occurred while deserializing the wally manifest
        #[error("error deserializing wally manifest")]
        WallyManifest(#[from] toml::de::Error),
    }
}
