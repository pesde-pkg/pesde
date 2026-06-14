use actix_web::HttpResponse;
use actix_web::ResponseError;
use actix_web::body::MessageBody;
use actix_web::dev::ServiceRequest;
use actix_web::dev::ServiceResponse;
use actix_web::http::header::AUTHORIZATION;
use actix_web::middleware::Next;
use actix_web::web;
use constant_time_eq::constant_time_eq_n;
use sha2::Digest as _;

use crate::AppState;
use crate::shared::error::Category;
use crate::shared::error::http_response;

#[derive(Debug, thiserror::Error)]
#[error("authentication required")]
pub struct Unauthenticated;

impl ResponseError for Unauthenticated {
	fn error_response(&self) -> HttpResponse {
		http_response(Category::Unauthenticated, self)
	}
}

pub type TokenHash = [u8; 64];

#[must_use]
pub fn hash_token(token: &str) -> TokenHash {
	let mut hasher = sha2::Sha512::default();
	hasher.update(token.as_bytes());
	hasher.finalize().into()
}

pub async fn authenticate(
	req: ServiceRequest,
	next: Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
	let state = req.app_data::<web::Data<AppState>>().unwrap();

	let required = if req.method().is_safe() {
		state.read_requires_auth
	} else {
		true
	};

	if required
		&& let Some(expected) = &state.access_token_hash
		&& req
			.headers()
			.get(AUTHORIZATION)
			.and_then(|t| t.to_str().ok())
			.is_none_or(|token| !constant_time_eq_n(&hash_token(token), expected))
	{
		return Err(Unauthenticated.into());
	}

	next.call(req).await
}
