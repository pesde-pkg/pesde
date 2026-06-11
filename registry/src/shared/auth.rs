use std::future::Ready;
use std::future::ready;

use actix_web::FromRequest;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use actix_web::ResponseError;
use actix_web::dev::Payload;
use actix_web::http::header::AUTHORIZATION;
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

fn authenticate(req: &HttpRequest, required: bool) -> Result<(), Unauthenticated> {
	let state = req.app_data::<web::Data<AppState>>().unwrap();

	let Some(expected) = &state.access_token_hash else {
		return Ok(());
	};

	if !required {
		return Ok(());
	}

	match req
		.headers()
		.get(AUTHORIZATION)
		.and_then(|t| t.to_str().ok())
	{
		Some(token) if constant_time_eq_n(&hash_token(token), expected) => Ok(()),
		_ => Err(Unauthenticated),
	}
}

pub struct ReadGuard;

impl FromRequest for ReadGuard {
	type Error = Unauthenticated;
	type Future = Ready<Result<Self, Unauthenticated>>;

	fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
		let required = req
			.app_data::<web::Data<AppState>>()
			.unwrap()
			.read_requires_auth;

		ready(authenticate(req, required).map(|_| ReadGuard))
	}
}

pub struct WriteGuard;

impl FromRequest for WriteGuard {
	type Error = Unauthenticated;
	type Future = Ready<Result<Self, Unauthenticated>>;

	fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
		ready(authenticate(req, true).map(|_| WriteGuard))
	}
}
