use std::collections::HashMap;
use std::str::FromStr as _;
use std::sync::Arc;

use anyhow::Context as _;
use async_stream::stream;
use async_trait::async_trait;
use futures::AsyncReadExt as _;
use futures::AsyncWriteExt as _;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use itertools::Either;
use pesde::link::PackageStructure;
use pesde::registry::LocalNameId;
use pesde::registry::PackageName;
use pesde::registry::PackageVersion;
use pesde::registry::PackageVersionValue;
use pesde::registry::PesdeStyleVersion;
use pesde::registry::ScopeId;
use pesde::registry::ScopeState;
use pesde::registry::manifest::ExportPath;
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
use tokio::sync::OnceCell;
use tokio::sync::SetOnce;
use tracing::instrument;

use crate::cli::registry::RegistryClient;

pub struct RegistryPesdePackageSource {
	client: Arc<RegistryClient>,
	cache: std::sync::Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
	scope_states: HashMap<ScopeId, Arc<OnceCell<ScopeState>>>,
	package_versions:
		HashMap<PackageName, Arc<SetOnce<HashMap<PesdeStyleVersion, PackageVersionValue>>>>,
}

impl RegistryPesdePackageSource {
	pub fn new(client: Arc<RegistryClient>) -> Self {
		Self {
			client,
			cache: Default::default(),
		}
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
						anyhow::Ok(
							self.client
								.scope_head(&ScopeId::from(name.scope()))
								.await?
								.with_context(|| format!("scope {} does not exist", name.scope()))?
								.tip
								.entry
								.payload
								.header()
								.post_state
								.clone(),
						)
					})
					.await?;

				let package_version = PackageVersion {
					pkg: LocalNameId::from(name.local_name()),
					version: version.clone(),
				};

				let value = self
					.client
					.tree_entry(&scope_state.package_versions_root, &package_version)
					.await?
					.with_context(|| format!("package {name}@{version} not found"))?;

				Ok(get(&value))
			}
		}
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
			.get_or_try_init(async || {
				anyhow::Ok(
					self.client
						.scope_head(&ScopeId::from(name.scope()))
						.await?
						.with_context(|| format!("scope {} does not exist", name.scope()))?
						.tip
						.entry
						.payload
						.header()
						.post_state
						.clone(),
				)
			})
			.await?;

		let mut entries = self
			.client
			.tree_entries(&scope_state.package_versions_root)
			.await?;

		let stream = stream!({
			let mut cache = HashMap::new();

			let pkg = LocalNameId::from(name.local_name());

			while let Some((key, value)) = entries.try_next().await? {
				if key.pkg != pkg {
					continue;
				}

				if req.matches(&key.version.clone().into()) {
					yield Ok::<_, SourceError>((
						PackageId::Pesde(registry.clone(), name.clone(), key.version.clone()),
						None::<InspectResult>,
					));
				}

				cache.insert(key.version, value);
			}

			cached.set(cache).unwrap();
		});

		Ok(ResolveResult::Found(stream.boxed()))
	}

	#[instrument(skip(self, reporter, to), level = "debug")]
	async fn download(
		&self,
		package_id: &PackageId,
		reporter: Arc<dyn DownloadProgressReporter + 'static>,
		to: &mut dyn PackageStorer,
	) -> Result<(), SourceError> {
		let PackageId::Pesde(_, name, version) = package_id else {
			return Err(anyhow::format_err!(
				"invalid package id for registry pesde source: {package_id}"
			)
			.into());
		};

		let archive_hash = self
			.get_from_package_version(name, version, |value| value.archive_hash)
			.await?;
		let archive_bytes = self
			.client
			.blob_with_reporter(&archive_hash, reporter)
			.await?;

		let mut archive = async_zip::tokio::read::seek::ZipFileReader::with_tokio(
			std::io::Cursor::new(archive_bytes),
		)
		.await?;

		if archive.file().zip64() {
			return Err(anyhow::format_err!(
				"zip archive for package `{package_id}` is zip64, which is not supported"
			)
			.into());
		}

		let mut buf = vec![0; 8 * 1024];

		for index in 0..archive.file().entries().len() {
			let entry = archive.file().entries().get(index).unwrap();
			let entry_name = entry.filename().as_str()?;

			tracing::trace!("{index}-th entry in package `{package_id}`: {entry_name}");

			let path = ExportPath::from_str(entry_name.strip_suffix('/').unwrap_or(entry_name))
				.with_context(|| format!("zip archive entry `{entry_name}`"))?;

			if entry.dir()? {
				to.directory_at(path.into()).await?;
				continue;
			}

			let size_hint = entry.uncompressed_size();
			let file = to.file_at(path.clone().into(), size_hint).await?;

			let mut entry_reader = archive.reader_with_entry(index).await?;
			while let n = entry_reader.read(&mut buf).await?
				&& n > 0
			{
				file.write_all(&buf[..n]).await?;
			}

			if entry_reader.compute_hash() != entry_reader.entry().crc32() {
				return Err(anyhow::format_err!(
					"hash mismatch for file `{path}` in package `{package_id}`"
				)
				.into());
			}
		}

		Ok(())
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
		let manifest_contents = self.client.blob(&manifest_hash).await?;
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
