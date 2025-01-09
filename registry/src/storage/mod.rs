use crate::{benv, error::RegistryError, make_reqwest};
use actix_web::HttpResponse;
use pesde::{names::PackageName, source::ids::VersionId};
use rusty_s3::{Bucket, Credentials, UrlStyle};
use std::fmt::Display;

mod fs;
mod s3;

#[derive(Debug)]
pub enum Storage {
	S3(s3::S3Storage),
	FS(fs::FSStorage),
}

pub trait StorageImpl: Display {
	async fn store_package(
		&self,
		package_name: &PackageName,
		version: &VersionId,
		contents: Vec<u8>,
	) -> Result<(), crate::error::RegistryError>;
	async fn get_package(
		&self,
		package_name: &PackageName,
		version: &VersionId,
	) -> Result<HttpResponse, crate::error::RegistryError>;

	async fn store_readme(
		&self,
		package_name: &PackageName,
		version: &VersionId,
		contents: Vec<u8>,
	) -> Result<(), crate::error::RegistryError>;
	async fn get_readme(
		&self,
		package_name: &PackageName,
		version: &VersionId,
	) -> Result<HttpResponse, crate::error::RegistryError>;

	async fn store_doc(
		&self,
		doc_hash: String,
		contents: Vec<u8>,
	) -> Result<(), crate::error::RegistryError>;
	async fn get_doc(&self, doc_hash: &str) -> Result<HttpResponse, crate::error::RegistryError>;
}

impl StorageImpl for Storage {
	async fn store_package(
		&self,
		package_name: &PackageName,
		version: &VersionId,
		contents: Vec<u8>,
	) -> Result<(), RegistryError> {
		match self {
			Storage::S3(s3) => s3.store_package(package_name, version, contents).await,
			Storage::FS(fs) => fs.store_package(package_name, version, contents).await,
		}
	}

	async fn get_package(
		&self,
		package_name: &PackageName,
		version: &VersionId,
	) -> Result<HttpResponse, RegistryError> {
		match self {
			Storage::S3(s3) => s3.get_package(package_name, version).await,
			Storage::FS(fs) => fs.get_package(package_name, version).await,
		}
	}

	async fn store_readme(
		&self,
		package_name: &PackageName,
		version: &VersionId,
		contents: Vec<u8>,
	) -> Result<(), RegistryError> {
		match self {
			Storage::S3(s3) => s3.store_readme(package_name, version, contents).await,
			Storage::FS(fs) => fs.store_readme(package_name, version, contents).await,
		}
	}

	async fn get_readme(
		&self,
		package_name: &PackageName,
		version: &VersionId,
	) -> Result<HttpResponse, RegistryError> {
		match self {
			Storage::S3(s3) => s3.get_readme(package_name, version).await,
			Storage::FS(fs) => fs.get_readme(package_name, version).await,
		}
	}

	async fn store_doc(&self, doc_hash: String, contents: Vec<u8>) -> Result<(), RegistryError> {
		match self {
			Storage::S3(s3) => s3.store_doc(doc_hash, contents).await,
			Storage::FS(fs) => fs.store_doc(doc_hash, contents).await,
		}
	}

	async fn get_doc(&self, doc_hash: &str) -> Result<HttpResponse, RegistryError> {
		match self {
			Storage::S3(s3) => s3.get_doc(doc_hash).await,
			Storage::FS(fs) => fs.get_doc(doc_hash).await,
		}
	}
}

impl Display for Storage {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Storage::S3(s3) => write!(f, "{}", s3),
			Storage::FS(fs) => write!(f, "{}", fs),
		}
	}
}

pub fn get_storage_from_env() -> Storage {
	if let Ok(endpoint) = benv!(parse "S3_ENDPOINT") {
		Storage::S3(s3::S3Storage {
			s3_bucket: Bucket::new(
				endpoint,
				UrlStyle::Path,
				benv!(required "S3_BUCKET_NAME"),
				benv!(required "S3_REGION"),
			)
			.unwrap(),
			s3_credentials: Credentials::new(
				benv!(required "S3_ACCESS_KEY"),
				benv!(required "S3_SECRET_KEY"),
			),
			reqwest_client: make_reqwest(),
		})
	} else if let Ok(root) = benv!(parse "FS_STORAGE_ROOT") {
		Storage::FS(fs::FSStorage { root })
	} else {
		panic!("no storage backend configured")
	}
}
