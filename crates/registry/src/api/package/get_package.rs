use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use pesde::source::pesde::registry::PackageInfoResponse;
use pesde_registry_core::db::Backend;

use crate::AppState;
use crate::api::package::Error;
use crate::shared::auth::ReadGuard;

#[get("/package/{scope}/{name}")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name)>,
) -> Result<impl Responder, Error> {
	let (scope, name) = path.into_inner();
	let package_name = PackageName::new(scope, name);

	let Some(info) = handler(app_state.db.as_ref(), &package_name).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(info))
}

async fn handler(
	db: &dyn Backend,
	name: &PackageName,
) -> anyhow::Result<Option<PackageInfoResponse>> {
	db.package_info(name).await
}
