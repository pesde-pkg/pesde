//! pesde package source
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use backend::PesdePackageBackends;
use backend::PesdePackageSourceBackend as _;
use futures::StreamExt as _;

use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::hash::Hash;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::DependencySpecifiers;
use crate::source::IGNORED_DIRS;
use crate::source::IGNORED_FILES;
use crate::source::PackageExports;
use crate::source::PackageRefs;
use crate::source::PackageSource;
use crate::source::ResolveResult;
use crate::source::ResolvedPackage;
use crate::source::SourceState;
use crate::source::fs::FsEntry;
use crate::source::fs::PackageFs;
use crate::source::fs::store_in_cas;
use crate::source::pesde::backend::ApiPesdePackageSourceBackend;
use crate::source::pesde::registry::LogHeadResponseState;
use crate::util::ToEscaped as _;
use fs_err::tokio as fs;
use serde::Deserialize;
use serde::Serialize;
use tracing::instrument;

pub mod backend;
pub mod pkg_ref;
pub mod registry;
pub mod specifier;

/// State for a pesde package source (MMR data)
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PesdeSourceState {
	/// The MMR size for this source (number of entries in the log)
	pub mmr_size: u64,
	/// The MMR accumulator (peak hashes) for this source
	pub accumulator: Vec<Hash>,
}

/// The pesde package source
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct PesdePackageSource {
	repo: PesdePackageBackends,
}
ser_display_deser_fromstr!(PesdePackageSource);

impl Display for PesdePackageSource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo)
	}
}

impl FromStr for PesdePackageSource {
	type Err = crate::source::pesde::backend::errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl PesdePackageSource {
	/// Creates a new pesde package source
	#[must_use]
	pub fn new(repo: PesdePackageBackends) -> Self {
		Self { repo }
	}

	/// Creates a pesde package source from a URL
	#[must_use]
	pub fn from_url(api_url: impl Into<Arc<url::Url>>) -> Self {
		Self::new(PesdePackageBackends::Api(
			ApiPesdePackageSourceBackend::new(api_url),
		))
	}

	/// Gets the repository backend
	#[must_use]
	pub fn repo(&self) -> &PesdePackageBackends {
		&self.repo
	}
}

impl PackageSource for PesdePackageSource {
	type RefreshIndexError = errors::RefreshIndexError;
	type RefreshStateError = errors::RefreshStateError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetExportsError = errors::GetExportsError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh_index(&self, project: &Project) -> Result<(), Self::RefreshIndexError> {
		self.repo.refresh_index(project).await
	}

	#[instrument(skip_all, level = "debug")]
	async fn refresh_state(
		&self,
		project: &Project,
		old_state: Option<&SourceState>,
	) -> Result<Option<SourceState>, Self::RefreshStateError> {
		let old_state = old_state.map(|old_state| {
			let SourceState::Pesde(old_state) = old_state else {
				unreachable!("invalid source state type for pesde package source");
			};

			old_state
		});

		let new_state = self.repo.fetch_state(project, old_state).await?;
		let new_state = match (old_state, new_state) {
			(Some(_), None) => return Err(errors::RefreshStateErrorKind::NoNewState.into()),
			(None, None) => return Ok(None),
			(None, Some(new_state)) => {
				let LogHeadResponseState::OnlyNewState { mmr_size_to } = new_state.state else {
					return Err(errors::RefreshStateErrorKind::InvalidResponseState.into());
				};

				PesdeSourceState {
					mmr_size: mmr_size_to,
					accumulator: new_state.accumulator,
				}
			}
			(Some(old_state), Some(new_state)) => {
				let LogHeadResponseState::WithPreviousState { proof } = new_state.state else {
					return Err(errors::RefreshStateErrorKind::InvalidResponseState.into());
				};

				if !proof.verify(&old_state.accumulator, &new_state.accumulator)? {
					return Err(errors::RefreshStateErrorKind::ConsistencyProofFailed.into());
				}

				PesdeSourceState {
					mmr_size: proof.mmr_size_to(),
					accumulator: new_state.accumulator,
				}
			}
		};

		Ok(Some(SourceState::Pesde(new_state)))
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		_subproject: &Subproject,
		_specifier: &DependencySpecifiers,
		_refreshed_sources: &RefreshedSources,
	) -> Result<ResolveResult, Self::ResolveError> {
		todo!()
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &ResolvedPackage,
		reporter: Arc<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let PackageRefs::Pesde(pkg_ref) = package.id.pkg_ref() else {
			unreachable!("invalid package ref type for pesde package source");
		};

		let index_file = project
			.cas_dir()
			.join("index")
			.join("pesde")
			.join(self.repo.to_string().escaped())
			.join(pkg_ref.name.scope().to_string().escaped())
			.join(pkg_ref.name.name().to_string().escaped())
			.join(package.id.version().to_string());

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}@{}",
					pkg_ref.name,
					package.id.version()
				);

				reporter.report_done();

				return toml::from_str::<PackageFs>(&s).map_err(Into::into);
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadErrorKind::ReadIndex(e).into()),
		}

		let entries_stream = self
			.repo
			.download_entries(project, &pkg_ref.name, package.id.version(), reporter)
			.await?;
		tokio::pin!(entries_stream);

		let mut entries = BTreeMap::new();

		while let Some(entry_result) = entries_stream.next().await {
			let (path, contents) = entry_result?;
			let Some(name) = path.file_name() else {
				continue;
			};

			let Some(contents) = contents else {
				if IGNORED_DIRS.contains(&name) {
					continue;
				}
				entries.insert(path, FsEntry::Directory);
				continue;
			};

			if IGNORED_FILES.contains(&name) {
				continue;
			}

			let (_, hash) = store_in_cas(project.cas_dir(), &*contents)
				.await
				.map_err(errors::DownloadErrorKind::WriteIndex)?;
			entries.insert(path, FsEntry::File(hash));
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
	async fn get_exports(
		&self,
		_project: &Project,
		_package: &ResolvedPackage,
		_path: &Path,
	) -> Result<PackageExports, Self::GetExportsError> {
		todo!()
	}
}

/// Errors that can occur when interacting with the pesde package source
pub mod errors {
	use thiserror::Error;

	use crate::names::PackageName;

	/// Errors that can occur when refreshing the pesde package source index
	pub type RefreshIndexError = super::backend::errors::RefreshIndexError;

	/// Errors that can occur when refreshing the pesde package source state
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RefreshStateError))]
	#[non_exhaustive]
	pub enum RefreshStateErrorKind {
		/// Error from backend
		#[error("error refreshing state")]
		Backend(#[from] super::backend::errors::RefreshStateError),

		/// The backend has not returned a new state despite there being a previous state
		#[error("backend did not return a new state despite there being a previous state")]
		NoNewState,

		/// The consistency proof from the backend did not verify against the previous state
		#[error("consistency proof did not verify against previous state")]
		ConsistencyProofFailed,

		/// Error interacting with Merkleberg
		#[error("error verifying consistency proof")]
		Merkleberg(#[from] merkleberg::Error),

		/// The response state was invalid as compared to the client's state (e.g. a TOFU client got a response with a consistency proof)
		#[error("invalid response state from backend")]
		InvalidResponseState,
	}

	/// Errors that can occur when resolving a package from a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveError))]
	#[non_exhaustive]
	pub enum ResolveErrorKind {
		/// Package not found in index
		#[error("package `{0}` not found")]
		NotFound(PackageName),
	}

	/// Errors that can occur when downloading a package from a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// Error from backend
		#[error("error from backend")]
		Backend(#[from] crate::source::pesde::backend::errors::DownloadError),

		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[source] std::io::Error),

		/// Error writing index file
		#[error("error writing index file")]
		WriteIndex(#[source] std::io::Error),

		/// Error serializing index file
		#[error("error serializing index file")]
		SerializeIndex(#[from] toml::ser::Error),

		/// Error deserializing index file
		#[error("error deserializing index file")]
		DeserializeIndex(#[from] toml::de::Error),
	}

	/// Errors that can occur when getting the target for a package from a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetExportsError))]
	#[non_exhaustive]
	pub enum GetExportsErrorKind {
		/// Package not found in index
		#[error("package `{0}` not found in index")]
		NotFound(PackageName),
	}
}
