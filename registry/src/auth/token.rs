use crate::auth::{get_token_from_req, AuthImpl, UserId};
use actix_web::{dev::ServiceRequest, Error as ActixError};
use constant_time_eq::constant_time_eq_32;
use sha2::{Digest as _, Sha256};
use std::fmt::Display;

#[derive(Debug)]
pub struct TokenAuth {
	// needs to be an SHA-256 hash
	pub token: [u8; 32],
}

impl AuthImpl for TokenAuth {
	async fn for_write_request(&self, req: &ServiceRequest) -> Result<Option<UserId>, ActixError> {
		let Some(token) = get_token_from_req(req) else {
			return Ok(None);
		};

		let token: [u8; 32] = Sha256::digest(token.as_bytes()).into();

		Ok(constant_time_eq_32(&self.token, &token).then_some(UserId::DEFAULT))
	}
}

impl Display for TokenAuth {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Token")
	}
}
