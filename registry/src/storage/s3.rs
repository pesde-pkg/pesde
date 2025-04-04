use crate::{
	error::{RegistryError, ReqwestErrorExt as _},
	storage::StorageImpl,
};
use actix_web::{http::header::LOCATION, HttpResponse};
use pesde::{names::PackageName, source::ids::VersionId};
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE};
use rusty_s3::{
	actions::{GetObject, PutObject},
	Bucket, Credentials, S3Action as _,
};
use std::{fmt::Display, time::Duration};

#[derive(Debug)]
pub struct S3Storage {
	pub bucket: Bucket,
	pub credentials: Credentials,
	pub reqwest_client: reqwest::Client,
}

pub const S3_SIGN_DURATION: Duration = Duration::from_secs(60 * 15);

impl StorageImpl for S3Storage {
	async fn store_package(
		&self,
		package_name: &PackageName,
		version: &VersionId,
		contents: Vec<u8>,
	) -> Result<(), RegistryError> {
		let object_url = PutObject::new(
			&self.bucket,
			Some(&self.credentials),
			&format!(
				"{package_name}/{}/{}/pkg.tar.gz",
				version.version(),
				version.target()
			),
		)
		.sign(S3_SIGN_DURATION);

		self.reqwest_client
			.put(object_url)
			.header(CONTENT_TYPE, "application/gzip")
			.header(CONTENT_ENCODING, "gzip")
			.body(contents)
			.send()
			.await?
			.into_error()
			.await?;

		Ok(())
	}

	async fn get_package(
		&self,
		package_name: &PackageName,
		version: &VersionId,
	) -> Result<HttpResponse, RegistryError> {
		let object_url = GetObject::new(
			&self.bucket,
			Some(&self.credentials),
			&format!(
				"{package_name}/{}/{}/pkg.tar.gz",
				version.version(),
				version.target()
			),
		)
		.sign(S3_SIGN_DURATION);

		Ok(HttpResponse::TemporaryRedirect()
			.append_header((LOCATION, object_url.as_str()))
			.finish())
	}

	async fn store_readme(
		&self,
		package_name: &PackageName,
		version: &VersionId,
		contents: Vec<u8>,
	) -> Result<(), RegistryError> {
		let object_url = PutObject::new(
			&self.bucket,
			Some(&self.credentials),
			&format!(
				"{package_name}/{}/{}/readme.gz",
				version.version(),
				version.target()
			),
		)
		.sign(S3_SIGN_DURATION);

		self.reqwest_client
			.put(object_url)
			.header(CONTENT_TYPE, "text/plain")
			.header(CONTENT_ENCODING, "gzip")
			.body(contents)
			.send()
			.await?
			.into_error()
			.await?;

		Ok(())
	}

	async fn get_readme(
		&self,
		package_name: &PackageName,
		version: &VersionId,
	) -> Result<HttpResponse, RegistryError> {
		let object_url = GetObject::new(
			&self.bucket,
			Some(&self.credentials),
			&format!(
				"{package_name}/{}/{}/readme.gz",
				version.version(),
				version.target()
			),
		)
		.sign(S3_SIGN_DURATION);

		Ok(HttpResponse::TemporaryRedirect()
			.append_header((LOCATION, object_url.as_str()))
			.finish())
	}

	async fn store_doc(&self, doc_hash: String, contents: Vec<u8>) -> Result<(), RegistryError> {
		let object_url = PutObject::new(
			&self.bucket,
			Some(&self.credentials),
			// capitalize Doc to prevent conflicts with scope names
			&format!("Doc/{doc_hash}.gz"),
		)
		.sign(S3_SIGN_DURATION);

		self.reqwest_client
			.put(object_url)
			.header(CONTENT_TYPE, "text/plain")
			.header(CONTENT_ENCODING, "gzip")
			.body(contents)
			.send()
			.await?
			.into_error()
			.await?;

		Ok(())
	}

	async fn get_doc(&self, doc_hash: &str) -> Result<HttpResponse, RegistryError> {
		let object_url = GetObject::new(
			&self.bucket,
			Some(&self.credentials),
			&format!("Doc/{doc_hash}.gz"),
		)
		.sign(S3_SIGN_DURATION);

		Ok(HttpResponse::TemporaryRedirect()
			.append_header((LOCATION, object_url.as_str()))
			.finish())
	}
}

impl Display for S3Storage {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "S3 (bucket name: {})", self.bucket.name())
	}
}
