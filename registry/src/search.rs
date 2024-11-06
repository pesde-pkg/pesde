use crate::AppState;
use pesde::{
    names::PackageName,
    source::pesde::{IndexFileEntry, PesdePackageSource},
    Project,
};
use tantivy::{
    doc,
    query::QueryParser,
    schema::{IndexRecordOption, TextFieldIndexing, TextOptions, FAST, STORED, STRING},
    tokenizer::TextAnalyzer,
    DateTime, IndexReader, IndexWriter, Term,
};

pub fn make_search(
    project: &Project,
    source: &PesdePackageSource,
) -> (IndexReader, IndexWriter, QueryParser) {
    let mut schema_builder = tantivy::schema::SchemaBuilder::new();

    let field_options = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("ngram")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );

    let id_field = schema_builder.add_text_field("id", STRING | STORED);
    let scope = schema_builder.add_text_field("scope", field_options.clone());
    let name = schema_builder.add_text_field("name", field_options.clone());
    let description = schema_builder.add_text_field("description", field_options);
    let published_at = schema_builder.add_date_field("published_at", FAST);

    let search_index = tantivy::Index::create_in_ram(schema_builder.build());
    search_index.tokenizers().register(
        "ngram",
        TextAnalyzer::builder(tantivy::tokenizer::NgramTokenizer::all_ngrams(1, 12).unwrap())
            .filter(tantivy::tokenizer::LowerCaser)
            .build(),
    );

    let search_reader = search_index
        .reader_builder()
        .reload_policy(tantivy::ReloadPolicy::Manual)
        .try_into()
        .unwrap();
    let mut search_writer = search_index.writer(50_000_000).unwrap();

    for (pkg_name, mut file) in source.all_packages(project).unwrap() {
        let Some((_, latest_entry)) = file.pop_last() else {
            log::warn!("no versions found for {pkg_name}");
            continue;
        };

        search_writer.add_document(doc!(
            id_field => pkg_name.to_string(),
            scope => pkg_name.as_str().0,
            name => pkg_name.as_str().1,
            description => latest_entry.description.unwrap_or_default(),
            published_at => DateTime::from_timestamp_secs(latest_entry.published_at.timestamp()),
        )).unwrap();
    }

    search_writer.commit().unwrap();
    search_reader.reload().unwrap();

    let mut query_parser = QueryParser::for_index(&search_index, vec![scope, name, description]);
    query_parser.set_field_boost(scope, 2.0);
    query_parser.set_field_boost(name, 3.5);

    (search_reader, search_writer, query_parser)
}

pub fn update_version(app_state: &AppState, name: &PackageName, entry: IndexFileEntry) {
    let mut search_writer = app_state.search_writer.lock().unwrap();
    let schema = search_writer.index().schema();
    let id_field = schema.get_field("id").unwrap();

    search_writer.delete_term(Term::from_field_text(id_field, &name.to_string()));

    search_writer.add_document(doc!(
        id_field => name.to_string(),
        schema.get_field("scope").unwrap() => name.as_str().0,
        schema.get_field("name").unwrap() => name.as_str().1,
        schema.get_field("description").unwrap() => entry.description.unwrap_or_default(),
        schema.get_field("published_at").unwrap() => DateTime::from_timestamp_secs(entry.published_at.timestamp())
    )).unwrap();

    search_writer.commit().unwrap();
    app_state.search_reader.reload().unwrap();
}
