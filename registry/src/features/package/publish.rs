use crate::AppState;
use crate::shared::blob::BlobStorage;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;

#[post("/package/publish")]
pub(super) async fn http_v2(app_state: web::Data<AppState>) -> HttpResult {
	handler(&app_state.database, &app_state.blob_storage).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &Database, blob: &BlobStorage) -> AppResult<()> {
	query(db, blob).await?;
	Ok(())
}

async fn query(db: &Database, _blob: &BlobStorage) -> anyhow::Result<()> {
	todo!()
}
