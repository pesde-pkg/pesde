use actix_web::HttpResponse;
use actix_web::ResponseError;
use pesde_registry_core::db::StoreError;

use crate::shared::error::Category;
use crate::shared::error::http_response;

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
	#[error(transparent)]
	Internal(#[from] anyhow::Error),

	#[error("root not found")]
	RootNotFound,
}

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		let category = match self {
			Error::Internal(_) => Category::Internal,
			Error::RootNotFound => Category::BadRequest,
		};
		http_response(category, self)
	}
}

impl From<StoreError> for Error {
	fn from(value: StoreError) -> Self {
		Error::Internal(value.0)
	}
}
