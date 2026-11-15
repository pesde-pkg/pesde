mod error;
mod publish;

pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(publish::http_v2);
}
