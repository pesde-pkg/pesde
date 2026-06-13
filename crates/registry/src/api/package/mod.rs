mod deprecate;
mod error;
mod get_archive;
mod get_package;
mod get_readme;
mod get_version;
mod get_versions;
mod publish;
mod yank;

pub use error::Error;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(deprecate::http_v2)
		.service(get_archive::http_v2)
		.service(get_readme::http_v2)
		.service(get_versions::http_v2)
		.service(get_package::http_v2)
		.service(get_version::http_v2)
		.service(publish::http_v2)
		.service(yank::http_v2);
}
