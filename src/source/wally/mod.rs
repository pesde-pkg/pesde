use crate::{
	GixUrl, Project,
	manifest::target::Target,
	names::wally::WallyPackageName,
	reporters::{DownloadProgressReporter, response_to_async_read},
	ser_display_deser_fromstr,
	source::{
		IGNORED_DIRS, IGNORED_FILES, PackageSources, ResolveResult,
		fs::{FsEntry, PackageFs, store_in_cas},
		git_index::{GitBasedSource, read_file, root_tree},
		ids::VersionId,
		refs::PackageRefs,
		traits::{
			DownloadOptions, GetTargetOptions, PackageSource, RefreshOptions, ResolveOptions,
		},
		wally::{compat_util::get_target, manifest::WallyManifest, pkg_ref::WallyPackageRef},
	},
	util::hash,
	version_matches,
};
use fs_err::tokio as fs;
use relative_path::RelativePathBuf;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use std::{
	collections::{BTreeMap, BTreeSet},
	fmt::Display,
	path::PathBuf,
	str::FromStr,
};
use tokio::{io::AsyncReadExt as _, pin, task::spawn_blocking};
use tokio_util::compat::FuturesAsyncReadCompatExt as _;
use tracing::instrument;

pub(crate) mod compat_util;
pub(crate) mod manifest;
/// The Wally package reference
pub mod pkg_ref;
/// The Wally dependency specifier
pub mod specifier;

/// The Wally package source
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct WallyPackageSource {
	repo_url: GixUrl,
}
ser_display_deser_fromstr!(WallyPackageSource);

impl Display for WallyPackageSource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo_url)
	}
}

impl FromStr for WallyPackageSource {
	type Err = crate::errors::GixUrlError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl GitBasedSource for WallyPackageSource {
	fn path(&self, project: &Project) -> PathBuf {
		project
			.data_dir()
			.join("wally_indices")
			.join(hash(self.as_bytes()))
	}

	fn repo_url(&self) -> &GixUrl {
		&self.repo_url
	}
}

impl WallyPackageSource {
	/// Creates a new Wally package source
	#[must_use]
	pub fn new(repo_url: GixUrl) -> Self {
		Self { repo_url }
	}

	fn as_bytes(&self) -> Vec<u8> {
		self.repo_url.to_string().into_bytes()
	}

	/// Reads the config file
	#[instrument(skip_all, ret(level = "trace"), level = "debug")]
	pub async fn config(&self, project: &Project) -> Result<WallyIndexConfig, errors::ConfigError> {
		let repo_url = self.repo_url.clone();
		let path = self.path(project);

		spawn_blocking(move || {
			let repo = gix::open(&path)?;
			let tree = root_tree(&repo)?;
			let file = read_file(&tree, ["config.json"])?;

			match file {
				Some(s) => serde_json::from_str(&s).map_err(Into::into),
				None => Err(errors::ConfigErrorKind::Missing(repo_url).into()),
			}
		})
		.await
		.unwrap()
	}

	pub(crate) async fn read_index_file(
		&self,
		project: &Project,
		pkg_name: WallyPackageName,
	) -> Result<Option<String>, errors::ResolveError> {
		let path = self.path(project);

		spawn_blocking(move || {
			let repo = gix::open(&path)?;
			let tree = root_tree(&repo)?;
			let (scope, name) = pkg_name.as_str();

			read_file(&tree, [scope, name])
				.map_err(|e| errors::ResolveErrorKind::Read(pkg_name, e).into())
		})
		.await
		.unwrap()
	}
}

impl PackageSource for WallyPackageSource {
	type Specifier = specifier::WallyDependencySpecifier;
	type Ref = WallyPackageRef;
	type RefreshError = crate::source::git_index::errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetTargetError = errors::GetTargetError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, options: &RefreshOptions) -> Result<(), Self::RefreshError> {
		GitBasedSource::refresh(self, options).await
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		specifier: &Self::Specifier,
		options: &ResolveOptions,
	) -> Result<ResolveResult, Self::ResolveError> {
		let ResolveOptions {
			project,
			refreshed_sources,
			..
		} = options;

		let mut string = self
			.read_index_file(project, specifier.name.clone())
			.await?;
		let mut index_url = self.repo_url.clone();

		if string.is_none() {
			tracing::debug!(
				"{} not found in Wally registry. searching in backup registries",
				specifier.name
			);
			let config = self.config(project).await?;
			let refresh_options = RefreshOptions {
				project: project.clone(),
			};

			for url in config.fallback_registries {
				let url = match url.parse() {
					Ok(u) => u,
					Err(e) => {
						tracing::warn!("invalid fallback registry URL {url}: {e}");
						continue;
					}
				};
				let source = WallyPackageSource::new(url);

				match refreshed_sources
					.refresh(&PackageSources::Wally(source.clone()), &refresh_options)
					.await
					.map_err(super::errors::RefreshError::into_inner)
				{
					Ok(()) => {}
					Err(super::errors::RefreshErrorKind::Wally(e)) => {
						return Err(errors::ResolveErrorKind::Refresh(e).into());
					}
					Err(e) => panic!("unexpected error: {e:?}"),
				}

				match source
					.read_index_file(project, specifier.name.clone())
					.await
				{
					Ok(Some(res)) => {
						string = Some(res);
						index_url = source.repo_url;
						break;
					}
					Ok(None) => {
						tracing::debug!("{} not found in {}", specifier.name, source.repo_url);
					}
					Err(e) => return Err(e),
				}
			}
		}

		let Some(string) = string else {
			return Err(errors::ResolveErrorKind::NotFound(specifier.name.clone()).into());
		};

		let entries: Vec<WallyManifest> = string
			.lines()
			.map(serde_json::from_str)
			.collect::<Result<_, _>>()
			.map_err(|e| errors::ResolveErrorKind::Parse(specifier.name.clone(), e))?;

		tracing::debug!("{} has {} possible entries", specifier.name, entries.len());

		let versions = entries
			.into_iter()
			.filter(|manifest| version_matches(&specifier.version, &manifest.package.version))
			.map(|mut manifest| {
				let dependencies = manifest.all_dependencies().map_err(|e| {
					errors::ResolveErrorKind::AllDependencies(specifier.name.clone(), e)
				})?;

				Ok((
					VersionId::new(manifest.package.version, manifest.package.realm.to_target()),
					dependencies,
				))
			})
			.collect::<Result<_, errors::ResolveError>>()?;

		Ok((
			PackageSources::Wally(WallyPackageSource {
				repo_url: index_url,
			}),
			PackageRefs::Wally(WallyPackageRef {
				name: specifier.name.clone(),
			}),
			versions,
			BTreeSet::new(),
		))
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		pkg_ref: &Self::Ref,
		options: &DownloadOptions<'_, R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let DownloadOptions {
			project,
			reqwest,
			reporter,
			version_id,
			..
		} = options;

		let config = self.config(project).await?;
		let index_file = project
			.cas_dir()
			.join("wally_index")
			.join(hash(self.as_bytes()))
			.join(pkg_ref.name.escaped())
			.join(version_id.version().to_string());

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}@{version_id}",
					pkg_ref.name
				);

				reporter.report_done();

				return toml::from_str::<PackageFs>(&s).map_err(Into::into);
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadErrorKind::ReadIndex(e).into()),
		}

		let (scope, name) = pkg_ref.name.as_str();

		let mut request = reqwest
			.get(format!(
				"{}/v1/package-contents/{}/{}/{}",
				config.api.as_str().trim_end_matches('/'),
				urlencoding::encode(scope),
				urlencoding::encode(name),
				urlencoding::encode(&version_id.version().to_string())
			))
			.header(
				"Wally-Version",
				std::env::var("PESDE_WALLY_VERSION")
					.as_deref()
					.unwrap_or("0.3.2"),
			);

		if let Some(token) = project.auth_config().tokens().get(&self.repo_url) {
			tracing::debug!("using token for {}", self.repo_url);
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await?.error_for_status()?;

		let total_len = response.content_length().unwrap_or(0);
		let bytes = response_to_async_read(response, reporter.clone());
		pin!(bytes);

		let mut buf = Vec::with_capacity(total_len as usize);
		bytes.read_to_end(&mut buf).await?;

		let mut archive =
			async_zip::tokio::read::seek::ZipFileReader::with_tokio(std::io::Cursor::new(&mut buf))
				.await?;

		let archive_entries = (0..archive.file().entries().len())
			.map(|index| {
				let entry = archive.file().entries().get(index).unwrap();
				let relative_path = RelativePathBuf::from_path(entry.filename().as_str()?).unwrap();
				Ok::<_, errors::DownloadError>((index, entry.dir()?, relative_path))
			})
			.collect::<Result<Vec<_>, _>>()?;

		let mut entries = BTreeMap::new();

		for (index, is_dir, relative_path) in archive_entries {
			let name = relative_path.file_name().unwrap_or("");
			if if is_dir {
				IGNORED_DIRS.contains(&name)
			} else {
				IGNORED_FILES.contains(&name)
			} {
				continue;
			}

			if is_dir {
				entries.insert(relative_path, FsEntry::Directory);
				continue;
			}

			let entry_reader = archive.reader_without_entry(index).await?;

			let hash = store_in_cas(project.cas_dir(), entry_reader.compat()).await?;

			entries.insert(relative_path, FsEntry::File(hash));
		}

		let fs = PackageFs::Cached(entries);

		if let Some(parent) = index_file.parent() {
			fs::create_dir_all(parent)
				.await
				.map_err(errors::DownloadErrorKind::WriteIndex)?;
		}

		fs::write(&index_file, toml::to_string(&fs)?)
			.await
			.map_err(errors::DownloadErrorKind::WriteIndex)?;

		Ok(fs)
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_target(
		&self,
		_pkg_ref: &Self::Ref,
		options: &GetTargetOptions<'_>,
	) -> Result<Target, Self::GetTargetError> {
		get_target(options).await.map_err(Into::into)
	}
}

/// A Wally index config
#[derive(Debug, Clone, Deserialize)]
pub struct WallyIndexConfig {
	api: url::Url,
	#[serde(default)]
	fallback_registries: Vec<String>,
}

/// Errors that can occur when interacting with a Wally package source
pub mod errors {
	use thiserror::Error;

	use crate::{GixUrl, names::wally::WallyPackageName, source::git_index::errors::ReadFile};

	/// Errors that can occur when resolving a package from a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveError))]
	#[non_exhaustive]
	pub enum ResolveErrorKind {
		/// Error opening repository
		#[error("error opening repository")]
		Open(#[from] gix::open::Error),

		/// Error getting tree
		#[error("error getting tree")]
		Tree(#[from] crate::source::git_index::errors::TreeError),

		/// Package not found in index
		#[error("package {0} not found")]
		NotFound(WallyPackageName),

		/// Error reading file for package
		#[error("error reading file for {0}")]
		Read(WallyPackageName, #[source] ReadFile),

		/// Error parsing file for package
		#[error("error parsing file for {0}")]
		Parse(WallyPackageName, #[source] serde_json::Error),

		/// Error parsing all dependencies
		#[error("error parsing all dependencies for {0}")]
		AllDependencies(
			WallyPackageName,
			#[source] crate::manifest::errors::AllDependenciesError,
		),

		/// Error reading config file
		#[error("error reading config file")]
		Config(#[from] ConfigError),

		/// Error refreshing backup registry source
		#[error("error refreshing backup registry source")]
		Refresh(#[from] crate::source::git_index::errors::RefreshError),

		/// Error resolving package in backup registries
		#[error("error resolving package in backup registries")]
		BackupResolve(#[from] Box<ResolveErrorKind>),
	}

	/// Errors that can occur when reading the config file for a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ConfigError))]
	#[non_exhaustive]
	pub enum ConfigErrorKind {
		/// Error opening repository
		#[error("error opening repository")]
		Open(#[from] gix::open::Error),

		/// Error getting tree
		#[error("error getting tree")]
		Tree(#[from] crate::source::git_index::errors::TreeError),

		/// Error reading file
		#[error("error reading config file")]
		ReadFile(#[from] ReadFile),

		/// Error parsing config file
		#[error("error parsing config file")]
		Parse(#[from] serde_json::Error),

		/// The config file is missing
		#[error("missing config file for index at {0}")]
		Missing(GixUrl),
	}

	/// Errors that can occur when downloading a package from a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// Error reading index file
		#[error("error reading config file")]
		ReadFile(#[from] ConfigError),

		/// Error downloading package
		#[error("error downloading package")]
		Download(#[from] reqwest::Error),

		/// Error deserializing index file
		#[error("error deserializing index file")]
		Deserialize(#[from] toml::de::Error),

		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[source] std::io::Error),

		/// Error decompressing archive
		#[error("error decompressing archive")]
		Decompress(#[from] async_zip::error::ZipError),

		/// Error interacting with the filesystem
		#[error("error interacting with the filesystem")]
		Io(#[from] std::io::Error),

		/// Error serializing index file
		#[error("error serializing index file")]
		SerializeIndex(#[from] toml::ser::Error),

		/// Creating the target failed
		#[error("error creating a target")]
		GetTarget(#[from] crate::source::wally::compat_util::errors::GetTargetError),

		/// Error writing index file
		#[error("error writing index file")]
		WriteIndex(#[source] std::io::Error),
	}

	/// Errors that can occur when getting a target from a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetTargetError))]
	#[non_exhaustive]
	pub enum GetTargetErrorKind {
		/// Error getting target
		#[error("error getting target")]
		GetTarget(#[from] crate::source::wally::compat_util::errors::GetTargetError),
	}
}
