use actix_web::HttpResponse;
use actix_web::http::StatusCode;
use serde_json::json;

#[derive(Debug, Clone, Copy)]
pub enum Category {
	BadRequest,
	Unauthenticated,
	Unauthorized,
	NotFound,
	Conflict,
	Internal,
}

pub fn http_response(
	category: Category,
	error: &(impl std::error::Error + ?Sized),
) -> HttpResponse {
	let status = match category {
		Category::BadRequest => StatusCode::BAD_REQUEST,
		Category::Unauthenticated => StatusCode::UNAUTHORIZED,
		Category::Unauthorized => StatusCode::FORBIDDEN,
		Category::NotFound => StatusCode::NOT_FOUND,
		Category::Conflict => StatusCode::CONFLICT,
		Category::Internal => StatusCode::INTERNAL_SERVER_ERROR,
	};

	// Internal errors are logged and their detail withheld from the response.
	if matches!(category, Category::Internal) {
		tracing::error!("internal server error: {error:#?}");
		return HttpResponse::build(status).json(json!({ "error": "internal server error" }));
	}

	HttpResponse::build(status).json(json!({ "error": error.to_string() }))
}
