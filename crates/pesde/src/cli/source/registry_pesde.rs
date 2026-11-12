#![expect(clippy::todo)]
use std::sync::Arc;

use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use pesde::Url;
use pesde::registry::PackageVersionsTree;
use pesde::registry::ScopeId;
use pesde::registry::ScopeStateResponse;
use pesde::registry::TreeEntriesEndpointResponse;
use pesde::reporters::DownloadProgressReporter;
use pesde::source::DependencySpecifier;
use pesde::source::InspectResult;
use pesde::source::PackageId;
use pesde::source::PackageSource;
use pesde::source::PesdeDependencySpecifier;
use pesde::source::ResolveResult;
use pesde::source::SourceError;
use pesde::vfs::PackageStorer;
use reqwest::header::AUTHORIZATION;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::BufReader;
use tokio_util::io::StreamReader;

use crate::cli::rootless_url;
use crate::cli::url_component_encode;

pub struct RegistryPesdePackageSource {
	pub url: Url,
	pub reqwest: reqwest::Client,
	pub authorization: Option<Box<str>>,
}

#[async_trait]
impl PackageSource for RegistryPesdePackageSource {
	async fn resolve(&self, specifier: &DependencySpecifier) -> Result<ResolveResult, SourceError> {
		let DependencySpecifier::Pesde(specifier) = specifier else {
			return Err(anyhow::format_err!(
				"invalid dependency specifier for registry pesde source: {specifier}"
			)
			.into());
		};

		let PesdeDependencySpecifier {
			name,
			req,
			registry,
		} = specifier.clone();

		let mut request = self.reqwest.get(format!(
			"{}/api/v2/scope/{}/state",
			rootless_url(&self.url),
			url_component_encode(&ScopeId::from(name.scope()).to_string()),
		));

		if let Some(authorization) = &self.authorization {
			request = request.header(AUTHORIZATION, authorization.as_ref());
		}

		let state_response: ScopeStateResponse =
			request.send().await?.error_for_status()?.json().await?;

		let packages_response = self
			.reqwest
			.get(format!(
				"{}/api/v2/tree/package_versions/{}/entries",
				rootless_url(&self.url),
				url_component_encode(&state_response.package_versions_root.0.to_string()),
			))
			.send()
			.await?
			.error_for_status()?;

		let stream = stream!({
			let stream = StreamReader::new(
				packages_response
					.bytes_stream()
					.map_err(std::io::Error::other),
			);
			let mut packages_reader = BufReader::new(stream).lines();

			while let Some(line) = packages_reader.next_line().await? {
				let entry: TreeEntriesEndpointResponse<PackageVersionsTree> =
					serde_json::from_str(&line)?;
				match entry {
					TreeEntriesEndpointResponse::Entry { key, value: _ } => {
						let version = key.version.into();
						if req.matches(&version) {
							yield Ok::<_, SourceError>((
								PackageId::Pesde(registry.clone(), name.clone(), version),
								None::<InspectResult>,
							));
						}
					}
					TreeEntriesEndpointResponse::Proof { .. } => {
						// TODO: handle proof
					}
				}
			}
		});

		Ok(ResolveResult::Found(stream.boxed()))
	}

	#[expect(clippy::todo)]
	async fn download(
		&self,
		_package_id: &PackageId,
		_reporter: Arc<dyn DownloadProgressReporter + 'static>,
		_to: &mut dyn PackageStorer,
	) -> Result<(), SourceError> {
		todo!()
	}

	async fn inspect(&self, _package_id: &PackageId) -> Result<InspectResult, SourceError> {
		todo!()
	}
}
