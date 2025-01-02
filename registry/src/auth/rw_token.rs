use crate::auth::{get_token_from_req, AuthImpl, UserId};
use actix_web::{dev::ServiceRequest, Error as ActixError};
use constant_time_eq::constant_time_eq_32;
use sha2::{Digest, Sha256};
use std::fmt::Display;

#[derive(Debug)]
pub struct RwTokenAuth {
	pub read_token: [u8; 32],
	pub write_token: [u8; 32],
}

impl AuthImpl for RwTokenAuth {
	async fn for_write_request(&self, req: &ServiceRequest) -> Result<Option<UserId>, ActixError> {
		let token = match get_token_from_req(req) {
			Some(token) => token,
			None => return Ok(None),
		};

		let token: [u8; 32] = Sha256::digest(token.as_bytes()).into();

		Ok(if constant_time_eq_32(&self.write_token, &token) {
			Some(UserId::DEFAULT)
		} else {
			None
		})
	}

	async fn for_read_request(&self, req: &ServiceRequest) -> Result<Option<UserId>, ActixError> {
		let token = match get_token_from_req(req) {
			Some(token) => token,
			None => return Ok(None),
		};

		let token: [u8; 32] = Sha256::digest(token.as_bytes()).into();

		Ok(if constant_time_eq_32(&self.read_token, &token) {
			Some(UserId::DEFAULT)
		} else {
			None
		})
	}

	fn read_needs_auth(&self) -> bool {
		true
	}
}

impl Display for RwTokenAuth {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "RwToken")
	}
}
