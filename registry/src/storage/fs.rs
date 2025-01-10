use crate::{error::RegistryError, storage::StorageImpl};
use actix_web::{
	http::header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE},
	HttpResponse,
};
use fs_err::tokio as fs;
use pesde::{names::PackageName, source::ids::VersionId};
use std::{
	fmt::Display,
	path::{Path, PathBuf},
};
use tokio_util::io::ReaderStream;

#[derive(Debug)]
pub struct FSStorage {
	pub root: PathBuf,
}

async fn read_file_to_response(
	path: &Path,
	content_type: &str,
) -> Result<HttpResponse, RegistryError> {
	Ok(match fs::File::open(path).await {
		Ok(file) => HttpResponse::Ok()
			.append_header((CONTENT_TYPE, content_type))
			.append_header((CONTENT_ENCODING, "gzip"))
			.append_header((CONTENT_LENGTH, file.metadata().await?.len()))
			.streaming(ReaderStream::new(file)),
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
	) -> Result<(), RegistryError> {
		let (scope, name) = package_name.as_str();

		let path = self
			.root
			.join(scope)
			.join(name)
			.join(version.version().to_string())
			.join(version.target().to_string());
		fs::create_dir_all(&path).await?;

		fs::write(path.join("pkg.tar.gz"), &contents).await?;

		Ok(())
	}

	async fn get_package(
		&self,
		package_name: &PackageName,
		version: &VersionId,
	) -> Result<HttpResponse, RegistryError> {
		let (scope, name) = package_name.as_str();

		let path = self
			.root
			.join(scope)
			.join(name)
			.join(version.version().to_string())
			.join(version.target().to_string());

		read_file_to_response(&path.join("pkg.tar.gz"), "application/gzip").await
	}

	async fn store_readme(
		&self,
		package_name: &PackageName,
		version: &VersionId,
		contents: Vec<u8>,
	) -> Result<(), RegistryError> {
		let (scope, name) = package_name.as_str();

		let path = self
			.root
			.join(scope)
			.join(name)
			.join(version.version().to_string())
			.join(version.target().to_string());
		fs::create_dir_all(&path).await?;

		fs::write(path.join("readme.gz"), &contents).await?;

		Ok(())
	}

	async fn get_readme(
		&self,
		package_name: &PackageName,
		version: &VersionId,
	) -> Result<HttpResponse, RegistryError> {
		let (scope, name) = package_name.as_str();

		let path = self
			.root
			.join(scope)
			.join(name)
			.join(version.version().to_string())
			.join(version.target().to_string());

		read_file_to_response(&path.join("readme.gz"), "text/plain").await
	}

	async fn store_doc(&self, doc_hash: String, contents: Vec<u8>) -> Result<(), RegistryError> {
		let path = self.root.join("Doc");
		fs::create_dir_all(&path).await?;

		fs::write(path.join(format!("{doc_hash}.gz")), &contents).await?;

		Ok(())
	}

	async fn get_doc(&self, doc_hash: &str) -> Result<HttpResponse, RegistryError> {
		let path = self.root.join("Doc");

		read_file_to_response(&path.join(format!("{doc_hash}.gz")), "text/plain").await
	}
}

impl Display for FSStorage {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "FS ({})", self.root.display())
	}
}
