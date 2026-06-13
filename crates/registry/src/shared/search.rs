use std::collections::HashMap;
use std::sync::Arc;

use actix_web::web;
use futures::Stream;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::lock::Mutex;
use futures::stream;
use itertools::Itertools as _;
use pesde::names::PackageName;
use pesde::source::pesde::registry::SearchResultItem;
use pesde_registry_core::db::Backend;
use pesde_registry_core::features::search::SearchPackage;
use serde::Serialize;
use tantivy::DateTime;
use tantivy::Index;
use tantivy::IndexReader;
use tantivy::IndexWriter;
use tantivy::Order;
use tantivy::Term;
use tantivy::collector::Count;
use tantivy::collector::TopDocs;
use tantivy::doc;
use tantivy::query::AllQuery;
use tantivy::query::QueryParser;
use tantivy::schema::FAST;
use tantivy::schema::Field;
use tantivy::schema::IndexRecordOption;
use tantivy::schema::STORED;
use tantivy::schema::Schema;
use tantivy::schema::TextFieldIndexing;
use tantivy::schema::TextOptions;
use tantivy::schema::Value as _;
use tantivy::tokenizer::TextAnalyzer;

const WRITER_HEAP_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct Fields {
	id: Field,
	pos: Field,
	scope: Field,
	name: Field,
	description: Field,
	published_at: Field,
}

pub struct Search {
	index: Index,
	reader: IndexReader,
	writer: Arc<Mutex<IndexWriter>>,
	fields: Fields,
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
	count: usize,
	results: Vec<SearchResultItem>,
}

impl Search {
	pub async fn new(
		packages: impl Stream<Item = anyhow::Result<SearchPackage>>,
	) -> anyhow::Result<Self> {
		let mut schema = Schema::builder();
		let field_options = TextOptions::default().set_indexing_options(
			TextFieldIndexing::default()
				.set_tokenizer("ngram")
				.set_index_option(IndexRecordOption::WithFreqsAndPositions),
		);

		let fields = Fields {
			id: schema.add_u64_field("id", STORED),
			pos: schema.add_u64_field("pos", STORED),
			scope: schema.add_text_field("scope", field_options.clone()),
			name: schema.add_text_field("name", field_options.clone()),
			description: schema.add_text_field("description", field_options),
			published_at: schema.add_date_field("published_at", FAST),
		};

		let schema = schema.build();

		let index = Index::create_in_ram(schema);
		index.tokenizers().register(
			"ngram",
			TextAnalyzer::builder(tantivy::tokenizer::NgramTokenizer::all_ngrams(2, 12).unwrap())
				.filter(tantivy::tokenizer::LowerCaser)
				.build(),
		);

		let mut writer = index.writer(WRITER_HEAP_BYTES)?;

		tokio::pin!(packages);
		while let Some(pkg) = packages.try_next().await? {
			writer.add_document(doc!(
				fields.id => pkg.id,
				fields.pos => pkg.pos,
				fields.scope => pkg.item.name.scope().as_str(),
				fields.name => pkg.item.name.name().as_str(),
				fields.description => pkg.item.description,
				fields.published_at => DateTime::from_timestamp_secs(pkg.item.published_at.as_second()),
			))?;
		}
		writer.commit()?;

		let reader = index
			.reader_builder()
			.reload_policy(tantivy::ReloadPolicy::Manual)
			.try_into()?;

		Ok(Self {
			index,
			reader,
			writer: Arc::new(Mutex::new(writer)),
			fields,
		})
	}

	pub async fn update(&self, backend: &dyn Backend, name: PackageName) -> anyhow::Result<()> {
		let package = backend.searchable_version(&name).await?;

		let fields = self.fields;
		let mut writer = self.writer.clone().lock_owned().await;
		let reader = self.reader.clone();
		web::block(move || {
			writer.delete_term(Term::from_field_u64(fields.id, package.id));
			writer.add_document(doc!(
				fields.id => package.id,
				fields.pos => package.pos,
				fields.scope => name.scope().as_str(),
				fields.name => name.name().as_str(),
				fields.description => package.item.description,
				fields.published_at => DateTime::from_timestamp_secs(package.item.published_at.as_second()),
			))?;
			writer.commit()?;
			drop(writer);

			reader.reload()?;

			Ok::<_, anyhow::Error>(())
		})
		.await??;

		Ok(())
	}

	pub async fn query(
		&self,
		backend: &dyn Backend,
		query: String,
		limit: usize,
		offset: usize,
	) -> anyhow::Result<QueryResult> {
		let searcher = self.reader.searcher();
		let fields = self.fields;

		let mut query_parser = QueryParser::for_index(
			&self.index,
			vec![fields.scope, fields.name, fields.description],
		);
		query_parser.set_field_boost(fields.scope, 2.0);
		query_parser.set_field_boost(fields.name, 3.5);

		let (count, top_docs) = web::block(move || {
			let (count, top_docs) = if query.is_empty() {
				let (count, top_docs) = searcher.search(
					&AllQuery,
					&(
						Count,
						TopDocs::with_limit(limit)
							.and_offset(offset)
							.order_by_fast_field::<DateTime>("published_at", Order::Desc),
					),
				)?;

				let top_docs = top_docs
					.into_iter()
					.map(|(_, addr)| addr)
					.collect::<Vec<_>>();

				(count, top_docs)
			} else {
				let (count, top_docs) = searcher.search(
					&query_parser.parse_query_lenient(&query).0,
					&(
						Count,
						TopDocs::with_limit(limit)
							.and_offset(offset)
							.order_by_score(),
					),
				)?;

				let top_docs = top_docs
					.into_iter()
					.map(|(score, addr)| {
						let segment_reader = searcher.segment_reader(addr.segment_ord);
						let fast_field_reader =
							segment_reader.fast_fields().date("published_at").unwrap();
						let published_at = fast_field_reader.first(addr.doc_id).unwrap();

						(score, published_at, addr)
					})
					.sorted_by(|a, b| {
						b.0.partial_cmp(&a.0)
							.unwrap_or(std::cmp::Ordering::Equal)
							.then_with(|| b.1.cmp(&a.1))
					})
					.map(|(.., addr)| addr)
					.collect::<Vec<_>>();

				(count, top_docs)
			};

			let top_docs = top_docs
				.into_iter()
				.map(|addr| {
					(&searcher.doc::<HashMap<_, _>>(addr).unwrap()[&fields.pos])
						.as_u64()
						.unwrap()
				})
				.collect::<Vec<_>>();

			Ok::<_, anyhow::Error>((count, top_docs))
		})
		.await??;

		let results = stream::iter(top_docs)
			.then(async |pos| backend.search_result_by_pos(pos).await)
			.try_collect::<Vec<_>>()
			.await?;

		Ok(QueryResult { count, results })
	}
}
