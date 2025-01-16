use crate::{
	engine::source::{
		archive::Archive,
		traits::{DownloadOptions, EngineSource, ResolveOptions},
	},
	reporters::DownloadProgressReporter,
};
use semver::{Version, VersionReq};
use std::{collections::BTreeMap, path::PathBuf};

/// Archives
pub mod archive;
/// The GitHub engine source
pub mod github;
/// Traits for engine sources
pub mod traits;

/// Engine references
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub enum EngineRefs {
	/// A GitHub engine reference
	GitHub(github::engine_ref::Release),
}

/// Engine sources
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub enum EngineSources {
	/// A GitHub engine source
	GitHub(github::GitHubEngineSource),
}

impl EngineSource for EngineSources {
	type Ref = EngineRefs;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;

	fn directory(&self) -> PathBuf {
		match self {
			EngineSources::GitHub(source) => source.directory(),
		}
	}

	fn expected_file_name(&self) -> &str {
		match self {
			EngineSources::GitHub(source) => source.expected_file_name(),
		}
	}

	async fn resolve(
		&self,
		requirement: &VersionReq,
		options: &ResolveOptions,
	) -> Result<BTreeMap<Version, Self::Ref>, Self::ResolveError> {
		match self {
			EngineSources::GitHub(source) => source
				.resolve(requirement, options)
				.await
				.map(|map| {
					map.into_iter()
						.map(|(version, release)| (version, EngineRefs::GitHub(release)))
						.collect()
				})
				.map_err(Into::into),
		}
	}

	async fn download<R: DownloadProgressReporter + 'static>(
		&self,
		engine_ref: &Self::Ref,
		options: &DownloadOptions<R>,
	) -> Result<Archive, Self::DownloadError> {
		match (self, engine_ref) {
			(EngineSources::GitHub(source), EngineRefs::GitHub(release)) => {
				source.download(release, options).await.map_err(Into::into)
			}

			// for the future
			#[allow(unreachable_patterns)]
			_ => Err(errors::DownloadError::Mismatch),
		}
	}
}

impl EngineSources {
	/// Returns the source for the pesde engine
	pub fn pesde() -> Self {
		let mut parts = env!("CARGO_PKG_REPOSITORY").split('/').skip(3);
		let (owner, repo) = (
			parts.next().unwrap().to_string(),
			parts.next().unwrap().to_string(),
		);

		EngineSources::GitHub(github::GitHubEngineSource {
			owner,
			repo,
			asset_template: format!(
				"pesde-{{VERSION}}-{}-{}.zip",
				std::env::consts::OS,
				std::env::consts::ARCH
			),
		})
	}

	/// Returns the source for the lune engine
	pub fn lune() -> Self {
		EngineSources::GitHub(github::GitHubEngineSource {
			owner: "lune-org".into(),
			repo: "lune".into(),
			asset_template: format!(
				"lune-{{VERSION}}-{}-{}.zip",
				std::env::consts::OS,
				std::env::consts::ARCH
			),
		})
	}
}

/// Errors that can occur when working with engine sources
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when resolving an engine
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ResolveError {
		/// Failed to resolve the GitHub engine
		#[error("failed to resolve github engine")]
		GitHub(#[from] super::github::errors::ResolveError),
	}

	/// Errors that can occur when downloading an engine
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DownloadError {
		/// Failed to download the GitHub engine
		#[error("failed to download github engine")]
		GitHub(#[from] super::github::errors::DownloadError),

		/// Mismatched engine reference
		#[error("mismatched engine reference")]
		Mismatch,
	}
}
