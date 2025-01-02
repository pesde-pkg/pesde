mod github;
mod none;
mod rw_token;
mod token;

use crate::{benv, make_reqwest, AppState};
use actix_governor::{KeyExtractor, SimpleKeyExtractionError};
use actix_web::{
	body::MessageBody,
	dev::{ServiceRequest, ServiceResponse},
	error::Error as ActixError,
	http::header::AUTHORIZATION,
	middleware::Next,
	web, HttpMessage, HttpResponse,
};
use pesde::source::pesde::IndexConfig;
use sentry::add_breadcrumb;
use sha2::{Digest, Sha256};
use std::fmt::Display;

#[derive(Debug, Copy, Clone, Hash, PartialOrd, PartialEq, Eq, Ord)]
pub struct UserId(pub u64);

impl UserId {
	// there isn't any account on GitHub that has the ID 0, so it should be safe to use it
	pub const DEFAULT: UserId = UserId(0);
}

#[derive(Debug, Clone)]
pub struct UserIdExtractor;

impl KeyExtractor for UserIdExtractor {
	type Key = UserId;
	type KeyExtractionError = SimpleKeyExtractionError<&'static str>;

	fn extract(&self, req: &ServiceRequest) -> Result<Self::Key, Self::KeyExtractionError> {
		match req.extensions().get::<UserId>() {
			Some(user_id) => Ok(*user_id),
			None => Err(SimpleKeyExtractionError::new("UserId not found")),
		}
	}
}

#[derive(Debug)]
pub enum Auth {
	GitHub(github::GitHubAuth),
	None(none::NoneAuth),
	Token(token::TokenAuth),
	RwToken(rw_token::RwTokenAuth),
}

pub trait AuthImpl: Display {
	async fn for_write_request(&self, req: &ServiceRequest) -> Result<Option<UserId>, ActixError>;

	async fn for_read_request(&self, req: &ServiceRequest) -> Result<Option<UserId>, ActixError> {
		self.for_write_request(req).await
	}

	fn read_needs_auth(&self) -> bool {
		benv!("READ_NEEDS_AUTH").is_ok()
	}
}

impl AuthImpl for Auth {
	async fn for_write_request(&self, req: &ServiceRequest) -> Result<Option<UserId>, ActixError> {
		match self {
			Auth::GitHub(github) => github.for_write_request(req).await,
			Auth::None(none) => none.for_write_request(req).await,
			Auth::Token(token) => token.for_write_request(req).await,
			Auth::RwToken(rw_token) => rw_token.for_write_request(req).await,
		}
	}

	async fn for_read_request(&self, req: &ServiceRequest) -> Result<Option<UserId>, ActixError> {
		match self {
			Auth::GitHub(github) => github.for_read_request(req).await,
			Auth::None(none) => none.for_write_request(req).await,
			Auth::Token(token) => token.for_write_request(req).await,
			Auth::RwToken(rw_token) => rw_token.for_read_request(req).await,
		}
	}

	fn read_needs_auth(&self) -> bool {
		match self {
			Auth::GitHub(github) => github.read_needs_auth(),
			Auth::None(none) => none.read_needs_auth(),
			Auth::Token(token) => token.read_needs_auth(),
			Auth::RwToken(rw_token) => rw_token.read_needs_auth(),
		}
	}
}

impl Display for Auth {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Auth::GitHub(github) => write!(f, "{}", github),
			Auth::None(none) => write!(f, "{}", none),
			Auth::Token(token) => write!(f, "{}", token),
			Auth::RwToken(rw_token) => write!(f, "{}", rw_token),
		}
	}
}

pub async fn write_mw(
	app_state: web::Data<AppState>,
	req: ServiceRequest,
	next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, ActixError> {
	let user_id = match app_state.auth.for_write_request(&req).await? {
		Some(user_id) => user_id,
		None => {
			return Ok(req
				.into_response(HttpResponse::Unauthorized().finish())
				.map_into_right_body())
		}
	};

	add_breadcrumb(sentry::Breadcrumb {
		category: Some("auth".into()),
		message: Some(format!("write request authorized as {}", user_id.0)),
		level: sentry::Level::Info,
		..Default::default()
	});

	req.extensions_mut().insert(user_id);

	next.call(req).await.map(|res| res.map_into_left_body())
}

pub async fn read_mw(
	app_state: web::Data<AppState>,
	req: ServiceRequest,
	next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, ActixError> {
	if app_state.auth.read_needs_auth() {
		let user_id = match app_state.auth.for_read_request(&req).await? {
			Some(user_id) => user_id,
			None => {
				return Ok(req
					.into_response(HttpResponse::Unauthorized().finish())
					.map_into_right_body())
			}
		};

		add_breadcrumb(sentry::Breadcrumb {
			category: Some("auth".into()),
			message: Some(format!("read request authorized as {}", user_id.0)),
			level: sentry::Level::Info,
			..Default::default()
		});

		req.extensions_mut().insert(Some(user_id));
	} else {
		req.extensions_mut().insert(None::<UserId>);
	}

	next.call(req).await.map(|res| res.map_into_left_body())
}

pub fn get_auth_from_env(config: &IndexConfig) -> Auth {
	if let Ok(token) = benv!("ACCESS_TOKEN") {
		Auth::Token(token::TokenAuth {
			token: *Sha256::digest(token.as_bytes()).as_ref(),
		})
	} else if let Ok(client_secret) = benv!("GITHUB_CLIENT_SECRET") {
		Auth::GitHub(github::GitHubAuth {
			reqwest_client: make_reqwest(),
			client_id: config
				.github_oauth_client_id
				.clone()
				.expect("index isn't configured for GitHub"),
			client_secret,
		})
	} else if let Ok((r, w)) =
		benv!("READ_ACCESS_TOKEN").and_then(|r| benv!("WRITE_ACCESS_TOKEN").map(|w| (r, w)))
	{
		Auth::RwToken(rw_token::RwTokenAuth {
			read_token: *Sha256::digest(r.as_bytes()).as_ref(),
			write_token: *Sha256::digest(w.as_bytes()).as_ref(),
		})
	} else {
		Auth::None(none::NoneAuth)
	}
}

pub fn get_token_from_req(req: &ServiceRequest) -> Option<String> {
	let token = req
		.headers()
		.get(AUTHORIZATION)
		.and_then(|token| token.to_str().ok())?;

	let token = if token.to_lowercase().starts_with("bearer ") {
		token[7..].to_string()
	} else {
		token.to_string()
	};

	Some(token)
}
