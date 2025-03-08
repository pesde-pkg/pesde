use actix_web::{web, HttpResponse};

use crate::{
	error::RegistryError,
	package::read_package,
	request_path::{resolve_version_and_target, AnyOrSpecificTarget, LatestOrSpecificVersion},
	storage::StorageImpl as _,
	AppState,
};
use pesde::names::PackageName;

pub async fn get_package_readme(
	app_state: web::Data<AppState>,
	path: web::Path<(PackageName, LatestOrSpecificVersion, AnyOrSpecificTarget)>,
) -> Result<HttpResponse, RegistryError> {
	let (name, version, target) = path.into_inner();

	let Some(file) = read_package(&app_state, &name, &*app_state.source.read().await).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	let Some(v_id) = resolve_version_and_target(&file, version, &target) else {
		return Ok(HttpResponse::NotFound().finish());
	};

	app_state.storage.get_readme(&name, v_id).await
}
