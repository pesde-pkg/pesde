use std::{
    collections::BTreeMap,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use crate::{
    manifest::target::TargetKind,
    source::{IGNORED_DIRS, IGNORED_FILES},
    util::hash,
};
use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

fn make_readonly(_file: &fs_err::File) -> std::io::Result<()> {
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

pub(crate) fn store_in_cas<P: AsRef<Path>>(
    cas_dir: P,
    contents: &[u8],
) -> std::io::Result<(String, PathBuf)> {
    let hash = hash(contents);
    let (prefix, rest) = hash.split_at(2);

    let folder = cas_dir.as_ref().join(prefix);
    fs_err::create_dir_all(&folder)?;

    let cas_path = folder.join(rest);
    if !cas_path.exists() {
        let mut file = fs_err::File::create(&cas_path)?;
        file.write_all(contents)?;

        make_readonly(&file)?;
    }

    Ok((hash, cas_path))
}

pub(crate) fn store_reader_in_cas<P: AsRef<Path>>(
    cas_dir: P,
    contents: &mut dyn Read,
) -> std::io::Result<String> {
    let tmp_dir = cas_dir.as_ref().join(".tmp");
    fs_err::create_dir_all(&tmp_dir)?;
    let mut hasher = Sha256::new();
    let mut buf = [0; 8 * 1024];
    let mut file_writer = BufWriter::new(tempfile::NamedTempFile::new_in(&tmp_dir)?);

    loop {
        let bytes_read = contents.read(&mut buf)?;
        if bytes_read == 0 {
            break;
        }

        let bytes = &buf[..bytes_read];
        hasher.update(bytes);
        file_writer.write_all(bytes)?;
    }

    let hash = format!("{:x}", hasher.finalize());
    let (prefix, rest) = hash.split_at(2);

    let folder = cas_dir.as_ref().join(prefix);
    fs_err::create_dir_all(&folder)?;

    let cas_path = folder.join(rest);
    match file_writer.into_inner()?.persist_noclobber(&cas_path) {
        Ok(_) => {
            make_readonly(&fs_err::File::open(cas_path)?)?;
        }
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.error),
    };

    Ok(hash)
}

fn copy_dir_all(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
    target: TargetKind,
) -> std::io::Result<()> {
    fs_err::create_dir_all(&dst)?;
    'outer: for entry in fs_err::read_dir(src.as_ref().to_path_buf())? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let file_name = entry.file_name().to_string_lossy().to_string();

        if ty.is_dir() {
            if IGNORED_DIRS.contains(&file_name.as_ref()) {
                continue;
            }

            for other_target in TargetKind::VARIANTS {
                if target.packages_folder(other_target) == file_name {
                    continue 'outer;
                }
            }

            copy_dir_all(entry.path(), dst.as_ref().join(&file_name), target)?;
        } else {
            if IGNORED_FILES.contains(&file_name.as_ref()) {
                continue;
            }

            fs_err::copy(entry.path(), dst.as_ref().join(file_name))?;
        }
    }
    Ok(())
}

impl PackageFS {
    /// Write the package to the given destination
    pub fn write_to<P: AsRef<Path>, Q: AsRef<Path>>(
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
                                fs_err::create_dir_all(parent)?;
                            }

                            let (prefix, rest) = hash.split_at(2);
                            let cas_file_path = cas_path.as_ref().join(prefix).join(rest);

                            if link {
                                fs_err::hard_link(cas_file_path, path)?;
                            } else {
                                let mut f = fs_err::File::create(&path)?;
                                f.write_all(&fs_err::read(cas_file_path)?)?;

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
                            fs_err::create_dir_all(path)?;
                        }
                    }
                }
            }
            PackageFS::Copy(src, target) => {
                copy_dir_all(src, destination, *target)?;
            }
        }

        Ok(())
    }

    /// Returns the contents of the file with the given hash
    pub fn read_file<P: AsRef<Path>, H: AsRef<str>>(
        &self,
        file_hash: H,
        cas_path: P,
    ) -> Option<String> {
        if !matches!(self, PackageFS::CAS(_)) {
            return None;
        }

        let (prefix, rest) = file_hash.as_ref().split_at(2);
        let cas_file_path = cas_path.as_ref().join(prefix).join(rest);
        fs_err::read_to_string(cas_file_path).ok()
    }
}
