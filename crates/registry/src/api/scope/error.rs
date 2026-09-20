use actix_web::HttpResponse;
use actix_web::ResponseError;

use crate::shared::error::Category;
use crate::shared::error::http_response;
use crate::shared::log::FromLogError;

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
	#[error(transparent)]
	Internal(#[from] anyhow::Error),

	#[error(transparent)]
	Merkleberg(#[from] merkleberg::Error),

	#[error("version publishing must go through its designated endpoint")]
	PublishVersionInPostEntry,

	#[error("scope doesn't exist")]
	ScopeNotFound,

	#[error("scope write not authorised")]
	Unauthorized,
}

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		let category = match self {
			Error::Internal(_) => Category::Internal,
			Error::Merkleberg(merkleberg::Error::GenProofForInvalidLeaves) => Category::BadRequest,
			Error::Merkleberg(_) => Category::Internal,
			Error::PublishVersionInPostEntry => Category::BadRequest,
			Error::ScopeNotFound => Category::NotFound,
			Error::Unauthorized => Category::Unauthorized,
		};
		http_response(category, self)
	}
}

impl FromLogError for Error {}
