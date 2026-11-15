mod error;
mod get_by_hash;

pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_by_hash::http_v2);
}
