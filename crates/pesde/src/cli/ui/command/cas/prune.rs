use crate::cli::reporters::run_with_reporter;
use crate::cli::ui::CommandContext;
use anyhow::Context as _;
use clap::Args;
use futures::TryStreamExt as _;
use pesde::hash::Hash;

/// Removes unused files from the CAS
#[derive(Debug, Args)]
pub struct PruneCommand;

impl PruneCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let vfs = ctx.vfs().await?;

		let (removed_hash_count, removed_package_count) =
			run_with_reporter(async |_, root_progress, _| {
				let root_progress = root_progress;
				root_progress.reset();
				root_progress.set_message("remove unused files");
				let removed_hashes = vfs.remove_unused_hashes().await?;

				root_progress.reset();
				root_progress.set_message("delete unused packages");

				let mut removed = 0u64;

				let stream = vfs.list_cas_packages();
				let mut stream = std::pin::pin!(stream);
				while let Some((path, manifest)) = stream.try_next().await? {
					for hash_str in manifest.entries.values().flatten() {
						let hash = Hash::from_encoded(manifest.hash_algorithm, hash_str);
						let Ok(hash) = hash else {
							tracing::warn!("skipping invalid hash `{hash_str}`");
							continue;
						};

						if removed_hashes.contains(&hash) {
							tracing::debug!(
								"removing unused file `{}` due to hash `{hash}`",
								path.display()
							);
							removed += 1;
							root_progress.inc(1);
							tokio::fs::remove_file(&path)
								.await
								.context("failed to remove unused file")?;
							break;
						}
					}
				}

				anyhow::Ok((removed_hashes.len(), removed))
			})
			.await?;

		println!(
			"done! removed {removed_package_count} unused packages and {removed_hash_count} individual files!",
		);

		Ok(())
	}
}
