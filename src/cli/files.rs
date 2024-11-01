use std::path::Path;

pub fn make_executable<P: AsRef<Path>>(_path: P) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use anyhow::Context;
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs_err::metadata(&_path)
            .context("failed to get bin link file metadata")?
            .permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs_err::set_permissions(&_path, perms)
            .context("failed to set bin link file permissions")?;
    }

    Ok(())
}
