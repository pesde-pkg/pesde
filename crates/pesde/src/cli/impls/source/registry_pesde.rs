use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context as _;
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use itertools::Either;
use merkle_bplustree::proof::range::RangeVerifier as _;
use merkle_bplustree::proof::range::range_verifier;
use pesde::Url;
use pesde::hash::Hash;
use pesde::link::PackageStructure;
use pesde::registry::LocalNameId;
use pesde::registry::LogHeadResponse;
use pesde::registry::PackageName;
use pesde::registry::PackageVersion;
use pesde::registry::PackageVersionValue;
use pesde::registry::PackageVersionsTree;
use pesde::registry::PesdeStyleVersion;
use pesde::registry::ScopeEntryPayload;
use pesde::registry::ScopeId;
use pesde::registry::ScopeState;
use pesde::registry::TreeEntriesEndpointResponse;
use pesde::registry::TreeEntryEndpointResponse;
use pesde::registry::manifest::ResolvedDependencySpecifier;
use pesde::registry::manifest::ResolvedManifest;
use pesde::reporters::DownloadProgressReporter;
use pesde::source::DependencySpecifier;
use pesde::source::InspectResult;
use pesde::source::PackageId;
use pesde::source::PackageSource;
use pesde::source::PesdeDependencySpecifier;
use pesde::source::ResolveResult;
use pesde::source::SourceError;
use pesde::source::WallyDependencySpecifier;
use pesde::vfs::PackageStorer;
use reqwest::header::AUTHORIZATION;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::BufReader;
use tokio::sync::OnceCell;
use tokio::sync::SetOnce;
use tokio_util::io::StreamReader;
use tracing::instrument;

use crate::cli::impls::source::NetworkAccess;
use crate::cli::rootless_url;
use crate::cli::url_component_encode;

pub struct RegistryPesdePackageSource {
	url: Url,
	network: NetworkAccess,
	cache: std::sync::Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
	scope_states: HashMap<ScopeId, Arc<OnceCell<ScopeState>>>,
	package_versions:
		HashMap<PackageName, Arc<SetOnce<HashMap<PesdeStyleVersion, PackageVersionValue>>>>,
}

impl RegistryPesdePackageSource {
	pub fn new(url: Url, network: NetworkAccess) -> Self {
		Self {
			url,
			network,
			cache: Default::default(),
		}
	}

	async fn fetch_scope_state(&self, scope_id: &ScopeId) -> Result<ScopeState, SourceError> {
		let _permit = self.network.network_concurrency.acquire().await;

		let mut request = self.network.reqwest.get(format!(
			"{}/api/v2/scope/{}/log/head",
			rootless_url(&self.url),
			url_component_encode(&scope_id.to_string()),
		));

		if let Some(authorization) = &self.network.authorization {
			request = request.header(AUTHORIZATION, authorization.as_ref());
		}

		// TODO: check transparency log consistency & inclusion proofs
		let head: LogHeadResponse<ScopeEntryPayload> =
			request.send().await?.error_for_status()?.json().await?;

		Ok(head.tip.entry.payload.header().post_state.clone())
	}

	async fn get_from_package_version<T>(
		&self,
		name: &PackageName,
		version: &PesdeStyleVersion,
		get: impl FnOnce(&PackageVersionValue) -> T,
	) -> Result<T, SourceError> {
		let cached = {
			let mut cache = self.cache.lock().unwrap();
			if let Some(cached) = cache.package_versions.get(name) {
				Either::Left(cached.clone())
			} else {
				// we'll fetch one entry instead; won't be able to fill the cache

				let scope_state = cache
					.scope_states
					.entry(ScopeId::from(name.scope()))
					.or_default();

				Either::Right(scope_state.clone())
			}
		};

		match cached {
			Either::Left(cached) => {
				let state = cached
					.wait()
					.await
					.get(version)
					.context("package version not found")?;

				Ok(get(state))
			}
			Either::Right(scope_state) => {
				let scope_state = scope_state
					.get_or_try_init(async || {
						self.fetch_scope_state(&ScopeId::from(name.scope())).await
					})
					.await?;

				let _permit = self.network.network_concurrency.acquire().await;

				let package_versions_root = scope_state.package_versions_root;
				let package_version = PackageVersion {
					pkg: LocalNameId::from(name.local_name()),
					version: version.clone(),
				};

				let mut request = self.network.reqwest.get(format!(
					"{}/api/v2/tree/package_versions/{}/entry/{}",
					rootless_url(&self.url),
					url_component_encode(&package_versions_root.0.to_string()),
					url_component_encode(&package_version.to_string()),
				));

				if let Some(authorization) = &self.network.authorization {
					request = request.header(AUTHORIZATION, authorization.as_ref());
				}

				let entry_response: TreeEntryEndpointResponse<PackageVersionsTree> =
					request.send().await?.error_for_status()?.json().await?;
				match entry_response {
					TreeEntryEndpointResponse::Included { proof, value } => {
						if proof
							.verify(&package_versions_root.0, &package_version, &value)
							.is_err()
						{
							return Err(anyhow::format_err!(
								"failed to verify the package versions tree proof for package {name}@{version}"
							)
							.into());
						}

						Ok(get(&value))
					}
					TreeEntryEndpointResponse::Excluded { proof } => {
						if proof
							.verify(&package_versions_root.0, &package_version)
							.is_err()
						{
							return Err(anyhow::format_err!(
								"failed to verify the package versions tree proof for package {name}@{version}"
							)
							.into());
						}

						Err(anyhow::format_err!("package {name}@{version} not found").into())
					}
				}
			}
		}
	}

	async fn get_blob(&self, hash: &Hash) -> Result<bytes::Bytes, SourceError> {
		let _permit = self.network.network_concurrency.acquire().await;

		let mut request = self.network.reqwest.get(format!(
			"{}/api/v2/blob/by-hash/{}/{}",
			rootless_url(&self.url),
			url_component_encode(&hash.algorithm().to_string()),
			url_component_encode(&hash.encoded()),
		));

		if let Some(authorization) = &self.network.authorization {
			request = request.header(AUTHORIZATION, authorization.as_ref());
		}

		let response = request.send().await?.error_for_status()?;

		let bytes = response.bytes().await?;

		if Hash::digest(hash.algorithm(), &bytes) != *hash {
			return Err(anyhow::format_err!(
				"blob hash mismatch: expected {}, got {}",
				hash.encoded(),
				Hash::digest(hash.algorithm(), &bytes).encoded()
			)
			.into());
		}

		Ok(bytes)
	}
}

#[async_trait]
impl PackageSource for RegistryPesdePackageSource {
	#[instrument(skip(self), level = "debug")]
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

		let cached = {
			let mut cache = self.cache.lock().unwrap();
			match cache.package_versions.entry(name.clone()) {
				std::collections::hash_map::Entry::Occupied(entry) => {
					Either::Left(entry.get().clone())
				}
				std::collections::hash_map::Entry::Vacant(entry) => {
					let cached = Arc::new(SetOnce::new());
					entry.insert(cached.clone());

					let scope_state = cache
						.scope_states
						.entry(ScopeId::from(name.scope()))
						.or_default();

					Either::Right((scope_state.clone(), cached))
				}
			}
		};

		let (scope_state, cached) = match cached {
			Either::Left(cached) => {
				let stream = stream!({
					let cache = cached.wait().await;
					for version in cache.keys() {
						if req.matches(&version.clone().into()) {
							yield Ok::<_, SourceError>((
								PackageId::Pesde(registry.clone(), name.clone(), version.clone()),
								None,
							));
						}
					}
				});

				return Ok(ResolveResult::Found(stream.boxed()));
			}
			Either::Right(c) => c,
		};

		let scope_state = scope_state
			.get_or_try_init(async || self.fetch_scope_state(&ScopeId::from(name.scope())).await)
			.await?;

		let package_versions_root = scope_state.package_versions_root;

		let _permit = self.network.network_concurrency.acquire().await;

		let mut packages_request = self.network.reqwest.get(format!(
			"{}/api/v2/tree/package_versions/{}/entries",
			rootless_url(&self.url),
			url_component_encode(&package_versions_root.0.to_string()),
		));

		if let Some(authorization) = &self.network.authorization {
			packages_request = packages_request.header(AUTHORIZATION, authorization.as_ref());
		}

		let packages_response = packages_request.send().await?.error_for_status()?;

		let stream = stream!({
			let stream = StreamReader::new(
				packages_response
					.bytes_stream()
					.map_err(std::io::Error::other),
			);
			let mut packages_reader = BufReader::new(stream).lines();

			let mut cache = HashMap::new();

			let mut verifier = range_verifier(..);

			while let Some(line) = packages_reader.next_line().await? {
				let entry: TreeEntriesEndpointResponse<PackageVersionsTree> =
					serde_json::from_str(&line)?;
				match entry {
					TreeEntriesEndpointResponse::Entry { key, value } => {
						verifier.push_entry(&key, &value).context(
							"failed to push an entry to the package versions tree proof verifier",
						)?;

						if req.matches(&key.version.clone().into()) {
							yield Ok::<_, SourceError>((
								PackageId::Pesde(
									registry.clone(),
									name.clone(),
									key.version.clone(),
								),
								None::<InspectResult>,
							));
						}

						cache.insert(key.version, value);
					}
					TreeEntriesEndpointResponse::Node(node) => {
						verifier.push_node(node).context(
							"failed to push a node to the package versions tree proof verifier",
						)?;
					}
				}
			}

			verifier
				.finish(&package_versions_root.0)
				.context("failed to verify the package versions tree proof")?;

			cached.set(cache).unwrap();
		});

		Ok(ResolveResult::Found(stream.boxed()))
	}

	#[expect(clippy::todo)]
	#[instrument(skip(self, _reporter, _to), level = "debug")]
	async fn download(
		&self,
		_package_id: &PackageId,
		_reporter: Arc<dyn DownloadProgressReporter + 'static>,
		_to: &mut dyn PackageStorer,
	) -> Result<(), SourceError> {
		todo!()
	}

	#[instrument(skip(self), level = "debug")]
	async fn inspect(&self, package_id: &PackageId) -> Result<InspectResult, SourceError> {
		let PackageId::Pesde(outer_registry, name, version) = package_id else {
			return Err(anyhow::format_err!(
				"invalid package id for registry pesde source: {package_id}"
			)
			.into());
		};

		let manifest_hash = self
			.get_from_package_version(name, version, |value| value.manifest_hash)
			.await?;
		let manifest_contents = self.get_blob(&manifest_hash).await?;
		let manifest: ResolvedManifest = toml::from_slice(&manifest_contents)?;

		Ok(InspectResult {
			structure: PackageStructure::Pesde {
				exports: manifest.exports,
				config: manifest.config_luau,
			},
			dependencies: manifest
				.dependencies
				.into_inner()
				.into_iter()
				.map(|(name, (spec, ty))| {
					let spec = match spec {
						ResolvedDependencySpecifier::Pesde {
							name,
							req,
							registry,
						} => DependencySpecifier::Pesde(PesdeDependencySpecifier {
							name,
							req: req.into_inner(),
							registry: registry.map_or_else(
								|| outer_registry.clone(),
								pesde::bounded::BoundedUrl::into_inner,
							),
						}),
						ResolvedDependencySpecifier::Wally { name, req, index } => {
							DependencySpecifier::Wally(WallyDependencySpecifier {
								name,
								req: req.into_inner(),
								index: index.into_inner(),
							})
						}
					};

					(name, (spec, ty))
				})
				.collect(),
		})
	}
}
