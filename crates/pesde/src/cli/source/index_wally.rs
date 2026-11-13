use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::BufRead as _;
use std::path::Path;
use std::sync::Arc;
use std::sync::LazyLock;

use anyhow::Context as _;
use async_trait::async_trait;
use futures::AsyncReadExt as _;
use futures::AsyncWriteExt as _;
use futures::StreamExt as _;
use futures::stream;
use pesde::__private::str_as_fs_safe;
use pesde::Url;
use pesde::link::PackageStructure;
use pesde::registry::manifest::Alias;
use pesde::registry::manifest::DependencyType;
use pesde::registry::manifest::wally_compat::WallyPackageName;
use pesde::reporters::DownloadProgressReporter;
use pesde::semver::Version;
use pesde::semver::VersionReq;
use pesde::source::DependencySpecifier;
use pesde::source::InspectResult;
use pesde::source::PackageId;
use pesde::source::PackageSource;
use pesde::source::ResolveResult;
use pesde::source::SourceError;
use pesde::source::SourceId;
use pesde::source::WallyDependencySpecifier;
use pesde::vfs::PackageStorer;
use pesde::vfs::VfsPath;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use serde::Deserializer;
use tokio::io::AsyncReadExt as _;
use tokio::task::spawn_blocking;

use crate::cli::dirs::cache_dir;
use crate::cli::reporters::response_to_async_buf_read;
use crate::cli::rootless_url;
use crate::cli::source::NetworkAccess;
use crate::cli::source::git_index::read_file;
use crate::cli::source::git_index::refresh_git_repo;
use crate::cli::source::git_index::root_tree;
use crate::cli::url_component_encode;

pub struct IndexWallyPackageSource {
	path: Box<Path>,
	config: WallyIndexConfig,
	network: NetworkAccess,
}

static WALLY_VERSION_HEADER: LazyLock<Box<str>> = LazyLock::new(|| {
	std::env::var("PESDE_WALLY_VERSION")
		.as_deref()
		.unwrap_or("0.3.2")
		.into()
});

impl IndexWallyPackageSource {
	pub async fn new(url: Url, network: NetworkAccess) -> anyhow::Result<Self> {
		let path = cache_dir()?
			.join("git_repos")
			.join("wally")
			.join(str_as_fs_safe(url.as_str()))
			.into_boxed_path();

		refresh_git_repo(path.clone().into_path_buf(), url.clone()).await?;

		spawn_blocking(move || {
			let repo = gix::open(path.clone())?;
			let tree = root_tree(&repo)?;
			let file = read_file(&tree, ["config.json"])?;

			match file {
				Some(s) => Ok(Self {
					path,
					config: serde_json::from_slice(&s)?,
					network,
				}),
				None => {
					anyhow::bail!("config.json not found in wally index git repository `{url}`")
				}
			}
		})
		.await
		.unwrap()
	}

	async fn read_file(
		&self,
		name: WallyPackageName,
	) -> anyhow::Result<
		Option<impl Iterator<Item = Result<(Version, InspectResult), SourceError>> + use<>>,
	> {
		let file = spawn_blocking({
			let path = self.path.clone();
			let name = name.clone();

			move || {
				let repo = gix::open(path)?;
				let tree = root_tree(&repo)?;
				read_file(&tree, [name.scope().as_str(), name.local_name().as_str()])
			}
		})
		.await??;

		let Some(file) = file else {
			return Ok(None);
		};

		let file = std::io::Cursor::new(file)
			.lines()
			.zip(std::iter::repeat(name))
			.map(move |(l, name)| match l {
				Ok(l) => serde_json::from_str::<WallyManifest>(&l).map_err(|e| {
					anyhow::format_err!(
						"failed to parse wally index file for package `{name}`: {e}"
					)
				}),
				Err(e) => Err(anyhow::format_err!(
					"failed to read line from wally index file for package `{name}`: {e}"
				)),
			})
			.map(move |m| match m {
				Ok(manifest) => Ok(manifest.into_inspect_parts()?),
				Err(e) => Err(e.into()),
			});

		Ok(Some(file))
	}
}

#[async_trait]
impl PackageSource for IndexWallyPackageSource {
	async fn resolve(&self, specifier: &DependencySpecifier) -> Result<ResolveResult, SourceError> {
		let DependencySpecifier::Wally(specifier) = specifier else {
			return Err(anyhow::format_err!(
				"invalid dependency specifier for index wally source: {specifier}"
			)
			.into());
		};

		let WallyDependencySpecifier { name, req, index } = specifier.clone();

		let Some(iter) = self.read_file(name.clone()).await? else {
			if self.config.fallback_registries.is_empty() {
				return Err(anyhow::format_err!(
					"package `{name}` not found in wally index `{index}`"
				)
				.into());
			}

			return Ok(ResolveResult::Elsewhere(
				self.config.fallback_registries.clone(),
			));
		};

		let iter = iter.filter_map(move |m| match m {
			Ok((version, inspect)) => req.matches(&version).then(|| {
				Ok((
					PackageId::Wally(index.clone(), name.clone(), version),
					Some(inspect),
				))
			}),
			Err(e) => Some(Err(e)),
		});

		Ok(ResolveResult::Found(stream::iter(iter).boxed()))
	}

	async fn download(
		&self,
		package_id: &PackageId,
		reporter: Arc<dyn DownloadProgressReporter + 'static>,
		to: &mut dyn PackageStorer,
	) -> Result<(), SourceError> {
		let PackageId::Wally(_, name, version) = package_id else {
			return Err(anyhow::format_err!(
				"invalid package id for index wally source: {package_id}"
			)
			.into());
		};

		let _permit = self.network.network_concurrency.acquire().await;
		let mut request = self
			.network
			.reqwest
			.get(format!(
				"{}/v1/package-contents/{}/{}/{}",
				rootless_url(&self.config.api),
				url_component_encode(name.scope()),
				url_component_encode(name.local_name()),
				url_component_encode(&version.to_string())
			))
			.header("Wally-Version", &**WALLY_VERSION_HEADER);

		if let Some(authorization) = &self.network.authorization {
			request = request.header(AUTHORIZATION, authorization.as_ref());
		}

		let response = request.send().await?.error_for_status()?;

		let total_len = response.content_length().unwrap_or(0);
		let bytes = response_to_async_buf_read(response, reporter.clone());
		tokio::pin!(bytes);

		let mut archive_bytes = Vec::with_capacity(total_len as usize);
		bytes
			.read_to_end(&mut archive_bytes)
			.await
			.context("failed to read archive bytes")?;

		let mut archive = async_zip::tokio::read::seek::ZipFileReader::with_tokio(
			std::io::Cursor::new(archive_bytes),
		)
		.await?;

		let mut buf = vec![0; 8 * 1024];

		for index in 0..archive.file().entries().len() {
			let entry = archive.file().entries().get(index).unwrap();
			let entry_name = entry.filename().as_str()?;

			tracing::trace!("{index}-th entry in wally package `{package_id}`: {entry_name}");

			let path = VfsPath::from_path(entry_name)
				.with_context(|| format!("wally zip archvie `{entry_name}`"))?;

			if entry.dir()? {
				to.directory_at(path).await?;
				continue;
			}

			let size_hint = entry.uncompressed_size();
			let file = to.file_at(path.clone(), size_hint).await?;

			let mut entry_reader = archive.reader_with_entry(index).await?;
			while let n = entry_reader.read(&mut buf).await?
				&& n > 0
			{
				file.write_all(&buf[..n]).await?;
			}

			if entry_reader.compute_hash() != entry_reader.entry().crc32() {
				return Err(anyhow::format_err!(
					"hash mismatch for file `{path}` in wally package `{package_id}`"
				)
				.into());
			}
		}

		Ok(())
	}

	async fn inspect(&self, package_id: &PackageId) -> Result<InspectResult, SourceError> {
		let PackageId::Wally(_, name, version) = package_id else {
			return Err(anyhow::format_err!(
				"invalid package id for index wally source: {package_id}"
			)
			.into());
		};

		let inspect = self
			.read_file(name.clone())
			.await?
			.context("failed to read wally index file")?
			.find_map(|m| match m {
				Ok((v, inspect)) => (v == *version).then(|| Ok(inspect)),
				Err(e) => Some(Err(e)),
			})
			.context("package version not found in wally index file")??;

		Ok(inspect)
	}
}

#[derive(Debug, Clone, Deserialize)]
struct WallyIndexConfig {
	api: Url,
	#[serde(default, deserialize_with = "deserialize_fallback_registries")]
	fallback_registries: Box<[SourceId]>,
}
fn deserialize_fallback_registries<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<Box<[SourceId]>, D::Error> {
	struct Visitor;
	impl<'de> serde::de::Visitor<'de> for Visitor {
		type Value = Box<[SourceId]>;

		fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
			write!(formatter, "a list of fallback registries")
		}

		fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
		where
			A: serde::de::SeqAccess<'de>,
		{
			let mut registries = Vec::with_capacity(seq.size_hint().unwrap_or(0));
			while let Some(registry) = seq.next_element()? {
				registries.push(SourceId::Wally(registry));
			}
			Ok(registries.into())
		}
	}

	deserializer.deserialize_seq(Visitor)
}

#[derive(Deserialize, Copy, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub(crate) enum WallyRealm {
	#[serde(alias = "dev")]
	Shared,
	Server,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct WallyPackage {
	name: WallyPackageName,
	version: Version,
	#[serde(rename = "registry")]
	index: Url,
	#[expect(unused)]
	realm: WallyRealm,
}

fn deserialize_specifiers<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<HashMap<Alias, (WallyPackageName, VersionReq)>, D::Error> {
	struct Specifier((WallyPackageName, VersionReq));
	impl<'de> Deserialize<'de> for Specifier {
		fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
		where
			D: Deserializer<'de>,
		{
			deserializer.deserialize_str(SpecifierVisitor)
		}
	}

	struct SpecifierVisitor;
	impl serde::de::Visitor<'_> for SpecifierVisitor {
		type Value = Specifier;

		fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
			write!(formatter, "a wally package specifier")
		}

		fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
		where
			E: serde::de::Error,
		{
			let Some((name, version)) = v.split_once('@') else {
				return Err(serde::de::Error::custom(format!("invalid specifier `{v}`")));
			};

			Ok(Specifier((
				name.parse().map_err(serde::de::Error::custom)?,
				version.parse().map_err(serde::de::Error::custom)?,
			)))
		}
	}

	struct MapVisitor;
	impl<'de> serde::de::Visitor<'de> for MapVisitor {
		type Value = HashMap<Alias, (WallyPackageName, VersionReq)>;

		fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
			write!(formatter, "a map of alias to specifier")
		}

		fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
		where
			A: serde::de::MapAccess<'de>,
		{
			let mut specifiers = HashMap::new();
			while let Some((alias, specifier)) = map.next_entry::<Alias, Specifier>()? {
				match specifiers.entry(alias) {
					std::collections::hash_map::Entry::Vacant(entry) => {
						entry.insert(specifier.0);
					}
					std::collections::hash_map::Entry::Occupied(entry) => {
						return Err(serde::de::Error::custom(format!(
							"duplicate alias `{}` in specifiers",
							entry.key()
						)));
					}
				}
			}
			Ok(specifiers)
		}
	}

	deserializer.deserialize_map(MapVisitor)
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct WallyManifest {
	package: WallyPackage,
	#[serde(default, deserialize_with = "deserialize_specifiers")]
	dependencies: HashMap<Alias, (WallyPackageName, VersionReq)>,
	#[serde(default, deserialize_with = "deserialize_specifiers")]
	server_dependencies: HashMap<Alias, (WallyPackageName, VersionReq)>,
	#[serde(default, deserialize_with = "deserialize_specifiers")]
	dev_dependencies: HashMap<Alias, (WallyPackageName, VersionReq)>,
}

impl WallyManifest {
	pub(crate) fn into_inspect_parts(self) -> anyhow::Result<(Version, InspectResult)> {
		let iter = self
			.dependencies
			.into_iter()
			.chain(self.server_dependencies)
			.zip(std::iter::repeat(DependencyType::Standard))
			.chain(
				self.dev_dependencies
					.into_iter()
					.zip(std::iter::repeat(DependencyType::Dev)),
			);

		let mut dependencies = BTreeMap::new();
		for ((alias, (name, req)), dep_ty) in iter {
			let dep = DependencySpecifier::Wally(WallyDependencySpecifier {
				name: name.clone(),
				req,
				index: self.package.index.clone(),
			});

			match dependencies.entry(alias.clone()) {
				std::collections::btree_map::Entry::Vacant(entry) => {
					entry.insert((dep, dep_ty));
				}
				std::collections::btree_map::Entry::Occupied(_) => {
					anyhow::bail!(
						"duplicate dependency `{alias}` in wally index file for package `{name}`"
					);
				}
			}
		}

		Ok((
			self.package.version,
			InspectResult {
				structure: PackageStructure::Wally(self.package.name.local_name().clone()),
				dependencies,
			},
		))
	}
}
