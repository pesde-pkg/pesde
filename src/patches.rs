use crate::{
	MANIFEST_FILE_NAME,
	reporters::{PatchProgressReporter as _, PatchesReporter},
	source::ids::PackageId,
};
use fs_err::tokio as fs;
use futures::TryFutureExt as _;
use git2::{ApplyLocation, Diff, DiffFormat, DiffLineType, Repository, Signature};
use std::{
	path::{Path, PathBuf},
	sync::Arc,
};
use tokio::task::{JoinSet, spawn_blocking};
use tracing::instrument;

/// Set up a git repository for patches
pub fn setup_patches_repo<P: AsRef<Path>>(dir: P) -> Result<Repository, git2::Error> {
	let repo = Repository::init(&dir)?;
	repo.config()?.set_bool("core.autocrlf", false)?;

	{
		let signature = Signature::now(
			env!("CARGO_PKG_NAME"),
			concat!(env!("CARGO_PKG_NAME"), "@localhost"),
		)?;
		let mut index = repo.index()?;
		index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
		index.write()?;

		let oid = index.write_tree()?;
		let tree = repo.find_tree(oid)?;

		repo.commit(
			Some("HEAD"),
			&signature,
			&signature,
			"begin patch",
			&tree,
			&[],
		)?;
	};

	Ok(repo)
}

/// Create a patch from the current state of the repository
pub fn create_patch<P: AsRef<Path>>(dir: P) -> Result<Vec<u8>, git2::Error> {
	let mut patch = vec![];
	let repo = Repository::open(dir.as_ref())?;

	let original = repo.head()?.peel_to_tree()?;

	// reset the manifest file to the original state
	let mut checkout_builder = git2::build::CheckoutBuilder::new();
	checkout_builder.force();
	checkout_builder.path(MANIFEST_FILE_NAME);
	repo.checkout_tree(original.as_object(), Some(&mut checkout_builder))?;

	let mut diff_options = git2::DiffOptions::default();
	diff_options.include_untracked(true);
	diff_options.recurse_untracked_dirs(true);
	diff_options.show_untracked_content(true);

	let diff = repo.diff_tree_to_workdir(Some(&original), Some(&mut diff_options))?;

	diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
		if matches!(
			line.origin_value(),
			DiffLineType::Context | DiffLineType::Addition | DiffLineType::Deletion
		) {
			let origin = line.origin();
			let mut buffer = vec![0; origin.len_utf8()];
			origin.encode_utf8(&mut buffer);
			patch.extend(buffer);
		}

		patch.extend(line.content());

		true
	})?;

	Ok(patch)
}

// unlike a simple hard reset, this will also remove untracked files
fn reset_repo(repo: &Repository) -> Result<(), git2::Error> {
	let mut checkout_builder = git2::build::CheckoutBuilder::new();
	checkout_builder.force();
	checkout_builder.remove_untracked(true);
	repo.checkout_head(Some(&mut checkout_builder))?;

	Ok(())
}

/// Apply a patch to a dependency
#[instrument(skip(container_dir, patch_path, reporter), level = "debug")]
pub async fn apply_patch<Reporter>(
	package_id: &PackageId,
	container_dir: PathBuf,
	patch_path: &Path,
	reporter: Arc<Reporter>,
) -> Result<(), errors::ApplyPatchError>
where
	Reporter: PatchesReporter + Send + Sync + 'static,
{
	let dot_git = container_dir.join(".git");

	tracing::debug!("applying patch");

	let progress_reporter = reporter.report_patch(package_id.to_string());

	let patch = fs::read(&patch_path)
		.await
		.map_err(errors::ApplyPatchError::PatchRead)?;
	let patch = spawn_blocking(move || Diff::from_buffer(&patch))
		.await
		.unwrap()?;

	let mut apply_delta_tasks = patch
		.deltas()
		.filter(|delta| matches!(delta.status(), git2::Delta::Modified))
		.filter_map(|delta| delta.new_file().path())
		.map(|path| {
			let path = container_dir.join(path);

			async {
				// prevent CAS corruption by the file being modified
				let content = match fs::read(&path).await {
					Ok(content) => content,
					Err(e) if e.kind() == std::io::ErrorKind::IsADirectory => return Ok(()),
					Err(e) => return Err(e),
				};
				fs::remove_file(&path).await?;
				fs::write(path, content).await?;
				Ok(())
			}
			.map_err(errors::ApplyPatchError::File)
		})
		.collect::<JoinSet<_>>();

	while let Some(res) = apply_delta_tasks.join_next().await {
		res.unwrap()?;
	}

	spawn_blocking(move || {
		#[allow(clippy::disallowed_methods)]
		let repo = if dot_git.exists() {
			let repo = Repository::open(&container_dir)?;
			reset_repo(&repo)?;
			repo
		} else {
			setup_patches_repo(&container_dir)?
		};

		repo.apply(&patch, ApplyLocation::WorkDir, None)
	})
	.await
	.unwrap()?;

	tracing::debug!("patch applied");

	progress_reporter.report_done();

	Ok::<_, errors::ApplyPatchError>(())
}

/// Remove a patch from a dependency
#[instrument(level = "debug")]
pub async fn remove_patch(container_dir: PathBuf) -> Result<(), errors::ApplyPatchError> {
	let dot_git = container_dir.join(".git");

	tracing::debug!("removing patch");

	if fs::metadata(&dot_git).await.is_err() {
		return Ok(());
	}

	spawn_blocking(move || {
		let repo = Repository::open(&container_dir)?;
		reset_repo(&repo)?;

		Ok::<_, git2::Error>(())
	})
	.await
	.unwrap()?;

	match fs::remove_dir_all(&dot_git).await {
		Ok(()) => (),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
		Err(e) => return Err(errors::ApplyPatchError::File(e)),
	}

	tracing::debug!("patch removed");

	Ok::<_, errors::ApplyPatchError>(())
}

/// Errors that can occur when using patches
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when applying patches
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ApplyPatchError {
		/// Error interacting with git
		#[error("error interacting with git")]
		Git(#[from] git2::Error),

		/// Error reading the patch file
		#[error("error reading patch file")]
		PatchRead(#[source] std::io::Error),

		/// Error removing the .git directory
		#[error("error removing .git directory")]
		DotGitRemove(#[source] std::io::Error),

		/// Error interacting with a patched file
		#[error("error interacting with a patched file")]
		File(#[source] std::io::Error),
	}
}
