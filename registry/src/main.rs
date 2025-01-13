use crate::{
	auth::{get_auth_from_env, Auth, UserIdExtractor},
	search::make_search,
	storage::{get_storage_from_env, Storage},
};
use actix_cors::Cors;
use actix_governor::{Governor, GovernorConfigBuilder};
use actix_web::{
	middleware::{from_fn, Compress, NormalizePath, TrailingSlash},
	rt::System,
	web,
	web::PayloadConfig,
	App, HttpServer,
};
use fs_err::tokio as fs;
use pesde::{
	source::{
		pesde::PesdePackageSource,
		traits::{PackageSource, RefreshOptions},
	},
	AuthConfig, Project,
};
use std::{env::current_dir, path::PathBuf, sync::Arc};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
	fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

mod auth;
mod endpoints;
mod error;
mod git;
mod package;
mod request_path;
mod search;
mod storage;

pub fn make_reqwest() -> reqwest::Client {
	reqwest::ClientBuilder::new()
		.user_agent(concat!(
			env!("CARGO_PKG_NAME"),
			"/",
			env!("CARGO_PKG_VERSION")
		))
		.build()
		.unwrap()
}

pub struct AppState {
	pub source: Arc<tokio::sync::RwLock<PesdePackageSource>>,
	pub project: Project,
	pub storage: Storage,
	pub auth: Auth,

	pub search_reader: tantivy::IndexReader,
	pub search_writer: std::sync::Mutex<tantivy::IndexWriter>,
	pub query_parser: tantivy::query::QueryParser,
}

#[macro_export]
macro_rules! benv {
    ($name:expr) => {
        std::env::var($name)
    };
    ($name:expr => $default:expr) => {
        benv!($name).unwrap_or($default.to_string())
    };
    (required $name:expr) => {
        benv!($name).expect(concat!("Environment variable `", $name, "` must be set"))
    };
    (parse $name:expr) => {
        benv!($name)
            .map(|v| v.parse().expect(concat!(
                "Environment variable `",
                $name,
                "` must be a valid value"
            )))
    };
    (parse required $name:expr) => {
        benv!(parse $name).expect(concat!("Environment variable `", $name, "` must be set"))
    };
    (parse $name:expr => $default:expr) => {
        benv!($name => $default)
            .parse()
            .expect(concat!(
                "Environment variable `",
                $name,
                "` must a valid value"
            ))
    };
}

async fn run() -> std::io::Result<()> {
	let address = benv!("ADDRESS" => "127.0.0.1");
	let port: u16 = benv!(parse "PORT" => "8080");

	let cwd = current_dir().unwrap();
	let data_dir =
		PathBuf::from(benv!("DATA_DIR" => "{CWD}/data").replace("{CWD}", cwd.to_str().unwrap()));
	fs::create_dir_all(&data_dir).await.unwrap();

	let project = Project::new(
		&cwd,
		None::<PathBuf>,
		data_dir.join("project"),
		&cwd,
		AuthConfig::new().with_git_credentials(Some(gix::sec::identity::Account {
			username: benv!(required "GIT_USERNAME"),
			password: benv!(required "GIT_PASSWORD"),
		})),
	);
	let source = PesdePackageSource::new(benv!(required "INDEX_REPO_URL").try_into().unwrap());
	source
		.refresh(&RefreshOptions {
			project: project.clone(),
		})
		.await
		.expect("failed to refresh source");
	let config = source
		.config(&project)
		.await
		.expect("failed to get index config");

	let (search_reader, search_writer, query_parser) = make_search(&project, &source).await;

	let app_data = web::Data::new(AppState {
		storage: {
			let storage = get_storage_from_env();
			tracing::info!("storage: {storage}");
			storage
		},
		auth: {
			let auth = get_auth_from_env(&config);
			tracing::info!("auth: {auth}");
			auth
		},
		source: Arc::new(tokio::sync::RwLock::new(source)),
		project,

		search_reader,
		search_writer: std::sync::Mutex::new(search_writer),
		query_parser,
	});

	let publish_governor_config = GovernorConfigBuilder::default()
		.key_extractor(UserIdExtractor)
		.burst_size(12)
		.seconds_per_request(60)
		.use_headers()
		.finish()
		.unwrap();

	let publish_payload_config = PayloadConfig::new(config.max_archive_size);

	HttpServer::new(move || {
		App::new()
			.wrap(sentry_actix::Sentry::with_transaction())
			.wrap(NormalizePath::new(TrailingSlash::Trim))
			.wrap(Cors::permissive())
			.wrap(tracing_actix_web::TracingLogger::default())
			.wrap(Compress::default())
			.app_data(app_data.clone())
			.route(
				"/",
				web::get().to(|| async {
					concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))
				}),
			)
			.service(
				web::scope("/v0")
					.route(
						"/search",
						web::get()
							.to(endpoints::search::search_packages)
							.wrap(from_fn(auth::read_mw)),
					)
					.route(
						"/packages/{name}",
						web::get()
							.to(endpoints::package_versions::get_package_versions_v0)
							.wrap(from_fn(auth::read_mw)),
					)
					.route(
						"/packages/{name}/{version}/{target}",
						web::get()
							.to(endpoints::package_version::get_package_version_v0)
							.wrap(from_fn(auth::read_mw)),
					)
					.service(
						web::scope("/packages")
							.app_data(publish_payload_config.clone())
							.route(
								"",
								web::post()
									.to(endpoints::publish_version::publish_package)
									.wrap(Governor::new(&publish_governor_config))
									.wrap(from_fn(auth::write_mw)),
							),
					),
			)
			.service(
				web::scope("/v1")
					.route(
						"/search",
						web::get()
							.to(endpoints::search::search_packages)
							.wrap(from_fn(auth::read_mw)),
					)
					.route(
						"/packages/{name}",
						web::get()
							.to(endpoints::package_versions::get_package_versions)
							.wrap(from_fn(auth::read_mw)),
					)
					.service(
						web::resource("/packages/{name}/deprecate")
							.put(endpoints::deprecate_version::deprecate_package_version)
							.delete(endpoints::deprecate_version::deprecate_package_version)
							.wrap(from_fn(auth::write_mw)),
					)
					.route(
						"/packages/{name}/{version}/{target}",
						web::get()
							.to(endpoints::package_version::get_package_version)
							.wrap(from_fn(auth::read_mw)),
					)
					.route(
						"/packages/{name}/{version}/{target}/archive",
						web::get()
							.to(endpoints::package_archive::get_package_archive)
							.wrap(from_fn(auth::read_mw)),
					)
					.route(
						"/packages/{name}/{version}/{target}/doc",
						web::get()
							.to(endpoints::package_doc::get_package_doc)
							.wrap(from_fn(auth::read_mw)),
					)
					.route(
						"/packages/{name}/{version}/{target}/readme",
						web::get()
							.to(endpoints::package_readme::get_package_readme)
							.wrap(from_fn(auth::read_mw)),
					)
					.service(
						web::resource("/packages/{name}/{version}/{target}/yank")
							.put(endpoints::yank_version::yank_package_version)
							.delete(endpoints::yank_version::yank_package_version)
							.wrap(from_fn(auth::write_mw)),
					)
					.service(
						web::scope("/packages")
							.app_data(publish_payload_config.clone())
							.route(
								"",
								web::post()
									.to(endpoints::publish_version::publish_package)
									.wrap(Governor::new(&publish_governor_config))
									.wrap(from_fn(auth::write_mw)),
							),
					),
			)
	})
	.bind((address, port))?
	.run()
	.await
}

// can't use #[actix_web::main] because of Sentry:
// "Note: Macros like #[tokio::main] and #[actix_web::main] are not supported. The Sentry client must be initialized before the async runtime is started so that all threads are correctly connected to the Hub."
// https://docs.sentry.io/platforms/rust/guides/actix-web/
fn main() -> std::io::Result<()> {
	let _ = dotenvy::dotenv();

	let tracing_env_filter = EnvFilter::builder()
		.with_default_directive(LevelFilter::INFO.into())
		.from_env_lossy()
		.add_directive("reqwest=info".parse().unwrap())
		.add_directive("rustls=info".parse().unwrap())
		.add_directive("tokio_util=info".parse().unwrap())
		.add_directive("goblin=info".parse().unwrap())
		.add_directive("tower=info".parse().unwrap())
		.add_directive("hyper=info".parse().unwrap())
		.add_directive("h2=info".parse().unwrap());

	tracing_subscriber::registry()
		.with(tracing_env_filter)
		.with(
			tracing_subscriber::fmt::layer()
				.compact()
				.with_span_events(FmtSpan::NEW | FmtSpan::CLOSE),
		)
		.with(sentry::integrations::tracing::layer())
		.init();

	let guard = sentry::init(sentry::ClientOptions {
		release: sentry::release_name!(),
		dsn: benv!(parse "SENTRY_DSN").ok(),
		session_mode: sentry::SessionMode::Request,
		traces_sample_rate: 1.0,
		debug: true,
		..Default::default()
	});

	if guard.is_enabled() {
		std::env::set_var("RUST_BACKTRACE", "full");
		tracing::info!("sentry initialized");
	} else {
		tracing::info!("sentry **NOT** initialized");
	}

	System::new().block_on(run())
}
