mod error;
mod update_manifest;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(update_manifest::http_v2);
}
