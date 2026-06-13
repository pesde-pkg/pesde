use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::Entry;
use pesde::source::pesde::registry::IdentityEntry;
use pesde::source::pesde::registry::IdentityId;
use pesde_registry_core::db::Backend;

use crate::AppState;
use crate::api::identity::error::Error;
use crate::shared::auth::ReadGuard;

#[get("/identity/{identity_id}")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	identity_id: web::Path<IdentityId>,
) -> Result<impl Responder, Error> {
	let Some(entry) = handler(app_state.db.as_ref(), &identity_id).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(
	db: &dyn Backend,
	id: &IdentityId,
) -> anyhow::Result<Option<Entry<IdentityEntry>>> {
	db.identity_entry(id).await
}
