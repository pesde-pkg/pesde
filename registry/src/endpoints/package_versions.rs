use std::collections::{BTreeMap, BTreeSet};

use actix_web::{web, HttpResponse, Responder};

use crate::{error::Error, package::PackageResponse, AppState};
use pesde::{
	names::PackageName,
	source::{
		git_index::{read_file, root_tree, GitBasedSource},
		pesde::IndexFile,
	},
};

pub async fn get_package_versions(
	app_state: web::Data<AppState>,
	path: web::Path<PackageName>,
) -> Result<impl Responder, Error> {
	let name = path.into_inner();

	let (scope, name_part) = name.as_str();

	let file: IndexFile = {
		let source = app_state.source.lock().await;
		let repo = gix::open(source.path(&app_state.project))?;
		let tree = root_tree(&repo)?;

		match read_file(&tree, [scope, name_part])? {
			Some(versions) => toml::de::from_str(&versions)?,
			None => return Ok(HttpResponse::NotFound().finish()),
		}
	};

	let mut responses = BTreeMap::new();

	for (v_id, entry) in file.entries {
		let info = responses
			.entry(v_id.version().clone())
			.or_insert_with(|| PackageResponse {
				name: name.to_string(),
				version: v_id.version().to_string(),
				targets: BTreeSet::new(),
				description: entry.description.unwrap_or_default(),
				published_at: entry.published_at,
				license: entry.license.unwrap_or_default(),
				authors: entry.authors.clone(),
				repository: entry.repository.clone().map(|url| url.to_string()),
			});

		info.targets.insert(entry.target.into());
		info.published_at = info.published_at.max(entry.published_at);
	}

	Ok(HttpResponse::Ok().json(responses.into_values().collect::<Vec<_>>()))
}
