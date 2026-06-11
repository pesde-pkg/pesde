use actix_cors::Cors;
use actix_web::App;
use actix_web::HttpServer;
use actix_web::middleware::Compress;
use actix_web::middleware::NormalizePath;
use actix_web::middleware::TrailingSlash;
use actix_web::web;
use rusty_s3::Bucket;
use rusty_s3::Credentials;
use rusty_s3::UrlStyle;
use tracing::level_filters::LevelFilter;
use tracing_actix_web::TracingLogger;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::shared::auth::TokenHash;
use crate::shared::auth::hash_token;
use crate::shared::blob::BlobStorage;
use crate::shared::db::Backend;
use crate::util::Env;

mod features;
pub mod shared;
mod util;

pub struct AppState {
	db: Box<dyn Backend>,
	blob_storage: BlobStorage,
	access_token_hash: Option<TokenHash>,
	read_requires_auth: bool,
	max_archive_size: usize,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let tracing_env_filter = EnvFilter::builder()
		.with_default_directive(LevelFilter::INFO.into())
		.from_env_lossy();

	let fmt_layer = tracing_subscriber::fmt::layer();

	#[cfg(debug_assertions)]
	let fmt_layer = fmt_layer.with_timer(tracing_subscriber::fmt::time::uptime());

	#[cfg(not(debug_assertions))]
	let fmt_layer = fmt_layer.with_timer(tracing_subscriber::fmt::time::time());

	tracing_subscriber::registry()
		.with(tracing_env_filter)
		.with(fmt_layer)
		.init();

	let _ = dotenvy::dotenv();

	let address = Env::new("ADDRESS")
		.try_get()
		.await
		.unwrap_or_else(|| "0.0.0.0".to_string());
	let port = Env::new("PORT").try_parse().await.unwrap_or(8080);

	let max_archive_size: usize = Env::new("MAXIMUM_PACKAGE_ARCHIVE_SIZE")
		.try_parse()
		.await
		.unwrap_or(4 * 1024 * 1024);

	let app_state = web::Data::new(AppState {
		db: shared::db::connect(&Env::new("DATABASE_URL").get().await).await,
		blob_storage: match Env::new("PACKAGE_ARCHIVES_ROOT").try_parse().await {
			Some(root) => BlobStorage::FS(root),
			None => BlobStorage::S3 {
				bucket: Bucket::new(
					Env::new("S3_ENDPOINT").parse().await,
					UrlStyle::Path,
					Env::new("S3_BUCKET_NAME").get().await,
					Env::new("S3_REGION").get().await,
				)
				.unwrap(),
				credentials: Credentials::new(
					Env::new("S3_ACCESS_KEY").get().await,
					Env::new("S3_SECRET_KEY").get().await,
				),
				reqwest: reqwest::Client::builder()
					.user_agent(concat!(
						env!("CARGO_PKG_NAME"),
						"/",
						env!("CARGO_PKG_VERSION")
					))
					.build()
					.unwrap(),
			},
		},
		access_token_hash: Env::new("ACCESS_TOKEN")
			.try_get()
			.await
			.as_deref()
			.map(hash_token),
		read_requires_auth: Env::new("READ_REQUIRES_AUTH")
			.try_get()
			.await
			.is_some_and(|s| !s.is_empty()),
		max_archive_size,
	});

	HttpServer::new(move || {
		App::new()
			.app_data(app_state.clone())
			.wrap(NormalizePath::new(TrailingSlash::Trim))
			.wrap(Cors::permissive())
			.wrap(TracingLogger::default())
			.wrap(Compress::default())
			.route(
				"/",
				web::get()
					.to(async || concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))),
			)
			.service(
				web::scope("/v2")
					.configure(features::log::http_v2)
					.configure(features::package::http_v2)
					.configure(features::scope::http_v2)
					.configure(features::identity::http_v2),
			)
	})
	.bind((address, port))?
	.run()
	.await
}
