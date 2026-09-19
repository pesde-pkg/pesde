use actix_web::HttpResponse;
use actix_web::ResponseError;

use crate::shared::error::Category;
use crate::shared::error::http_response;

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
	#[error(transparent)]
	Internal(#[from] anyhow::Error),

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

	#[error(transparent)]
	Multipart(#[from] actix_multipart::MultipartError),

	#[error("`{name}` field exceeds the maximum size of {limit} bytes")]
	FieldTooLarge { name: &'static str, limit: usize },
}

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		let category = match self {
			Error::Internal(_) => Category::Internal,
			Error::ArchiveHashMismatch | Error::Multipart(_) | Error::FieldTooLarge { .. } => {
				Category::BadRequest
			}
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
