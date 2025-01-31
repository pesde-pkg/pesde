use crate::cli::{
	reporters::run_with_reporter,
	style::{INFO_STYLE, SUCCESS_STYLE},
};
use anyhow::Context;
use async_stream::try_stream;
use clap::Args;
use fs_err::tokio as fs;
use futures::{future::BoxFuture, FutureExt, Stream, StreamExt};
use pesde::{
	source::fs::{FsEntry, PackageFs},
	Project,
};
use std::{
	collections::{HashMap, HashSet},
	future::Future,
	path::{Path, PathBuf},
};
use tokio::task::JoinSet;

#[derive(Debug, Args)]
pub struct PruneCommand {}

async fn read_dir_stream(
	dir: &Path,
) -> std::io::Result<impl Stream<Item = std::io::Result<fs::DirEntry>>> {
	let mut read_dir = fs::read_dir(dir).await?;

	Ok(try_stream! {
		while let Some(entry) = read_dir.next_entry().await? {
			yield entry;
		}
	})
}

#[allow(unreachable_code)]
async fn get_nlinks(path: &Path) -> anyhow::Result<u64> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::MetadataExt;
		let metadata = fs::metadata(path).await?;
		return Ok(metadata.nlink());
	}
	// life if rust stabilized the nightly feature from 2019
	#[cfg(windows)]
	{
		use std::os::windows::ffi::OsStrExt;
		use windows::{
			core::PWSTR,
			Win32::{
				Foundation::CloseHandle,
				Storage::FileSystem::{
					CreateFileW, GetFileInformationByHandle, FILE_ATTRIBUTE_NORMAL,
					FILE_GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING,
				},
			},
		};

		let path = path.to_path_buf();
		return tokio::task::spawn_blocking(move || unsafe {
			let handle = CreateFileW(
				PWSTR(
					path.as_os_str()
						.encode_wide()
						.chain(std::iter::once(0))
						.collect::<Vec<_>>()
						.as_mut_ptr(),
				),
				FILE_GENERIC_READ.0,
				FILE_SHARE_READ,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None,
			)?;

			let mut info =
				windows::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION::default();
			let res = GetFileInformationByHandle(handle, &mut info);
			CloseHandle(handle)?;
			res?;

			Ok(info.nNumberOfLinks as u64)
		})
		.await
		.unwrap();
	}
	#[cfg(not(any(unix, windows)))]
	{
		compile_error!("unsupported platform");
	}
	anyhow::bail!("unsupported platform")
}

#[derive(Debug)]
struct ExtendJoinSet<T: Send + 'static>(JoinSet<T>);

impl<T: Send + 'static, F: Future<Output = T> + Send + 'static> Extend<F> for ExtendJoinSet<T> {
	fn extend<I: IntoIterator<Item = F>>(&mut self, iter: I) {
		for item in iter {
			self.0.spawn(item);
		}
	}
}

impl<T: Send + 'static> Default for ExtendJoinSet<T> {
	fn default() -> Self {
		Self(JoinSet::new())
	}
}

async fn discover_cas_packages(cas_dir: &Path) -> anyhow::Result<HashMap<PathBuf, PackageFs>> {
	fn read_entry(
		entry: fs::DirEntry,
	) -> BoxFuture<'static, anyhow::Result<HashMap<PathBuf, PackageFs>>> {
		async move {
			if entry
				.metadata()
				.await
				.context("failed to read entry metadata")?
				.is_dir()
			{
				let mut tasks = read_dir_stream(&entry.path())
					.await
					.context("failed to read entry directory")?
					.map(|entry| async move {
						read_entry(entry.context("failed to read inner cas index dir entry")?).await
					})
					.collect::<ExtendJoinSet<Result<_, anyhow::Error>>>()
					.await
					.0;

				let mut res = HashMap::new();
				while let Some(entry) = tasks.join_next().await {
					res.extend(entry.unwrap()?);
				}

				return Ok(res);
			};

			let contents = fs::read_to_string(entry.path()).await?;
			let fs = toml::from_str(&contents).context("failed to deserialize PackageFs")?;

			Ok(HashMap::from([(entry.path(), fs)]))
		}
		.boxed()
	}

	let mut tasks = ["index", "wally_index", "git_index"]
		.into_iter()
		.map(|index| cas_dir.join(index))
		.map(|index| async move {
			let mut tasks = read_dir_stream(&index)
				.await
				.context("failed to read index directory")?
				.map(|entry| async move {
					read_entry(entry.context("failed to read cas index dir entry")?).await
				})
				.collect::<ExtendJoinSet<Result<_, anyhow::Error>>>()
				.await
				.0;

			let mut res = HashMap::new();

			while let Some(task) = tasks.join_next().await {
				res.extend(task.unwrap()?);
			}

			Ok(res)
		})
		.collect::<JoinSet<Result<_, anyhow::Error>>>();

	let mut cas_entries = HashMap::new();

	while let Some(task) = tasks.join_next().await {
		cas_entries.extend(task.unwrap()?);
	}

	Ok(cas_entries)
}

async fn remove_hashes(cas_dir: &Path) -> anyhow::Result<HashSet<String>> {
	let mut tasks = read_dir_stream(cas_dir)
		.await?
		.map(|cas_entry| async move {
			let cas_entry = cas_entry.context("failed to read cas dir entry")?;
			let prefix = cas_entry.file_name();
			let Some(prefix) = prefix.to_str() else {
				return Ok(None);
			};
			// we only want hash directories
			if prefix.len() != 2 {
				return Ok(None);
			}

			let mut tasks = read_dir_stream(&cas_entry.path())
				.await
				.context("failed to read hash directory")?
				.map(|hash_entry| {
					let prefix = prefix.to_string();
					async move {
						let hash_entry = hash_entry.context("failed to read hash dir entry")?;
						let hash = hash_entry.file_name();
						let hash = hash.to_str().expect("non-UTF-8 hash").to_string();
						let hash = format!("{prefix}{hash}");

						let path = hash_entry.path();
						let nlinks = get_nlinks(&path)
							.await
							.context("failed to count file usage")?;
						if nlinks > 1 {
							return Ok(None);
						}

						fs::remove_file(path)
							.await
							.context("failed to remove unused file")?;

						Ok(Some(hash))
					}
				})
				.collect::<ExtendJoinSet<Result<_, anyhow::Error>>>()
				.await
				.0;

			let mut removed_hashes = HashSet::new();
			while let Some(removed_hash) = tasks.join_next().await {
				let Some(hash) = removed_hash.unwrap()? else {
					continue;
				};

				removed_hashes.insert(hash);
			}

			Ok(Some(removed_hashes))
		})
		.collect::<ExtendJoinSet<Result<_, anyhow::Error>>>()
		.await
		.0;

	let mut res = HashSet::new();

	while let Some(removed_hashes) = tasks.join_next().await {
		let Some(removed_hashes) = removed_hashes.unwrap()? else {
			continue;
		};

		res.extend(removed_hashes);
	}

	Ok(res)
}

impl PruneCommand {
	pub async fn run(self, project: Project) -> anyhow::Result<()> {
		// CAS structure:
		// /2 first chars of hash/rest of hash
		// /index/hash/name/version/target
		// /wally_index/hash/name/version
		// /git_index/hash/hash
		// the last thing in the path is the serialized PackageFs

		let (cas_entries, removed_hashes) = run_with_reporter(|_, root_progress, _| async {
			let root_progress = root_progress;
			root_progress.reset();
			root_progress.set_message("discover packages");
			let cas_entries = discover_cas_packages(project.cas_dir()).await?;
			root_progress.reset();
			root_progress.set_message("remove unused files");
			let removed_hashes = remove_hashes(project.cas_dir()).await?;

			Ok::<_, anyhow::Error>((cas_entries, removed_hashes))
		})
		.await?;

		let mut tasks = JoinSet::new();

		let mut removed_packages = 0usize;

		'entry: for (path, fs) in cas_entries {
			let PackageFs::CAS(entries) = fs else {
				continue;
			};

			for entry in entries.into_values() {
				let FsEntry::File(hash) = entry else {
					continue;
				};

				if removed_hashes.contains(&hash) {
					tasks.spawn(async move {
						fs::remove_file(path)
							.await
							.context("failed to remove unused file")
					});
					removed_packages += 1;
					// if at least one file is removed, the package is not used
					continue 'entry;
				}
			}
		}

		while let Some(task) = tasks.join_next().await {
			task.unwrap()?;
		}

		println!(
			"{} removed {} unused packages and {} individual files!",
			SUCCESS_STYLE.apply_to("done!"),
			INFO_STYLE.apply_to(removed_packages),
			INFO_STYLE.apply_to(removed_hashes.len())
		);

		Ok(())
	}
}
