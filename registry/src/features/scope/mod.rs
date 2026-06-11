mod error;
mod update_manifest;

use async_trait::async_trait;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;

use crate::shared::db::ManifestError;
use crate::shared::db::WriteStore;

pub use error::Error;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(update_manifest::http_v2);
}

#[async_trait]
pub trait Repository {
	async fn insert_manifest_update(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<ScopeManifestUpdateBody>,
	) -> Result<(), ManifestError>;
}
