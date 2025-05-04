use crate::{
	manifest::target::TargetKind,
	source::{ADDITIONAL_FORBIDDEN_FILES, IGNORED_DIRS, IGNORED_FILES},
	util,
};
use fs_err::tokio as fs;
use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
	collections::BTreeMap,
	fmt::Debug,
	path::{Path, PathBuf},
};
use tempfile::Builder;
use tokio::{
	io::{AsyncReadExt as _, AsyncWriteExt as _},
	pin,
	task::JoinSet,
};
use tracing::instrument;

/// A file system entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FsEntry {
	/// A file with the given hash
	#[serde(rename = "f")]
	File(String),
	/// A directory
	#[serde(rename = "d")]
	Directory,
}

/// A package's file system
#[derive(Debug, Clone, Serialize, Deserialize)]
// don't need to differentiate between CAS and non-CAS, since non-CAS won't be serialized
#[serde(untagged)]
pub enum PackageFs {
	/// A package stored in the CAS
	Cas(BTreeMap<RelativePathBuf, FsEntry>),
	/// A package that's to be copied
	Copy(PathBuf, TargetKind),
}

async fn set_readonly(path: &Path, readonly: bool) -> std::io::Result<()> {
	// on Windows, file deletion is disallowed if the file is read-only which breaks multiple features
	#[cfg(windows)]
	if readonly {
		return Ok(());
	}

	let mut permissions = fs::metadata(path).await?.permissions();
	if readonly {
		permissions.set_readonly(true);
	} else {
		#[cfg(windows)]
		#[allow(clippy::permissions_set_readonly_false)]
		{
			permissions.set_readonly(false);
		}

		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			permissions.set_mode(permissions.mode() | 0o644);
		}
	}

	fs::set_permissions(path, permissions).await
}

pub(crate) fn cas_path(hash: &str, cas_dir: &Path) -> PathBuf {
	let (prefix, rest) = hash.split_at(2);
	cas_dir.join(prefix).join(rest)
}

pub(crate) async fn store_in_cas<R: tokio::io::AsyncRead + Unpin, P: AsRef<Path>>(
	cas_dir: P,
	mut contents: R,
) -> std::io::Result<String> {
	let tmp_dir = cas_dir.as_ref().join(".tmp");
	fs::create_dir_all(&tmp_dir).await?;
	let mut hasher = Sha256::new();
	let mut buf = [0; 8 * 1024];

	let temp_path = Builder::new()
		.make_in(&tmp_dir, |_| Ok(()))?
		.into_temp_path();
	let mut file_writer = fs::File::create(temp_path.to_path_buf()).await?;

	loop {
		let bytes_future = contents.read(&mut buf);
		pin!(bytes_future);
		let bytes_read = bytes_future.await?;

		if bytes_read == 0 {
			break;
		}

		let bytes = &buf[..bytes_read];
		hasher.update(bytes);
		file_writer.write_all(bytes).await?;
	}

	let hash = format!("{:x}", hasher.finalize());

	let cas_path = cas_path(&hash, cas_dir.as_ref());
	fs::create_dir_all(cas_path.parent().unwrap()).await?;

	match temp_path.persist_noclobber(&cas_path) {
		Ok(_) => {
			set_readonly(&cas_path, true).await?;
		}
		Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
		Err(e) => return Err(e.error),
	}

	Ok(hash)
}

async fn package_fs_cas(
	entries: BTreeMap<RelativePathBuf, FsEntry>,
	destination: &Path,
	cas_dir_path: &Path,
	link: bool,
) -> std::io::Result<()> {
	let mut tasks = entries
		.iter()
		.map(|(path, entry)| {
			let destination = destination.to_path_buf();
			let cas_dir_path = cas_dir_path.to_path_buf();
			let path = path.to_path(destination);
			let entry = entry.clone();

			async move {
				match entry {
					FsEntry::File(hash) => {
						if let Some(parent) = path.parent() {
							fs::create_dir_all(parent).await?;
						}

						let cas_file_path = cas_path(&hash, &cas_dir_path);

						if link {
							fs::hard_link(cas_file_path, path).await?;
						} else {
							fs::copy(cas_file_path, &path).await?;
							set_readonly(&path, false).await?;
						}
					}
					FsEntry::Directory => {
						fs::create_dir_all(path).await?;
					}
				}

				Ok::<_, std::io::Error>(())
			}
		})
		.collect::<JoinSet<_>>();

	while let Some(task) = tasks.join_next().await {
		task.unwrap()?;
	}

	Ok(())
}

async fn package_fs_copy(
	src: &Path,
	target: TargetKind,
	destination: &Path,
) -> std::io::Result<()> {
	fs::create_dir_all(destination).await?;

	let mut tasks = JoinSet::new();
	let mut read_dir = fs::read_dir(src).await?;

	'entry: while let Some(entry) = read_dir.next_entry().await? {
		let path = entry.path();
		let relative_path = path.strip_prefix(src).unwrap();
		let dest_path = destination.join(relative_path);
		let file_name = relative_path.file_name().unwrap().to_str().ok_or_else(|| {
			std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid file name")
		})?;

		if entry.file_type().await?.is_dir() {
			if IGNORED_DIRS.contains(&file_name) {
				continue;
			}

			for other_target in TargetKind::VARIANTS {
				if target.packages_folder(*other_target) == file_name {
					continue 'entry;
				}
			}

			tasks.spawn(async { util::symlink_dir(path, dest_path).await });
			continue;
		}

		if IGNORED_FILES.contains(&file_name) || ADDITIONAL_FORBIDDEN_FILES.contains(&file_name) {
			continue;
		}

		tasks.spawn(async { util::symlink_file(path, dest_path).await });
	}

	while let Some(task) = tasks.join_next().await {
		task.unwrap()?;
	}

	Ok(())
}

impl PackageFs {
	/// Write the package to the given destination
	#[instrument(skip(self), level = "debug")]
	pub async fn write_to<P: AsRef<Path> + Debug, Q: AsRef<Path> + Debug>(
		&self,
		destination: P,
		cas_path: Q,
		link: bool,
	) -> std::io::Result<()> {
		match self {
			PackageFs::Cas(entries) => {
				package_fs_cas(
					entries.clone(),
					destination.as_ref(),
					cas_path.as_ref(),
					link,
				)
				.await
			}
			PackageFs::Copy(src, target) => {
				package_fs_copy(src, *target, destination.as_ref()).await
			}
		}
	}

	/// Returns the contents of the file with the given hash
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn read_file<P: AsRef<Path> + Debug, H: AsRef<str> + Debug>(
		&self,
		file_hash: H,
		cas_dir_path: P,
	) -> Option<String> {
		if !matches!(self, PackageFs::Cas(_)) {
			return None;
		}

		let cas_file_path = cas_path(file_hash.as_ref(), cas_dir_path.as_ref());
		fs::read_to_string(cas_file_path).await.ok()
	}
}
