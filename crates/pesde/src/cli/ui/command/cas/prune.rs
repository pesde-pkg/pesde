use crate::cli::reporters::run_with_reporter;
use crate::cli::ui::CommandContext;
use clap::Args;
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

				let stream = vfs.sqlite.list_cached_packages().await?;
				let mut stream = std::pin::pin!(stream);
				while let Some((package, manifest)) = stream.next().await? {
					let Some(manifest) = manifest else {
						tracing::debug!("package `{package}` has no CAS manifest, skipping");
						continue;
					};

					for hash_bytes in manifest.entries.values().flatten() {
						let hash = Hash::new(manifest.hash_algorithm, hash_bytes);
						let Ok(hash) = hash else {
							tracing::warn!("skipping invalid hash `{hash_bytes:?}`");
							continue;
						};

						if removed_hashes.contains(&hash) {
							tracing::debug!(
								"removing unused package `{package}` due to hash `{hash}`"
							);
							removed += 1;
							root_progress.inc(1);
							stream.delete_current_item().await?;
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
