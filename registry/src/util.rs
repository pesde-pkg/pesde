use actix_web::ResponseError;
use actix_web::http::StatusCode;
use fs_err::tokio as fs;
use serde_json::json;
use std::env::VarError;
use std::fmt::Display;
use std::str::FromStr;
use thiserror::Error;

pub struct Env {
	name: &'static str,
}

impl Env {
	pub fn new(name: &'static str) -> Self {
		Self { name }
	}

	pub async fn try_get(&self) -> Option<String> {
		match std::env::var(self.name) {
			Ok(result) => return Some(result),
			Err(VarError::NotPresent) => {}
			Err(e) => panic!("error reading `{}`: {e}", self.name),
		}

		let file_path = match std::env::var(format!("{}_FILE", self.name)) {
			Ok(result) => result,
			Err(VarError::NotPresent) => return None,
			Err(e) => panic!("error reading `{}_FILE`: {e}", self.name),
		};

		match fs::read_to_string(file_path).await {
			Ok(result) => Some(result.trim().to_string()),
			Err(e) => panic!("error reading `{}_FILE`: {e}", self.name),
		}
	}

	pub async fn try_parse<T>(&self) -> Option<T>
	where
		T: FromStr,
		<T as FromStr>::Err: Display,
	{
		match self.try_get().await.map(|result| result.parse()) {
			Some(Ok(result)) => Some(result),
			Some(Err(e)) => panic!("error parsing `{}`: {e}", self.name),
			None => None,
		}
	}

	pub async fn get(&self) -> String {
		match self.try_get().await {
			Some(result) => result,
			None => panic!(
				"{name} or {name}_FILE is required, but is not set",
				name = self.name
			),
		}
	}

	pub async fn parse<T>(&self) -> T
	where
		T: FromStr,
		<T as FromStr>::Err: Display,
	{
		match self.get().await.parse() {
			Ok(result) => result,
			Err(e) => panic!("error parsing `{}`: {e}", self.name),
		}
	}
}

#[derive(Debug, Error)]
#[error(transparent)]
pub struct AnyhowError(#[from] anyhow::Error);

#[derive(Debug, Error)]
pub enum AppError {
	#[error(transparent)]
	Internal(#[from] anyhow::Error),

	#[error(transparent)]
	InternalWrapper(#[from] AnyhowError),

	#[error(transparent)]
	Merkleberg(#[from] merkleberg::Error),
}

impl ResponseError for AppError {
	fn error_response(&self) -> actix_web::HttpResponse<actix_web::body::BoxBody> {
		let (status_code, body) = match self {
			AppError::Internal(e) | AppError::InternalWrapper(AnyhowError(e)) => {
				tracing::error!("internal server error: {e}");
				(
					StatusCode::INTERNAL_SERVER_ERROR,
					json!({ "error": "internal server error" }),
				)
			}
			AppError::Merkleberg(merkleberg::Error::GenProofForInvalidLeaves) => {
				(StatusCode::NOT_FOUND, json!({ "error": "leaf not known" }))
			}
			AppError::Merkleberg(e) => {
				tracing::error!("internal server error: {e}");
				(
					StatusCode::INTERNAL_SERVER_ERROR,
					json!({ "error": "internal server error" }),
				)
			}
		};

		actix_web::HttpResponse::build(status_code).json(body)
	}
}

pub type AppResult<T> = Result<T, AppError>;
pub type ControllerResult = AppResult<actix_web::HttpResponse>;
