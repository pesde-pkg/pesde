mod get_search;

use actix_web::HttpResponse;
use actix_web::ResponseError;
use async_trait::async_trait;
use futures::stream::BoxStream;
use jiff::Timestamp;
use pesde::names::PackageName;
use semver::Version;

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
	pub data: PackageSearchData,
}

#[derive(Debug)]
pub struct PackageSearchData {
	pub name: PackageName,
	pub version: Version,
	pub published_at: Timestamp,
	pub description: String,
}

#[async_trait]
pub trait Repository {
	async fn all_packages_for_index(&self) -> BoxStream<'_, anyhow::Result<SearchPackage>>;

	async fn searchable_version(&self, name: &PackageName) -> anyhow::Result<SearchPackage>;

	async fn search_data_by_pos(&self, pos: u64) -> anyhow::Result<PackageSearchData>;
}
