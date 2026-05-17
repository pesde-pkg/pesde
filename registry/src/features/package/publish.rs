use crate::AppState;
use crate::shared::blob::BlobStorage;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;

#[post("/v2/package/publish")]
pub async fn http(app_state: web::Data<AppState>) -> ControllerResult {
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
