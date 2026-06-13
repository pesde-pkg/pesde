mod error;
mod get_search;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_search::http_v2);
}
