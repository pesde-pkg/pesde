mod error;
mod get_archive;
mod publish;

pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_archive::http_v2).service(publish::http_v2);
}
