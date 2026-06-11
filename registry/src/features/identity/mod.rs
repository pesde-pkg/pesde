mod error;
mod get_identity;
mod register_identity;
mod rotate_key;

use async_trait::async_trait;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;

use crate::shared::db::IdentityWriteError;
use crate::shared::db::WriteStore;

pub use error::Error;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_identity::http_v2)
		.service(register_identity::http_v2)
		.service(rotate_key::http_v2);
}

#[async_trait]
pub trait Repository {
	async fn identity_entry(&self, id: &IdentityId)
	-> anyhow::Result<Option<Entry<IdentityEntry>>>;

	async fn insert_register(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &RegisterIdentityBody,
	) -> Result<(), IdentityWriteError>;

	async fn insert_rotation(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		old_sig: &Signature,
		new_sig: &Signature,
		body: &IdentityRotationBody,
	) -> Result<(), IdentityWriteError>;
}
