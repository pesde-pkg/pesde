use std::collections::HashMap;

use crate::{
	error::RegistryError,
	package::{read_package, PackageResponse},
	AppState,
};
use actix_web::{web, HttpResponse};
use pesde::names::PackageName;
use semver::Version;
use serde::Deserialize;
use tantivy::{collector::Count, query::AllQuery, schema::Value, DateTime, Order};
use tokio::task::JoinSet;

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

	// prevent a write lock on the source while we're reading the documents
	let _guard = app_state.source.read().await;

	let mut results = Vec::with_capacity(top_docs.len());
	results.extend((0..top_docs.len()).map(|_| None::<PackageResponse>));

	let mut tasks = top_docs
		.into_iter()
		.enumerate()
		.map(|(i, (_, doc_address))| {
			let app_state = app_state.clone();
			let doc = searcher.doc::<HashMap<_, _>>(doc_address).unwrap();

			async move {
				let id = doc
					.get(&id)
					.unwrap()
					.as_str()
					.unwrap()
					.parse::<PackageName>()
					.unwrap();
				let version = doc
					.get(&version)
					.unwrap()
					.as_str()
					.unwrap()
					.parse::<Version>()
					.unwrap();

				let file = read_package(&app_state, &id, &*app_state.source.read().await)
					.await?
					.unwrap();

				let version_id = file
					.entries
					.keys()
					.filter(|v_id| *v_id.version() == version)
					.max()
					.unwrap();

				Ok::<_, RegistryError>((i, PackageResponse::new(&id, version_id, &file)))
			}
		})
		.collect::<JoinSet<_>>();

	while let Some(res) = tasks.join_next().await {
		let (i, res) = res.unwrap()?;
		results[i] = Some(res);
	}

	Ok(HttpResponse::Ok().json(serde_json::json!({
		"data": results,
		"count": count,
	})))
}
