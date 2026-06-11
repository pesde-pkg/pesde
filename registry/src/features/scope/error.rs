use actix_web::HttpResponse;
use actix_web::ResponseError;

use crate::shared::db::ManifestError;
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

	#[error("not authorized to manage this scope")]
	Unauthorized,

	#[error("{0}")]
	BadRequest(String),
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
			Error::InvalidSignature | Error::UnknownIdentity | Error::BadRequest(_) => {
				Category::BadRequest
			}
			Error::Unauthorized => Category::Unauthorized,
		};
		http_response(category, self)
	}
}
