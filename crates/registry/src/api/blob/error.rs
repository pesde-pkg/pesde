use actix_web::HttpResponse;
use actix_web::ResponseError;

use crate::shared::error::Category;
use crate::shared::error::http_response;

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
	#[error(transparent)]
	Internal(#[from] anyhow::Error),

	#[error("the provided hash wasn't valid")]
	InvalidHash(#[from] pesde::hash::NewHashError),
}

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		let category = match self {
			Error::Internal(_) => Category::Internal,
			Error::InvalidHash(_) => Category::BadRequest,
		};
		http_response(category, self)
	}
}
