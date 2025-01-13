use crate::{
	error::RegistryError,
	package::{read_package, PackageResponse, PackageVersionsResponse},
	AppState,
};
use actix_web::{web, HttpResponse, Responder};
use pesde::{names::PackageName, source::ids::VersionId};
use semver::Version;
use std::collections::{btree_map::Entry, BTreeMap};

pub async fn get_package_versions_v0(
	app_state: web::Data<AppState>,
	path: web::Path<PackageName>,
) -> Result<impl Responder, RegistryError> {
	let name = path.into_inner();

	let Some(file) = read_package(&app_state, &name, &*app_state.source.read().await).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	let mut versions = BTreeMap::<&Version, &VersionId>::new();

	for v_id in file.entries.keys() {
		match versions.entry(v_id.version()) {
			Entry::Vacant(entry) => {
				entry.insert(v_id);
			}
			Entry::Occupied(mut entry) => {
				if entry.get() < &v_id {
					entry.insert(v_id);
				}
			}
		}
	}

	let responses = versions
		.into_values()
		.map(|v_id| PackageResponse::new(&name, v_id, &file))
		.collect::<Vec<_>>();

	Ok(HttpResponse::Ok().json(responses))
}

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
