use actix_web::HttpResponse;
use actix_web::ResponseError;
use pesde_registry_core::features::package::PackageWriteError;
use pesde_registry_core::features::scope::ManifestError;

use crate::shared::error::Category;
use crate::shared::error::http_response;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error(transparent)]
	Internal(#[from] anyhow::Error),

	#[error("signature verification failed")]
	InvalidSignature,

	#[error("the identity is not registered")]
	UnknownIdentity,

	#[error("not authorized to perform this action in the scope")]
	Unauthorized,

	#[error("the package version does not exist")]
	UnknownPackageVersion,

	#[error("the package version has already been published")]
	VersionAlreadyExists,

	#[error("the package version is already yanked")]
	AlreadyYanked,

	#[error("the package version is not yanked")]
	NotYanked,

	#[error("the package is already deprecated")]
	AlreadyDeprecated,

	#[error("the package is not deprecated")]
	NotDeprecated,

	#[error("the archive hash does not match the uploaded data")]
	ArchiveHashMismatch,

	#[error("{0}")]
	BadRequest(String),
}

impl From<PackageWriteError> for Error {
	fn from(error: PackageWriteError) -> Self {
		match error {
			PackageWriteError::VersionAlreadyExists => Error::VersionAlreadyExists,
			PackageWriteError::UnknownPackageVersion => Error::UnknownPackageVersion,
			PackageWriteError::AlreadyYanked => Error::AlreadyYanked,
			PackageWriteError::NotYanked => Error::NotYanked,
			PackageWriteError::AlreadyDeprecated => Error::AlreadyDeprecated,
			PackageWriteError::NotDeprecated => Error::NotDeprecated,
			PackageWriteError::Internal(e) => Error::Internal(e),
		}
	}
}

impl From<ManifestError> for Error {
	fn from(error: ManifestError) -> Self {
		match error {
			ManifestError::Internal(e) => Error::Internal(e),
			e @ ManifestError::UnregisteredIdentity(_) => Error::BadRequest(e.to_string()),
		}
	}
}

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		let category = match self {
			Error::Internal(_) => Category::Internal,
			Error::InvalidSignature
			| Error::ArchiveHashMismatch
			| Error::UnknownIdentity
			| Error::BadRequest(_) => Category::BadRequest,
			Error::Unauthorized => Category::Unauthorized,
			Error::UnknownPackageVersion => Category::NotFound,
			Error::VersionAlreadyExists
			| Error::AlreadyYanked
			| Error::NotYanked
			| Error::AlreadyDeprecated
			| Error::NotDeprecated => Category::Conflict,
		};
		http_response(category, self)
	}
}
