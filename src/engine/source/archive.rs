use futures::StreamExt as _;
use ouroboros::self_referencing;
use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
	pin::Pin,
	str::FromStr,
	task::{Context, Poll},
};
use tokio::{
	io::{AsyncBufRead, AsyncRead, AsyncReadExt as _, ReadBuf},
	pin,
};
use tokio_util::compat::Compat;

/// The kind of encoding used for the archive
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncodingKind {
	/// Gzip
	Gzip,
}

/// The kind of archive
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveKind {
	/// Tar
	Tar,
	/// Zip
	Zip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ArchiveInfo(ArchiveKind, Option<EncodingKind>);

impl FromStr for ArchiveInfo {
	type Err = errors::ArchiveInfoFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let parts = s.split('.').collect::<Vec<_>>();

		Ok(match &*parts {
			[.., "tar", "gz"] => ArchiveInfo(ArchiveKind::Tar, Some(EncodingKind::Gzip)),
			[.., "tar"] => ArchiveInfo(ArchiveKind::Tar, None),
			[.., "zip", "gz"] => {
				return Err(errors::ArchiveInfoFromStrError::Unsupported(
					ArchiveKind::Zip,
					Some(EncodingKind::Gzip),
				));
			}
			[.., "zip"] => ArchiveInfo(ArchiveKind::Zip, None),
			_ => return Err(errors::ArchiveInfoFromStrError::Invalid(s.to_string())),
		})
	}
}

pub(crate) type ArchiveReader = Pin<Box<dyn AsyncBufRead + Send>>;

/// An archive
pub struct Archive {
	pub(crate) info: ArchiveInfo,
	pub(crate) reader: ArchiveReader,
}

enum TarReader {
	Gzip(async_compression::tokio::bufread::GzipDecoder<ArchiveReader>),
	Plain(ArchiveReader),
}

impl AsyncRead for TarReader {
	fn poll_read(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		match Pin::into_inner(self) {
			Self::Gzip(r) => Pin::new(r).poll_read(cx, buf),
			Self::Plain(r) => Pin::new(r).poll_read(cx, buf),
		}
	}
}

#[self_referencing]
struct ZipArchiveEntry {
	archive: async_zip::tokio::read::seek::ZipFileReader<std::io::Cursor<Vec<u8>>>,
	#[borrows(mut archive)]
	#[not_covariant]
	reader: Compat<
		async_zip::tokio::read::ZipEntryReader<
			'this,
			std::io::Cursor<Vec<u8>>,
			async_zip::base::read::WithoutEntry,
		>,
	>,
}

enum ArchiveEntryInner {
	Tar(Box<tokio_tar::Entry<tokio_tar::Archive<TarReader>>>),
	Zip(ZipArchiveEntry),
}

/// An entry in an archive. Usually the executable
pub struct ArchiveEntry(ArchiveEntryInner);

impl AsyncRead for ArchiveEntry {
	fn poll_read(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		match &mut Pin::into_inner(self).0 {
			ArchiveEntryInner::Tar(r) => Pin::new(r).poll_read(cx, buf),
			ArchiveEntryInner::Zip(z) => {
				z.with_reader_mut(|reader| Pin::new(reader).poll_read(cx, buf))
			}
		}
	}
}

impl Archive {
	/// Finds the executable in the archive and returns it as an [`ArchiveEntry`]
	pub async fn find_executable(
		self,
		expected_file_name: &str,
	) -> Result<ArchiveEntry, errors::FindExecutableError> {
		#[derive(Debug, PartialEq, Eq)]
		struct Candidate {
			path: PathBuf,
			file_name_matches: bool,
			extension_matches: bool,
			has_permissions: bool,
		}

		impl Candidate {
			fn new(path: PathBuf, perms: u32, expected_file_name: &str) -> Self {
				Self {
					file_name_matches: path
						.file_name()
						.is_some_and(|name| name == expected_file_name),
					extension_matches: match path.extension() {
						Some(ext) if ext == std::env::consts::EXE_EXTENSION => true,
						None if std::env::consts::EXE_EXTENSION.is_empty() => true,
						_ => false,
					},
					path,
					has_permissions: perms & 0o111 != 0,
				}
			}

			fn should_be_considered(&self) -> bool {
				// if nothing matches, we should not consider this candidate as it is most likely not
				self.file_name_matches || self.extension_matches || self.has_permissions
			}
		}

		impl Ord for Candidate {
			fn cmp(&self, other: &Self) -> std::cmp::Ordering {
				self.file_name_matches
					.cmp(&other.file_name_matches)
					.then(self.extension_matches.cmp(&other.extension_matches))
					.then(self.has_permissions.cmp(&other.has_permissions))
			}
		}

		impl PartialOrd for Candidate {
			fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
				Some(self.cmp(other))
			}
		}

		let mut candidates = BTreeSet::new();

		match self.info {
			ArchiveInfo(ArchiveKind::Tar, encoding) => {
				use async_compression::tokio::bufread as decoders;

				let reader = match encoding {
					Some(EncodingKind::Gzip) => {
						TarReader::Gzip(decoders::GzipDecoder::new(self.reader))
					}
					None => TarReader::Plain(self.reader),
				};

				let mut archive = tokio_tar::Archive::new(reader);
				let mut entries = archive.entries()?;

				while let Some(entry) = entries.next().await.transpose()? {
					if entry.header().entry_type().is_dir() {
						continue;
					}

					let candidate = Candidate::new(
						entry.path()?.to_path_buf(),
						entry.header().mode()?,
						expected_file_name,
					);
					if candidate.should_be_considered() {
						candidates.insert(candidate);
					}
				}

				let Some(candidate) = candidates.pop_last() else {
					return Err(errors::FindExecutableError::ExecutableNotFound);
				};

				let mut entries = archive.entries()?;

				while let Some(entry) = entries.next().await.transpose()? {
					if entry.header().entry_type().is_dir() {
						continue;
					}

					let path = entry.path()?;
					if path == candidate.path {
						return Ok(ArchiveEntry(ArchiveEntryInner::Tar(Box::new(entry))));
					}
				}
			}
			ArchiveInfo(ArchiveKind::Zip, _) => {
				let reader = self.reader;
				pin!(reader);

				// TODO: would be lovely to not have to read the whole archive into memory
				let mut buf = vec![];
				reader.read_to_end(&mut buf).await?;

				let archive = async_zip::base::read::seek::ZipFileReader::with_tokio(
					std::io::Cursor::new(buf),
				)
				.await?;
				for entry in archive.file().entries() {
					if entry.dir()? {
						continue;
					}

					let path: &Path = entry.filename().as_str()?.as_ref();
					let candidate = Candidate::new(
						path.to_path_buf(),
						entry.unix_permissions().unwrap_or(0) as u32,
						expected_file_name,
					);
					if candidate.should_be_considered() {
						candidates.insert(candidate);
					}
				}

				let Some(candidate) = candidates.pop_last() else {
					return Err(errors::FindExecutableError::ExecutableNotFound);
				};

				for (i, entry) in archive.file().entries().iter().enumerate() {
					if entry.dir()? {
						continue;
					}

					let path: &Path = entry.filename().as_str()?.as_ref();
					if candidate.path == path {
						let entry = ZipArchiveEntryAsyncSendTryBuilder {
							archive,
							reader_builder: |archive| {
								Box::pin(async move {
									archive
										.reader_without_entry(i)
										.await
										.map(tokio_util::compat::FuturesAsyncReadCompatExt::compat)
								})
							},
						}
						.try_build()
						.await?;

						return Ok(ArchiveEntry(ArchiveEntryInner::Zip(entry)));
					}
				}
			}
		}

		Err(errors::FindExecutableError::ExecutableNotFound)
	}
}

/// Errors that can occur when working with archives
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing archive info
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ArchiveInfoFromStrError {
		/// The string is not a valid archive descriptor. E.g. `{name}.tar.gz`
		#[error("string `{0}` is not a valid archive descriptor")]
		Invalid(String),

		/// The archive type is not supported. E.g. `{name}.zip.gz`
		#[error("archive type {0:?} with encoding {1:?} is not supported")]
		Unsupported(super::ArchiveKind, Option<super::EncodingKind>),
	}

	/// Errors that can occur when finding an executable in an archive
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum FindExecutableError {
		/// The executable was not found in the archive
		#[error("failed to find executable in archive")]
		ExecutableNotFound,

		/// An IO error occurred
		#[error("IO error")]
		Io(#[from] std::io::Error),

		/// An error occurred reading the zip archive
		#[error("failed to read zip archive")]
		Zip(#[from] async_zip::error::ZipError),
	}
}
