use std::collections::HashMap;

use actix_web::{web, HttpResponse};
use semver::Version;
use serde::Deserialize;
use tantivy::{collector::Count, query::AllQuery, schema::Value, DateTime, Order};

use crate::{error::RegistryError, package::PackageResponse, AppState};
use pesde::{
	names::PackageName,
	source::{
		git_index::{read_file, root_tree, GitBasedSource},
		pesde::IndexFile,
	},
};

#[derive(Deserialize)]
pub struct Request {
	#[serde(default)]
	query: Option<String>,
	#[serde(default)]
	offset: usize,
}

pub async fn search_packages(
	app_state: web::Data<AppState>,
	request_query: web::Query<Request>,
) -> Result<HttpResponse, RegistryError> {
	let searcher = app_state.search_reader.searcher();
	let schema = searcher.schema();

	let id = schema.get_field("id").unwrap();
	let version = schema.get_field("version").unwrap();

	let query = request_query.query.as_deref().unwrap_or_default().trim();

	let query = if query.is_empty() {
		Box::new(AllQuery)
	} else {
		app_state.query_parser.parse_query(query)?
	};

	let (count, top_docs) = searcher
		.search(
			&query,
			&(
				Count,
				tantivy::collector::TopDocs::with_limit(50)
					.and_offset(request_query.offset)
					.order_by_fast_field::<DateTime>("published_at", Order::Desc),
			),
		)
		.unwrap();

	let source = app_state.source.read().await;
	let repo = gix::open(source.path(&app_state.project))?;
	let tree = root_tree(&repo)?;

	let top_docs = top_docs
		.into_iter()
		.map(|(_, doc_address)| {
			let doc = searcher.doc::<HashMap<_, _>>(doc_address).unwrap();

			let id = doc
				.get(&id)
				.unwrap()
				.as_str()
				.unwrap()
				.parse::<PackageName>()
				.unwrap();
			let (scope, name) = id.as_str();
			let version = doc
				.get(&version)
				.unwrap()
				.as_str()
				.unwrap()
				.parse::<Version>()
				.unwrap();

			let file: IndexFile =
				toml::de::from_str(&read_file(&tree, [scope, name]).unwrap().unwrap()).unwrap();

			let version_id = file
				.entries
				.keys()
				.filter(|v_id| *v_id.version() == version)
				.max()
				.unwrap();

			PackageResponse::new(&id, version_id, &file)
		})
		.collect::<Vec<_>>();

	Ok(HttpResponse::Ok().json(serde_json::json!({
		"data": top_docs,
		"count": count,
	})))
}
