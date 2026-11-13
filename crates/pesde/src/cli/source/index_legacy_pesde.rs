#![expect(deprecated)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use futures::AsyncWriteExt as _;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream;
use itertools::Itertools as _;
use pesde::__private::str_as_fs_safe;
use pesde::Url;
use pesde::legacy::RelativePathBuf;
use pesde::legacy::Target;
use pesde::legacy::TargetKind;
use pesde::link::PackageStructure;
use pesde::registry::PackageName;
use pesde::registry::manifest::Alias;
use pesde::registry::manifest::DependencyType;
use pesde::registry::manifest::wally_compat::WallyPackageName;
use pesde::reporters::DownloadProgressReporter;
use pesde::semver::Version;
use pesde::semver::VersionReq;
use pesde::source::DependencySpecifier;
use pesde::source::GitDependencySpecifier;
use pesde::source::InspectResult;
use pesde::source::LegacyPesdeDependencySpecifier;
use pesde::source::PackageId;
use pesde::source::PackageSource;
use pesde::source::ResolveResult;
use pesde::source::SourceError;
use pesde::source::WallyDependencySpecifier;
use pesde::vfs::PackageStorer;
use pesde::vfs::VfsPath;
use reqwest::header::ACCEPT;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use serde::Deserializer;
use serde::de::IgnoredAny;
use tokio::io::AsyncReadExt as _;
use tokio::task::spawn_blocking;

use crate::cli::dirs::cache_dir;
use crate::cli::reporters::response_to_async_buf_read;
use crate::cli::rootless_url;
use crate::cli::source::git_index::read_file;
use crate::cli::source::git_index::refresh_git_repo;
use crate::cli::source::git_index::root_tree;
use crate::cli::url_component_encode;

const IGNORED_FILES: &[&str] = &["foreman.toml", "aftman.toml", "rokit.toml", ".DS_Store"];

pub struct IndexLegacyPesdePackageSource {
	path: Box<Path>,
	config: IndexConfig,
	reqwest: reqwest::Client,
	authorization: Option<Box<str>>,
}

impl IndexLegacyPesdePackageSource {
	pub async fn new(
		url: Url,
		reqwest: reqwest::Client,
		authorization: Option<Box<str>>,
	) -> anyhow::Result<Self> {
		let path = cache_dir()?
			.join("git_repos")
			.join("pesde")
			.join(str_as_fs_safe(url.as_str()))
			.into_boxed_path();

		refresh_git_repo(path.clone().into_path_buf(), url.clone()).await?;

		spawn_blocking(move || {
			let repo = gix::open(path.clone())?;
			let tree = root_tree(&repo)?;
			let file = read_file(&tree, ["config.toml"])?;

			match file {
				Some(s) => Ok(Self {
					path,
					config: toml::from_slice(&s)
						.context("failed to parse config.toml of legacy pesde index")?,
					reqwest,
					authorization,
				}),
				None => {
					anyhow::bail!(
						"config.toml not found in legacy pesde index git repository `{url}`"
					)
				}
			}
		})
		.await
		.unwrap()
	}

	async fn index_file(&self, name: PackageName) -> anyhow::Result<Option<IndexFile>> {
		let file = spawn_blocking({
			let path = self.path.clone();

			move || {
				let repo = gix::open(path)?;
				let tree = root_tree(&repo)?;
				let Some(file) =
					read_file(&tree, [name.scope().as_str(), name.local_name().as_str()])?
				else {
					return Ok(None);
				};

				let file = toml::from_slice(&file).with_context(|| {
					format!("failed to parse legacy pesde index file for package `{name}`")
				})?;

				anyhow::Ok(Some(file))
			}
		})
		.await??;

		Ok(file)
	}
}

#[async_trait]
impl PackageSource for IndexLegacyPesdePackageSource {
	async fn resolve(&self, specifier: &DependencySpecifier) -> Result<ResolveResult, SourceError> {
		let DependencySpecifier::LegacyPesde(specifier) = specifier else {
			return Err(anyhow::format_err!(
				"invalid dependency specifier for index legacy pesde source: {specifier}"
			)
			.into());
		};

		let LegacyPesdeDependencySpecifier {
			name,
			req,
			index,
			target,
		} = specifier;

		let Some(file) = self.index_file(name.clone()).await? else {
			return Err(anyhow::format_err!(
				"package `{name}` not found in legacy pesde index `{index}`"
			)
			.into());
		};

		tracing::debug!("{name} has {} possible entries", file.entries.len());

		let mut suggestions = BTreeSet::new();
		let mut candidates = Vec::new();

		for (VersionId(version, entry_target), entry) in &file.entries {
			if entry.yanked || !req.matches(version) {
				continue;
			}

			suggestions.insert(*entry_target);

			if entry_target != target {
				continue;
			}

			match entry.inspect_result() {
				Ok(inspect) => candidates.push((
					PackageId::LegacyPesde(index.clone(), name.clone(), version.clone(), *target),
					Some(inspect),
				)),
				Err(e) => tracing::warn!("skipping `{name}@{version}` ({target}): {e:#}"),
			}
		}

		if candidates.is_empty() && !suggestions.is_empty() {
			return Err(anyhow::format_err!(
				"no matching version found for `{name}@{req}` with target `{target}`. available targets: {}",
				suggestions.iter().format(", ")
			)
			.into());
		}

		Ok(ResolveResult::Found(
			stream::iter(candidates.into_iter().map(Ok::<_, SourceError>)).boxed(),
		))
	}

	async fn download(
		&self,
		package_id: &PackageId,
		reporter: Arc<dyn DownloadProgressReporter + 'static>,
		to: &mut dyn PackageStorer,
	) -> Result<(), SourceError> {
		let PackageId::LegacyPesde(_, name, version, target) = package_id else {
			return Err(anyhow::format_err!(
				"invalid package id for index legacy pesde source: {package_id}"
			)
			.into());
		};

		let url = self.config.download_url(name, version, *target);

		let mut request = self
			.reqwest
			.get(&url)
			.header(ACCEPT, "application/octet-stream");

		if let Some(authorization) = &self.authorization {
			request = request.header(AUTHORIZATION, &**authorization);
		}

		let response = request.send().await?.error_for_status()?;

		let bytes = response_to_async_buf_read(response, reporter);
		tokio::pin!(bytes);

		let decoder = async_compression::tokio::bufread::GzipDecoder::new(bytes);
		let mut archive = tokio_tar::Archive::new(decoder);
		let mut entries = archive
			.entries()
			.context("failed to open legacy pesde package archive")?;

		let mut buf = vec![0; 8 * 1024];

		while let Some(mut entry) = entries
			.try_next()
			.await
			.context("failed to read entry of legacy pesde package archive")?
		{
			let entry_type = entry.header().entry_type();
			let path = entry
				.path()
				.context("invalid path in legacy pesde package archive")?
				.into_owned();

			tracing::trace!(
				"entry in legacy pesde package `{package_id}`: {}",
				path.display()
			);

			let vfs_path = VfsPath::from_path(&path).with_context(|| {
				format!("legacy pesde package archive entry `{}`", path.display())
			})?;

			if entry_type.is_dir() {
				to.directory_at(vfs_path).await?;
				continue;
			}

			if path
				.file_name()
				.and_then(|n| n.to_str())
				.is_some_and(|n| IGNORED_FILES.contains(&n))
			{
				continue;
			}

			let size_hint = entry.effective_size();
			let file = to.file_at(vfs_path, size_hint).await?;

			while let n = entry.read(&mut buf).await?
				&& n > 0
			{
				file.write_all(&buf[..n]).await?;
			}
		}

		Ok(())
	}

	async fn inspect(&self, package_id: &PackageId) -> Result<InspectResult, SourceError> {
		let PackageId::LegacyPesde(_, name, version, target) = package_id else {
			return Err(anyhow::format_err!(
				"invalid package id for index legacy pesde source: {package_id}"
			)
			.into());
		};

		let file = self
			.index_file(name.clone())
			.await?
			.with_context(|| format!("package `{name}` not found in legacy pesde index"))?;

		let entry = file
			.entries
			.get(&VersionId(version.clone(), *target))
			.with_context(|| {
				format!(
					"version `{version}` ({target}) of `{name}` not found in legacy pesde index"
				)
			})?;

		Ok(entry.inspect_result()?)
	}
}

#[derive(Debug, Deserialize)]
struct IndexConfig {
	api: Url,
	#[serde(default)]
	download: Option<Box<str>>,
}

impl IndexConfig {
	fn download_url(&self, name: &PackageName, version: &Version, target: TargetKind) -> String {
		self.download
			.as_deref()
			.unwrap_or("{API_URL}/v1/packages/{PACKAGE}/{PACKAGE_VERSION}/{PACKAGE_TARGET}/archive")
			.replace("{API_URL}", rootless_url(&self.api))
			.replace(
				"{PACKAGE}",
				&url_component_encode(&name.to_string()).to_string(),
			)
			.replace(
				"{PACKAGE_VERSION}",
				&url_component_encode(&version.to_string()).to_string(),
			)
			.replace(
				"{PACKAGE_TARGET}",
				&url_component_encode(&target.to_string()).to_string(),
			)
	}
}

#[derive(Debug, Deserialize)]
struct IndexFile {
	#[serde(default)]
	#[expect(unused)]
	meta: IgnoredAny,
	#[serde(flatten)]
	entries: BTreeMap<VersionId, IndexFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VersionId(Version, TargetKind);

impl<'de> Deserialize<'de> for VersionId {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct Visitor;
		impl serde::de::Visitor<'_> for Visitor {
			type Value = VersionId;

			fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(formatter, "a version id in the format of `version target`")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where
				E: serde::de::Error,
			{
				let (version, target) = v
					.split_once(' ')
					.ok_or_else(|| E::custom(format!("malformed version id `{v}`")))?;

				Ok(VersionId(
					version.parse().map_err(E::custom)?,
					target.parse().map_err(E::custom)?,
				))
			}
		}

		deserializer.deserialize_str(Visitor)
	}
}

#[derive(Debug, Deserialize)]
struct IndexFileEntry {
	target: Target,
	#[serde(default)]
	yanked: bool,
	#[serde(default)]
	dependencies: BTreeMap<Alias, (LegacyLegacyPesdeDependencySpecifier, DependencyType)>,
}

impl IndexFileEntry {
	fn inspect_result(&self) -> anyhow::Result<InspectResult> {
		let mut dependencies = BTreeMap::new();

		for (alias, (specifier, dep_ty)) in &self.dependencies {
			let specifier = match specifier {
				LegacyLegacyPesdeDependencySpecifier::LegacyPesde(s) => {
					DependencySpecifier::LegacyPesde(LegacyPesdeDependencySpecifier {
						name: s.name.clone(),
						req: s.version.clone(),
						index: s.index.parse().context("invalid index specifier")?,
						target: s.target.unwrap_or(self.target.kind()),
					})
				}
				LegacyLegacyPesdeDependencySpecifier::Wally(s) => {
					DependencySpecifier::Wally(WallyDependencySpecifier {
						name: s.name.clone(),
						req: s.version.clone(),
						index: s.index.parse().context("invalid index specifier")?,
					})
				}
				LegacyLegacyPesdeDependencySpecifier::Git(s) => {
					DependencySpecifier::Git(pesde::source::GitDependencySpecifier {
						url: s.repo.clone(),
						rev: s.rev.clone(),
						path: s.path.0.clone(),
					})
				}
				LegacyLegacyPesdeDependencySpecifier::Workspace(s) => {
					anyhow::bail!(
						"workspace dependencies are not supported in legacy pesde index: {s:?}"
					);
				}
			};

			dependencies.insert(alias.clone(), (specifier, *dep_ty));
		}

		Ok(InspectResult {
			structure: PackageStructure::LegacyPesde(
				self.target
					.exports()
					.map_err(anyhow::Error::from_boxed)
					.context("legacy pesde index file")?,
				self.target.kind(),
			),
			dependencies,
		})
	}
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LegacyLegacyPesdeDependencySpecifier {
	LegacyPesde(LegacyPesdeLegacyPesdeDependencySpecifier),
	Wally(LegacyPesdeWallyDependencySpecifier),
	Git(LegacyPesdeGitDependencySpecifier),
	Workspace(LegacyPesdeWorkspaceDependencySpecifier),
}

fn default_index() -> Box<str> {
	Box::from("default")
}

#[derive(Debug, Deserialize)]
struct LegacyPesdeLegacyPesdeDependencySpecifier {
	name: PackageName,
	version: VersionReq,
	#[serde(default = "default_index")]
	index: Box<str>,
	target: Option<TargetKind>,
}

#[derive(Debug, Deserialize)]
struct LegacyPesdeWallyDependencySpecifier {
	#[serde(rename = "wally")]
	name: WallyPackageName,
	version: VersionReq,
	#[serde(default = "default_index")]
	index: Box<str>,
}

#[derive(Debug, Deserialize)]
struct LegacyPesdeGitDependencySpecifier {
	repo: Url,
	rev: Arc<str>,
	#[serde(default)]
	path: RelativePathBuf,
}

#[derive(Debug, Deserialize)]
struct LegacyPesdeWorkspaceDependencySpecifier {
	#[expect(unused)]
	workspace: PackageName,
}

#[derive(Debug, Deserialize)]
pub struct LegacyPesdeManifest {
	pub target: Target,
	#[serde(default)]
	indices: BTreeMap<Box<str>, Url>,
	#[serde(default)]
	wally_indices: BTreeMap<Box<str>, Url>,
	#[serde(default)]
	dependencies: BTreeMap<Alias, LegacyLegacyPesdeDependencySpecifier>,
	#[serde(default)]
	peer_dependencies: BTreeMap<Alias, LegacyLegacyPesdeDependencySpecifier>,
	#[serde(default)]
	dev_dependencies: BTreeMap<Alias, LegacyLegacyPesdeDependencySpecifier>,
}

impl LegacyPesdeManifest {
	pub(crate) fn all_dependencies(
		self,
	) -> anyhow::Result<BTreeMap<Alias, (DependencySpecifier, DependencyType)>> {
		let mut deps = BTreeMap::new();

		let iter = self
			.dependencies
			.into_iter()
			.zip(std::iter::repeat(DependencyType::Standard))
			.chain(
				self.peer_dependencies
					.into_iter()
					.zip(std::iter::repeat(DependencyType::Peer)),
			)
			.chain(
				self.dev_dependencies
					.into_iter()
					.zip(std::iter::repeat(DependencyType::Dev)),
			);

		for ((alias, specifier), dep_ty) in iter {
			let specifier = match specifier {
				LegacyLegacyPesdeDependencySpecifier::LegacyPesde(s) => {
					DependencySpecifier::LegacyPesde(LegacyPesdeDependencySpecifier {
						name: s.name,
						req: s.version,
						index: self
							.indices
							.get(&*s.index)
							.with_context(|| format!("invalid index specifier `{}`", s.index))?
							.clone(),
						target: s.target.unwrap_or(self.target.kind()),
					})
				}
				LegacyLegacyPesdeDependencySpecifier::Wally(s) => {
					DependencySpecifier::Wally(WallyDependencySpecifier {
						name: s.name,
						req: s.version,
						index: self
							.wally_indices
							.get(&*s.index)
							.with_context(|| format!("invalid index specifier `{}`", s.index))?
							.clone(),
					})
				}
				LegacyLegacyPesdeDependencySpecifier::Git(s) => {
					DependencySpecifier::Git(GitDependencySpecifier {
						url: s.repo,
						rev: s.rev,
						path: s.path.0,
					})
				}
				LegacyLegacyPesdeDependencySpecifier::Workspace(s) => {
					anyhow::bail!(
						"workspace dependencies are not supported in legacy pesde index: {s:?}"
					);
				}
			};

			match deps.entry(alias) {
				std::collections::btree_map::Entry::Vacant(e) => {
					e.insert((specifier, dep_ty));
				}
				std::collections::btree_map::Entry::Occupied(e) => {
					anyhow::bail!(
						"duplicate dependency alias `{}` for legacy pesde package manifest",
						e.key()
					);
				}
			}
		}

		Ok(deps)
	}
}
