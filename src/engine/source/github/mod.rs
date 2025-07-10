/// The GitHub engine reference
pub mod engine_ref;

use crate::{
	engine::source::{
		archive::Archive,
		github::engine_ref::Release,
		traits::{DownloadOptions, EngineSource, ResolveOptions},
	},
	reporters::{response_to_async_read, DownloadProgressReporter},
	util::no_build_metadata,
	version_matches,
};
use gix::bstr::BStr;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use semver::{Version, VersionReq};
use std::{collections::BTreeMap, path::PathBuf};

/// The GitHub engine source
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct GitHubEngineSource {
	/// The owner of the repository to download from
	pub owner: String,
	/// The repository of which to download releases from
	pub repo: String,
	/// The template for the asset name. `{VERSION}` will be replaced with the version
	pub asset_template: String,
}

impl EngineSource for GitHubEngineSource {
	type Ref = Release;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;

	fn directory(&self) -> PathBuf {
		PathBuf::from("github").join(&self.owner).join(&self.repo)
	}

	fn expected_file_name(&self) -> &str {
		&self.repo
	}

	async fn resolve(
		&self,
		requirement: &VersionReq,
		options: &ResolveOptions,
	) -> Result<BTreeMap<Version, Self::Ref>, Self::ResolveError> {
		let ResolveOptions {
			reqwest,
			auth_config,
		} = options;

		let mut request = reqwest.get(format!(
			"https://api.github.com/repos/{}/{}/releases",
			urlencoding::encode(&self.owner),
			urlencoding::encode(&self.repo),
		));

		let github_api_url = gix::Url::from_bytes(BStr::new("https://github.com")).unwrap();
		if let Some(token) = auth_config.tokens().get(&github_api_url) {
			tracing::debug!("using token for {}", github_api_url);
			request = request.header(AUTHORIZATION, token);
		}

		Ok(request
			.send()
			.await?
			.error_for_status()?
			.json::<Vec<Release>>()
			.await?
			.into_iter()
			.filter_map(
				|release| match release.tag_name.trim_start_matches('v').parse() {
					Ok(version) if version_matches(requirement, &version) => {
						Some((version, release))
					}
					_ => None,
				},
			)
			.collect())
	}

	async fn download<R: DownloadProgressReporter + 'static>(
		&self,
		engine_ref: &Self::Ref,
		options: &DownloadOptions<R>,
	) -> Result<Archive, Self::DownloadError> {
		let DownloadOptions {
			reqwest,
			reporter,
			version,
			auth_config,
		} = options;

		let desired_asset_names = [
			self.asset_template
				.replace("{VERSION}", &version.to_string()),
			self.asset_template
				.replace("{VERSION}", &no_build_metadata(version).to_string()),
		];

		let asset = engine_ref
			.assets
			.iter()
			.find(|asset| {
				desired_asset_names
					.iter()
					.any(|name| asset.name.eq_ignore_ascii_case(name))
			})
			.ok_or(errors::DownloadError::AssetNotFound)?;

		reporter.report_start();

		let mut request = reqwest
			.get(asset.url.clone())
			.header(ACCEPT, "application/octet-stream");

		let github_api_url = gix::Url::from_bytes(BStr::new("https://github.com")).unwrap();
		if let Some(token) = auth_config.tokens().get(&github_api_url) {
			tracing::debug!("using token for {}", github_api_url);
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await?.error_for_status()?;

		Ok(Archive {
			info: asset.name.parse()?,
			reader: Box::pin(response_to_async_read(response, reporter.clone())),
		})
	}
}

/// Errors that can occur when working with the GitHub engine source
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when resolving a GitHub engine
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ResolveError {
		/// Handling the request failed
		#[error("failed to handle GitHub API request")]
		Request(#[from] reqwest::Error),
	}

	/// Errors that can occur when downloading a GitHub engine
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DownloadError {
		/// An asset for the current platform could not be found
		#[error("failed to find asset for current platform")]
		AssetNotFound,

		/// Handling the request failed
		#[error("failed to handle GitHub API request")]
		Request(#[from] reqwest::Error),

		/// The asset's name could not be parsed
		#[error("failed to parse asset name")]
		ParseAssetName(#[from] crate::engine::source::archive::errors::ArchiveInfoFromStrError),
	}
}
