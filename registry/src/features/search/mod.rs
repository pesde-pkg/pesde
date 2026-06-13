mod get_search;

use actix_web::HttpResponse;
use actix_web::ResponseError;
use async_trait::async_trait;
use futures::stream::BoxStream;
use pesde::names::PackageName;
use pesde::source::pesde::registry::SearchResultItem;

use crate::shared::error::Category;
use crate::shared::error::http_response;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_search::http_v2);
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error(#[from] anyhow::Error);

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		http_response(Category::Internal, self)
	}
}

#[derive(Debug)]
pub struct SearchPackage {
	pub id: u64,
	pub pos: u64,
	pub item: SearchResultItem,
}

#[async_trait]
pub trait Repository {
	async fn all_packages_for_index(&self) -> BoxStream<'_, anyhow::Result<SearchPackage>>;

	async fn searchable_version(&self, name: &PackageName) -> anyhow::Result<SearchPackage>;

	async fn search_result_by_pos(&self, pos: u64) -> anyhow::Result<SearchResultItem>;
}
