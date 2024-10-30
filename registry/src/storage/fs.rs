use crate::{error::Error, storage::StorageImpl};
use actix_web::{
    http::header::{CONTENT_ENCODING, CONTENT_TYPE},
    HttpResponse,
};
use pesde::{names::PackageName, source::version_id::VersionId};
use std::{fmt::Display, fs::create_dir_all, path::PathBuf};
use std::path::Path;

#[derive(Debug)]
pub struct FSStorage {
    pub root: PathBuf,
}

fn read_file_to_response(path: &Path, content_type: &str) -> Result<HttpResponse, Error> {
    Ok(match std::fs::read(path) {
        Ok(contents) => HttpResponse::Ok()
            .append_header((CONTENT_TYPE, content_type))
            .append_header((CONTENT_ENCODING, "gzip"))
            .body(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HttpResponse::NotFound().finish(),
        Err(e) => return Err(e.into()),
    })
}

impl StorageImpl for FSStorage {
    async fn store_package(
        &self,
        package_name: &PackageName,
        version: &VersionId,
        contents: Vec<u8>,
    ) -> Result<(), Error> {
        let (scope, name) = package_name.as_str();

        let path = self
            .root
            .join(scope)
            .join(name)
            .join(version.version().to_string())
            .join(version.target().to_string());
        create_dir_all(&path)?;

        std::fs::write(path.join("pkg.tar.gz"), &contents)?;

        Ok(())
    }

    async fn get_package(
        &self,
        package_name: &PackageName,
        version: &VersionId,
    ) -> Result<HttpResponse, Error> {
        let (scope, name) = package_name.as_str();

        let path = self
            .root
            .join(scope)
            .join(name)
            .join(version.version().to_string())
            .join(version.target().to_string());
        
        read_file_to_response(&path.join("pkg.tar.gz"), "application/gzip")
    }

    async fn store_readme(
        &self,
        package_name: &PackageName,
        version: &VersionId,
        contents: Vec<u8>,
    ) -> Result<(), Error> {
        let (scope, name) = package_name.as_str();

        let path = self
            .root
            .join(scope)
            .join(name)
            .join(version.version().to_string())
            .join(version.target().to_string());
        create_dir_all(&path)?;

        std::fs::write(path.join("readme.gz"), &contents)?;

        Ok(())
    }

    async fn get_readme(
        &self,
        package_name: &PackageName,
        version: &VersionId,
    ) -> Result<HttpResponse, Error> {
        let (scope, name) = package_name.as_str();

        let path = self
            .root
            .join(scope)
            .join(name)
            .join(version.version().to_string())
            .join(version.target().to_string());

        read_file_to_response(&path.join("readme.gz"), "text/plain")
    }

    async fn store_doc(&self, doc_hash: String, contents: Vec<u8>) -> Result<(), Error> {
        let path = self.root.join("Doc");
        create_dir_all(&path)?;

        std::fs::write(path.join(format!("{doc_hash}.gz")), &contents)?;

        Ok(())
    }

    async fn get_doc(&self, doc_hash: &str) -> Result<HttpResponse, Error> {
        let path = self.root.join("Doc");

        read_file_to_response(&path.join(format!("{doc_hash}.gz")), "text/plain")
    }
}

impl Display for FSStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FS")
    }
}
