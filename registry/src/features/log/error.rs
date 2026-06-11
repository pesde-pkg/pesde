use actix_web::HttpResponse;
use actix_web::ResponseError;

use crate::shared::error::Category;
use crate::shared::error::http_response;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error(transparent)]
	Internal(#[from] anyhow::Error),

	#[error(transparent)]
	Merkleberg(#[from] merkleberg::Error),

	#[error("the requested size {requested} exceeds the current log size {current}")]
	SizeOutOfRange { requested: u64, current: u64 },
}

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		let category = match self {
			Error::Internal(_) => Category::Internal,
			Error::Merkleberg(merkleberg::Error::GenProofForInvalidLeaves) => Category::NotFound,
			Error::Merkleberg(_) => Category::Internal,
			Error::SizeOutOfRange { .. } => Category::BadRequest,
		};
		http_response(category, self)
	}
}
