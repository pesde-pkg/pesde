use std::collections::HashMap;

use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use tantivy::{collector::Count, query::AllQuery, schema::Value, DateTime, Order};

use crate::{error::Error, package::PackageResponse, AppState};
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
	offset: Option<usize>,
}

pub async fn search_packages(
	app_state: web::Data<AppState>,
	request: web::Query<Request>,
) -> Result<impl Responder, Error> {
	let searcher = app_state.search_reader.searcher();
	let schema = searcher.schema();

	let id = schema.get_field("id").unwrap();

	let query = request.query.as_deref().unwrap_or_default().trim();

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
					.and_offset(request.offset.unwrap_or_default())
					.order_by_fast_field::<DateTime>("published_at", Order::Desc),
			),
		)
		.unwrap();

	let source = app_state.source.lock().await;
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

			let file: IndexFile =
				toml::de::from_str(&read_file(&tree, [scope, name]).unwrap().unwrap()).unwrap();

			let (latest_version, entry) = file
				.entries
				.iter()
				.max_by_key(|(v_id, _)| v_id.version())
				.unwrap();

			PackageResponse {
				name: id.to_string(),
				version: latest_version.version().to_string(),
				targets: file
					.entries
					.iter()
					.filter(|(v_id, _)| v_id.version() == latest_version.version())
					.map(|(_, entry)| (&entry.target).into())
					.collect(),
				description: entry.description.clone().unwrap_or_default(),
				published_at: file
					.entries
					.values()
					.map(|entry| entry.published_at)
					.max()
					.unwrap(),
				license: entry.license.clone().unwrap_or_default(),
				authors: entry.authors.clone(),
				repository: entry.repository.clone().map(|url| url.to_string()),
			}
		})
		.collect::<Vec<_>>();

	Ok(HttpResponse::Ok().json(serde_json::json!({
		"data": top_docs,
		"count": count,
	})))
}
