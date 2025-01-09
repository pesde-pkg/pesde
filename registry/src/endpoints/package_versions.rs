use actix_web::{web, HttpResponse, Responder};

use crate::{
	error::RegistryError,
	package::{read_package, PackageVersionsResponse},
	AppState,
};
use pesde::names::PackageName;

pub async fn get_package_versions(
	app_state: web::Data<AppState>,
	path: web::Path<PackageName>,
) -> Result<impl Responder, RegistryError> {
	let name = path.into_inner();

	let Some(file) = read_package(&app_state, &name, &*app_state.source.read().await).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(PackageVersionsResponse::new(&name, &file)))
}
