mod error;
mod get_entry;
mod get_head;
mod get_inclusion;

use async_trait::async_trait;
use pesde::source::pesde::registry::Entry;
use pesde::source::pesde::registry::EntryPayload;

pub use error::Error;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_entry::http_v2)
		.service(get_head::http_v2)
		.service(get_inclusion::http_v2);
}

#[async_trait]
pub trait Repository {
	async fn entry(&self, pos: u64) -> anyhow::Result<Option<Entry<EntryPayload>>>;
}
