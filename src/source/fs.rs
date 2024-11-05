use crate::{
    manifest::target::TargetKind,
    source::{IGNORED_DIRS, IGNORED_FILES},
};
use fs_err::tokio as fs;
use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    path::{Path, PathBuf},
};
use tempfile::Builder;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    pin,
};

/// A file system entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FSEntry {
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
pub enum PackageFS {
    /// A package stored in the CAS
    CAS(BTreeMap<RelativePathBuf, FSEntry>),
    /// A package that's to be copied
    Copy(PathBuf, TargetKind),
}

fn make_readonly(_file: &fs::File) -> std::io::Result<()> {
    // on Windows, file deletion is disallowed if the file is read-only which breaks patching
    #[cfg(not(windows))]
    {
        let mut permissions = _file.metadata()?.permissions();
        permissions.set_readonly(true);
        _file.set_permissions(permissions)
    }

    #[cfg(windows)]
    Ok(())
}

pub(crate) fn cas_path(hash: &str, cas_dir: &Path) -> PathBuf {
    let (prefix, rest) = hash.split_at(2);
    cas_dir.join(prefix).join(rest)
}

pub(crate) async fn store_in_cas<
    R: tokio::io::AsyncRead + Unpin,
    P: AsRef<Path>,
    C: FnMut(Vec<u8>) -> F,
    F: Future<Output = std::io::Result<()>>,
>(
    cas_dir: P,
    mut contents: R,
    mut bytes_cb: C,
) -> std::io::Result<String> {
    let tmp_dir = cas_dir.as_ref().join(".tmp");
    fs::create_dir_all(&tmp_dir).await?;
    let mut hasher = Sha256::new();
    let mut buf = [0; 8 * 1024];

    let temp_path = Builder::new()
        .make_in(&tmp_dir, |_| Ok(()))?
        .into_temp_path();
    let mut file_writer = BufWriter::new(fs::File::create(temp_path.to_path_buf()).await?);

    loop {
        let bytes_future = contents.read(&mut buf);
        pin!(bytes_future);
        let bytes_read = bytes_future.await?;

        if bytes_read == 0 {
            break;
        }

        let bytes = &buf[..bytes_read];
        hasher.update(bytes);
        bytes_cb(bytes.to_vec()).await?;
        file_writer.write_all(bytes).await?;
    }

    let hash = format!("{:x}", hasher.finalize());

    let cas_path = cas_path(&hash, cas_dir.as_ref());
    fs::create_dir_all(cas_path.parent().unwrap()).await?;

    match temp_path.persist_noclobber(&cas_path) {
        Ok(_) => {
            make_readonly(&file_writer.into_inner())?;
        }
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.error),
    };

    Ok(hash)
}

impl PackageFS {
    /// Write the package to the given destination
    pub async fn write_to<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        destination: P,
        cas_path: Q,
        link: bool,
    ) -> std::io::Result<()> {
        match self {
            PackageFS::CAS(entries) => {
                for (path, entry) in entries {
                    let path = path.to_path(destination.as_ref());

                    match entry {
                        FSEntry::File(hash) => {
                            if let Some(parent) = path.parent() {
                                fs::create_dir_all(parent).await?;
                            }

                            let (prefix, rest) = hash.split_at(2);
                            let cas_file_path = cas_path.as_ref().join(prefix).join(rest);

                            if link {
                                fs::hard_link(cas_file_path, path).await?;
                            } else {
                                let mut f = fs::File::create(&path).await?;
                                f.write_all(&fs::read(cas_file_path).await?).await?;

                                #[cfg(unix)]
                                {
                                    let mut permissions = f.metadata()?.permissions();
                                    use std::os::unix::fs::PermissionsExt;
                                    permissions.set_mode(permissions.mode() | 0o644);
                                    f.set_permissions(permissions)?;
                                }
                            }
                        }
                        FSEntry::Directory => {
                            fs::create_dir_all(path).await?;
                        }
                    }
                }
            }
            PackageFS::Copy(src, target) => {
                fs::create_dir_all(destination.as_ref()).await?;

                let mut read_dirs = VecDeque::from([fs::read_dir(src.to_path_buf())]);
                while let Some(read_dir) = read_dirs.pop_front() {
                    let mut read_dir = read_dir.await?;
                    while let Some(entry) = read_dir.next_entry().await? {
                        let relative_path =
                            RelativePathBuf::from_path(entry.path().strip_prefix(src).unwrap())
                                .unwrap();
                        let file_name = relative_path.file_name().unwrap();

                        if entry.file_type().await?.is_dir() {
                            if IGNORED_DIRS.contains(&file_name) {
                                continue;
                            }

                            for other_target in TargetKind::VARIANTS {
                                if target.packages_folder(other_target) == file_name {
                                    continue;
                                }
                            }

                            read_dirs.push_back(fs::read_dir(entry.path()));
                            continue;
                        }

                        if IGNORED_FILES.contains(&file_name) {
                            continue;
                        }

                        fs::copy(entry.path(), relative_path.to_path(destination.as_ref())).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Returns the contents of the file with the given hash
    pub async fn read_file<P: AsRef<Path>, H: AsRef<str>>(
        &self,
        file_hash: H,
        cas_path: P,
    ) -> Option<String> {
        if !matches!(self, PackageFS::CAS(_)) {
            return None;
        }

        let (prefix, rest) = file_hash.as_ref().split_at(2);
        let cas_file_path = cas_path.as_ref().join(prefix).join(rest);
        fs::read_to_string(cas_file_path).await.ok()
    }
}
