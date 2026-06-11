use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use pesde::source::pesde::registry::Entry;
use pesde::source::pesde::registry::PublishScopeEntry;
use semver::Version;

use crate::AppState;
use crate::features::package::Error;
use crate::shared::auth::ReadGuard;
use crate::shared::db::Backend;

#[get("/package/{scope}/{name}/{version}")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name, Version)>,
) -> Result<impl Responder, Error> {
	let (scope, name, version) = path.into_inner();
	let package_name = PackageName::new(scope, name);

	let Some(entry) = handler(app_state.db.as_ref(), &package_name, &version).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(
	db: &dyn Backend,
	name: &PackageName,
	version: &Version,
) -> anyhow::Result<Option<Entry<PublishScopeEntry>>> {
	db.package_version(name, version).await
}
