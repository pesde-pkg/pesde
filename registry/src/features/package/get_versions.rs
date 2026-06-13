use std::num::NonZero;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use pesde::source::pesde::registry::PackageVersionsResponse;
use serde::Deserialize;

use crate::AppState;
use crate::features::package::Error;
use crate::shared::auth::ReadGuard;
use crate::shared::db::Backend;

#[derive(Debug, Deserialize)]
struct VersionsQuery {
	#[serde(default)]
	after: u64,
	limit: Option<NonZero<u8>>,
}

#[get("/package/{scope}/{name}/versions")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name)>,
	query: web::Query<VersionsQuery>,
) -> Result<impl Responder, Error> {
	let (scope, name) = path.into_inner();
	let package_name = PackageName::new(scope, name);
	let limit = query
		.limit
		.unwrap_or(NonZero::new(50).unwrap())
		.min(NonZero::new(20).unwrap());

	let response = handler(app_state.db.as_ref(), &package_name, query.after, limit).await?;
	if response.total == 0 {
		return Ok(HttpResponse::NotFound().finish());
	}

	Ok(HttpResponse::Ok().json(response))
}

async fn handler(
	db: &dyn Backend,
	name: &PackageName,
	after: u64,
	limit: NonZero<u8>,
) -> anyhow::Result<PackageVersionsResponse> {
	db.package_versions(name, after, limit).await
}
