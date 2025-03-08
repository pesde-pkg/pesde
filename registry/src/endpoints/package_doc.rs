use crate::{
	error::RegistryError,
	package::read_package,
	request_path::{resolve_version_and_target, AnyOrSpecificTarget, LatestOrSpecificVersion},
	storage::StorageImpl as _,
	AppState,
};
use actix_web::{web, HttpResponse};
use pesde::{
	names::PackageName,
	source::{
		ids::VersionId,
		pesde::{DocEntryKind, IndexFile},
	},
};
use serde::Deserialize;

pub fn find_package_doc<'a>(
	file: &'a IndexFile,
	v_id: &VersionId,
	doc_name: &str,
) -> Option<&'a str> {
	let mut queue = file.entries[v_id]
		.docs
		.iter()
		.map(|doc| &doc.kind)
		.collect::<Vec<_>>();
	while let Some(doc) = queue.pop() {
		match doc {
			DocEntryKind::Page { name, hash } if name == doc_name => return Some(hash.as_str()),
			DocEntryKind::Category { items, .. } => {
				queue.extend(items.iter().map(|item| &item.kind));
			}
			DocEntryKind::Page { .. } => {}
		}
	}

	None
}

#[derive(Debug, Deserialize)]
pub struct Query {
	doc: String,
}

pub async fn get_package_doc(
	app_state: web::Data<AppState>,
	path: web::Path<(PackageName, LatestOrSpecificVersion, AnyOrSpecificTarget)>,
	request_query: web::Query<Query>,
) -> Result<HttpResponse, RegistryError> {
	let (name, version, target) = path.into_inner();

	let Some(file) = read_package(&app_state, &name, &*app_state.source.read().await).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	let Some(v_id) = resolve_version_and_target(&file, version, &target) else {
		return Ok(HttpResponse::NotFound().finish());
	};

	let Some(hash) = find_package_doc(&file, v_id, &request_query.doc) else {
		return Ok(HttpResponse::NotFound().finish());
	};

	app_state.storage.get_doc(hash).await
}
