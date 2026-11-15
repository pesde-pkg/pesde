use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use actix_web::web::Data;
use pesde::hash::Hash;
use pesde::hash::HashAlgorithm;

use super::error::Error;
use crate::AppState;
use crate::shared::blob::BlobResponse;
use crate::shared::blob::BlobStorage;

#[get("/blob/by-hash/{algorithm}/{hash}")]
pub(super) async fn http_v2(
	app_state: Data<AppState>,
	path: web::Path<(HashAlgorithm, Box<str>)>,
) -> Result<impl Responder, Error> {
	let hash = Hash::from_encoded(path.0, &path.1)?;

	let Some(response) = handler(&app_state.blob_storage, &hash).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(response.into())
}

async fn handler(blob: &BlobStorage, hash: &Hash) -> anyhow::Result<Option<BlobResponse>> {
	blob.get_object(hash).await
}
