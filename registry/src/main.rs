use actix_cors::Cors;
use actix_web::App;
use actix_web::HttpServer;
use actix_web::middleware::Compress;
use actix_web::middleware::NormalizePath;
use actix_web::middleware::TrailingSlash;
use actix_web::web;
use tracing::level_filters::LevelFilter;
use tracing_actix_web::TracingLogger;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::repos::Repos;
use crate::util::Env;

mod repos;
mod util;

pub struct AppState {
	pub repos: Repos,
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

	dotenvy::dotenv().unwrap();

	let address = Env::new("ADDRESS")
		.try_get()
		.await
		.unwrap_or_else(|| "0.0.0.0".to_string());
	let port = Env::new("PORT").try_parse().await.unwrap_or(8080);
	let database_url = Env::new("DATABASE_URL").get().await;

	let app_state = web::Data::new(AppState {
		repos: Repos::new(&database_url).await,
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
	})
	.bind((address, port))?
	.run()
	.await
}
