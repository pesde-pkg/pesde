use crate::cli::reporters::run_with_reporter;
use crate::cli::style::INFO_STYLE;
use crate::cli::style::SUCCESS_STYLE;
use crate::util::remove_empty_dir;
use anyhow::Context as _;
use async_stream::try_stream;
use clap::Args;
use fs_err::tokio as fs;
use futures::FutureExt as _;
use futures::Stream;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use pesde::Subproject;
use pesde::hash::Hash;
use pesde::hash::HashAlgorithm;
use pesde::source::fs::FsEntry;
use pesde::source::fs::PackageFs;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;
use tokio::task::JoinSet;

#[derive(Debug, Args)]
pub struct PruneCommand;

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
		use std::os::unix::fs::MetadataExt as _;
		let metadata = fs::metadata(path).await?;
		return Ok(metadata.nlink());
	}
	// life if rust stabilized the nightly feature from 2019
	#[cfg(windows)]
	{
		use std::os::windows::ffi::OsStrExt as _;
		use windows::Win32::Foundation::CloseHandle;
		use windows::Win32::Storage::FileSystem::CreateFileW;
		use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
		use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
		use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
		use windows::Win32::Storage::FileSystem::GetFileInformationByHandle;
		use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
		use windows::core::PWSTR;

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

// CAS structure:
// /<hash algorithm>/<first part of hash>/<rest of hash>
// /index/pesde/hash/name/version/target
// /index/wally/hash/name/version
// /index/git/hash
// the deepest part of the non hash paths is a file containing the serialized PackageFs

async fn discover_cas_packages(cas_dir: &Path) -> anyhow::Result<HashMap<PathBuf, PackageFs>> {
	fn read_entry(
		path: PathBuf,
	) -> BoxFuture<'static, anyhow::Result<HashMap<PathBuf, PackageFs>>> {
		async move {
			match fs::read_to_string(&path).await {
				Ok(contents) => {
					let fs =
						toml::from_str(&contents).context("failed to deserialize PackageFs")?;

					return Ok(HashMap::from([(path, fs)]));
				}
				Err(e) if e.kind() == std::io::ErrorKind::IsADirectory => {}
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
				Err(e) => return Err(e).context("failed to read PackageFs file"),
			}

			let mut tasks = read_dir_stream(&path)
				.await
				.context("failed to read entry directory")?
				.map(|entry| async move {
					read_entry(
						entry
							.context("failed to read inner cas index dir entry")?
							.path(),
					)
					.await
				})
				.collect::<JoinSet<Result<_, anyhow::Error>>>()
				.await;

			let mut res = HashMap::new();
			while let Some(entry) = tasks.join_next().await {
				res.extend(entry.unwrap()?);
			}

			Ok(res)
		}
		.boxed()
	}

	let cas_entries = read_entry(cas_dir.join("index"))
		.await
		.context("failed to read index directory")?;

	Ok(cas_entries)
}

async fn remove_hashes(cas_dir: &Path) -> anyhow::Result<HashSet<Hash>> {
	let mut res = HashSet::new();

	let tasks = match read_dir_stream(cas_dir).await {
		Ok(tasks) => tasks,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(res),
		Err(e) => return Err(e).context("failed to read cas directory"),
	};

	let mut tasks = tasks
		.map(|algorithm_entry| async move {
			let algorithm_entry = algorithm_entry.context("failed to read cas dir entry")?;
			let algorithm = algorithm_entry.file_name();
			let algorithm = algorithm.to_str().context("non-UTF-8 cas algorithm name")?;
			if algorithm == "index" {
				return Ok(None);
			}

			let Ok(algorithm) = HashAlgorithm::from_str(algorithm) else {
				tracing::warn!("skipping unrecognized hash algorithm directory `{algorithm}`");
				return Ok(None);
			};

			let mut tasks = read_dir_stream(&algorithm_entry.path())
				.await
				.context("failed to read hash directory")?
				.map(|prefix_entry| async move {
					let prefix_entry = prefix_entry.context("failed to read prefix dir entry")?;
					let prefix = prefix_entry.file_name();
					let prefix: Arc<str> = prefix.to_str().context("non-UTF-8 hash prefix")?.into();

					let mut tasks = read_dir_stream(&prefix_entry.path())
						.await
						.context("failed to read prefix directory")?
						.map(|rest_entry| (rest_entry, prefix.clone()))
						.map(|(rest_entry, prefix)| async move {
							let rest_entry = rest_entry.context("failed to read rest dir entry")?;
							let rest = rest_entry.file_name();
							let rest = rest.to_str().context("non-UTF-8 hash rest")?;
							let path = rest_entry.path();

							let nlinks = get_nlinks(&path)
								.await
								.context("failed to count file usage")?;
							if nlinks > 1 {
								return Ok(None);
							}

							let hash = Hash::new(algorithm, format!("{prefix}{rest}"));

							fs::remove_file(&path)
								.await
								.context("failed to remove unused file")?;

							if let Some(parent) = path.parent() {
								remove_empty_dir(parent).await?;
							}

							Ok(Some(hash))
						})
						.collect::<JoinSet<Result<_, anyhow::Error>>>()
						.await;

					let mut removed_hashes = HashSet::new();
					while let Some(removed_hash) = tasks.join_next().await {
						let Some(hash) = removed_hash.unwrap()? else {
							continue;
						};

						removed_hashes.insert(hash);
					}

					Ok(removed_hashes)
				})
				.collect::<JoinSet<Result<_, anyhow::Error>>>()
				.await;

			let mut removed_hashes = HashSet::new();
			while let Some(hashes) = tasks.join_next().await {
				removed_hashes.extend(hashes.unwrap()?);
			}

			Ok(Some(removed_hashes))
		})
		.collect::<JoinSet<Result<_, anyhow::Error>>>()
		.await;

	while let Some(removed_hashes) = tasks.join_next().await {
		let Some(removed_hashes) = removed_hashes.unwrap()? else {
			continue;
		};

		res.extend(removed_hashes);
	}

	Ok(res)
}

impl PruneCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let (cas_entries, removed_hashes) = run_with_reporter(|_, root_progress, _| async {
			let root_progress = root_progress;
			root_progress.reset();
			root_progress.set_message("discover packages");
			let cas_entries = discover_cas_packages(subproject.project().cas_dir()).await?;
			root_progress.reset();
			root_progress.set_message("remove unused files");
			let removed_hashes = remove_hashes(subproject.project().cas_dir()).await?;

			Ok::<_, anyhow::Error>((cas_entries, removed_hashes))
		})
		.await?;

		let mut tasks = JoinSet::new();

		let mut removed_packages = 0usize;

		'entry: for (path, fs) in cas_entries {
			let PackageFs::Cached(entries) = fs else {
				continue;
			};

			for entry in entries.into_values() {
				let FsEntry::File(hash) = entry else {
					continue;
				};

				if removed_hashes.contains(&hash) {
					let cas_dir = subproject.project().cas_dir().to_path_buf();
					tasks.spawn(async move {
						fs::remove_file(&path)
							.await
							.context("failed to remove unused file")?;

						// remove empty directories up to the cas dir
						let mut path = &*path;
						while let Some(parent) = path.parent() {
							if parent == cas_dir {
								break;
							}

							remove_empty_dir(parent).await?;
							path = parent;
						}

						Ok::<_, anyhow::Error>(())
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
