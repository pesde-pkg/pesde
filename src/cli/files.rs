use std::path::Path;

pub async fn make_executable<P: AsRef<Path>>(_path: P) -> anyhow::Result<()> {
	#[cfg(unix)]
	{
		use anyhow::Context as _;
		use fs_err::tokio as fs;
		use std::os::unix::fs::PermissionsExt as _;

		let mut perms = fs::metadata(&_path)
			.await
			.context("failed to get bin link file metadata")?
			.permissions();
		perms.set_mode(perms.mode() | 0o111);
		fs::set_permissions(&_path, perms)
			.await
			.context("failed to set bin link file permissions")?;
	}

	Ok(())
}
