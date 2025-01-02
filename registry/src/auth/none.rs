use crate::auth::{AuthImpl, UserId};
use actix_web::{dev::ServiceRequest, Error as ActixError};
use std::fmt::Display;

#[derive(Debug)]
pub struct NoneAuth;

impl AuthImpl for NoneAuth {
	async fn for_write_request(&self, _req: &ServiceRequest) -> Result<Option<UserId>, ActixError> {
		Ok(Some(UserId::DEFAULT))
	}
}

impl Display for NoneAuth {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "None")
	}
}
