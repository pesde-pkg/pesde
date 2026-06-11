use actix_web::HttpResponse;
use actix_web::ResponseError;

use crate::shared::db::IdentityWriteError;
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

	#[error("the public key has already been registered")]
	NonUniquePublicKey,

	#[error("the identity id has already been registered")]
	NonUniqueIdentityId,
}

impl From<IdentityWriteError> for Error {
	fn from(error: IdentityWriteError) -> Self {
		match error {
			IdentityWriteError::NonUniqueIdentityId => Error::NonUniqueIdentityId,
			IdentityWriteError::NonUniquePublicKey => Error::NonUniquePublicKey,
			IdentityWriteError::Internal(e) => Error::Internal(e),
		}
	}
}

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		let category = match self {
			Error::Internal(_) => Category::Internal,
			Error::InvalidSignature | Error::UnknownIdentity => Category::BadRequest,
			Error::NonUniquePublicKey | Error::NonUniqueIdentityId => Category::Conflict,
		};
		http_response(category, self)
	}
}
