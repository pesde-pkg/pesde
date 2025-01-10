use crate::{
	error::RegistryError,
	package::{read_package, PackageResponse},
	search::find_max_searchable,
	AppState,
};
use actix_web::{web, HttpResponse};
use pesde::names::PackageName;
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tantivy::{collector::Count, query::AllQuery, schema::Value, DateTime, Order};
use tokio::task::JoinSet;

#[derive(Deserialize)]
pub struct Request {
	#[serde(default)]
	query: String,
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

	let query = request_query.query.trim();

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

	let source = Arc::new(app_state.source.clone().read_owned().await);

	let mut results = Vec::with_capacity(top_docs.len());
	results.extend((0..top_docs.len()).map(|_| None::<PackageResponse>));

	let mut tasks = top_docs
		.into_iter()
		.enumerate()
		.map(|(i, (_, doc_address))| {
			let app_state = app_state.clone();
			let doc = searcher.doc::<HashMap<_, _>>(doc_address).unwrap();
			let source = source.clone();

			async move {
				let id = doc
					.get(&id)
					.unwrap()
					.as_str()
					.unwrap()
					.parse::<PackageName>()
					.unwrap();

				let file = read_package(&app_state, &id, &source).await?.unwrap();

				let (version_id, _) = find_max_searchable(&file).unwrap();

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
