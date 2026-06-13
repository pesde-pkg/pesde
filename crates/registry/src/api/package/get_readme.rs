use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use semver::Version;

use crate::AppState;
use crate::api::package::Error;
use crate::shared::auth::ReadGuard;
use crate::shared::blob::BlobResponse;
use crate::shared::blob::BlobStorage;

#[get("/package/{scope}/{name}/{version}/readme")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name, Version)>,
) -> Result<impl Responder, Error> {
	let (scope, name, version) = path.into_inner();
	let package_name = PackageName::new(scope, name);

	let Some(response) = handler(&app_state.blob_storage, &package_name, &version).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(response.into())
}

async fn handler(
	blob: &BlobStorage,
	name: &PackageName,
	version: &Version,
) -> anyhow::Result<Option<BlobResponse>> {
	blob.get_package_readme(name, version).await
}
