use crate::{
	auth::{get_token_from_req, AuthImpl, UserId},
	error::ReqwestErrorExt,
};
use actix_web::{dev::ServiceRequest, Error as ActixError};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(Debug)]
pub struct GitHubAuth {
	pub reqwest_client: reqwest::Client,
	pub client_id: String,
	pub client_secret: String,
}

#[derive(Debug, Serialize)]
struct TokenRequestBody {
	access_token: String,
}

impl AuthImpl for GitHubAuth {
	async fn for_write_request(&self, req: &ServiceRequest) -> Result<Option<UserId>, ActixError> {
		let token = match get_token_from_req(req) {
			Some(token) => token,
			None => return Ok(None),
		};

		let response = match self
			.reqwest_client
			.post(format!(
				"https://api.github.com/applications/{}/token",
				self.client_id
			))
			.basic_auth(&self.client_id, Some(&self.client_secret))
			.json(&TokenRequestBody {
				access_token: token,
			})
			.send()
			.await
		{
			Ok(response) => match response.error_for_status_ref() {
				Ok(_) => response,
				Err(e) if e.status().is_some_and(|s| s == StatusCode::NOT_FOUND) => {
					return Ok(None);
				}
				Err(_) => {
					tracing::error!(
						"failed to get user: {}",
						response.into_error().await.unwrap_err()
					);
					return Ok(None);
				}
			},
			Err(e) => {
				tracing::error!("failed to get user: {e}");
				return Ok(None);
			}
		};

		let user_id = match response.json::<UserResponse>().await {
			Ok(resp) => resp.user.id,
			Err(e) => {
				tracing::error!("failed to get user: {e}");
				return Ok(None);
			}
		};

		Ok(Some(UserId(user_id)))
	}
}

impl Display for GitHubAuth {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "GitHub")
	}
}

#[derive(Debug, Deserialize)]
struct User {
	id: u64,
}

#[derive(Debug, Deserialize)]
struct UserResponse {
	user: User,
}
