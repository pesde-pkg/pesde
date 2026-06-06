use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::IdentityId;

#[get("/identity/{identity_id}")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	identity_id: web::Path<IdentityId>,
) -> HttpResult {
	handler(&app_state.database, &identity_id).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &Database, identity_id: &IdentityId) -> AppResult<()> {
	query(db, identity_id).await?;
	Ok(())
}

async fn query(db: &Database, _identity_id: &IdentityId) -> anyhow::Result<()> {
	todo!()
}
