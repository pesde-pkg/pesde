mod get_search;

use actix_web::HttpResponse;
use actix_web::ResponseError;

use crate::shared::error::Category;
use crate::shared::error::http_response;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_search::http_v2);
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error(#[from] anyhow::Error);

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		http_response(Category::Internal, self)
	}
}
