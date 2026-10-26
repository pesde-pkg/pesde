use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::hash::Hash;

use crate::AppState;
use crate::api::package::error::Error;
use crate::shared::blob::BlobResponse;
use crate::shared::blob::BlobStorage;

#[get("/package-archive/{hash}")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	path: web::Path<Hash>,
) -> Result<impl Responder, Error> {
	let Some(response) = handler(&app_state.blob_storage, &path).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(response.into())
}

async fn handler(blob: &BlobStorage, hash: &Hash) -> anyhow::Result<Option<BlobResponse>> {
	blob.get_package_archive(hash).await
}
