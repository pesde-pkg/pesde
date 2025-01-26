use anyhow::Context;
use clap::Args;
use fs_err::tokio as fs;
use pesde::Project;
use std::{collections::HashSet, path::Path};
use tokio::task::JoinSet;

#[derive(Debug, Args)]
pub struct PruneCommand {}

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

async fn remove_hashes(cas_dir: &Path) -> anyhow::Result<HashSet<String>> {
	let mut tasks = JoinSet::new();

	let mut cas_entries = fs::read_dir(cas_dir)
		.await
		.context("failed to read directory")?;

	while let Some(cas_entry) = cas_entries
		.next_entry()
		.await
		.context("failed to read dir entry")?
	{
		let prefix = cas_entry.file_name();
		let Some(prefix) = prefix.to_str() else {
			continue;
		};
		// we only want hash directories
		if prefix.len() != 2 {
			continue;
		}

		let prefix = prefix.to_string();

		tasks.spawn(async move {
			let mut hash_entries = fs::read_dir(cas_entry.path())
				.await
				.context("failed to read hash directory")?;

			let mut tasks = JoinSet::new();

			while let Some(hash_entry) = hash_entries
				.next_entry()
				.await
				.context("failed to read hash dir entry")?
			{
				let hash = hash_entry.file_name();
				let hash = hash.to_str().expect("non-UTF-8 hash").to_string();
				let hash = format!("{prefix}{hash}");

				let path = hash_entry.path();
				tasks.spawn(async move {
					let nlinks = get_nlinks(&path)
						.await
						.context("failed to count file usage")?;
					if nlinks != 1 {
						return Ok::<_, anyhow::Error>(None);
					}

					fs::remove_file(path)
						.await
						.context("failed to remove unused file")?;

					Ok::<_, anyhow::Error>(Some(hash))
				});
			}

			let mut removed_hashes = HashSet::new();

			while let Some(removed_hash) = tasks.join_next().await {
				let Some(hash) = removed_hash.unwrap()? else {
					continue;
				};

				removed_hashes.insert(hash);
			}

			Ok::<_, anyhow::Error>(removed_hashes)
		});
	}

	let mut res = HashSet::new();

	while let Some(removed_hashes) = tasks.join_next().await {
		res.extend(removed_hashes.unwrap()?);
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
		let _ = remove_hashes(project.cas_dir()).await?;

		todo!("remove unused index entries");
	}
}
