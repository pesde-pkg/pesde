mod error;
mod get_entry;
mod get_head;
mod post_entry;

pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_head::http_v2)
		.service(get_entry::http_v2)
		.service(post_entry::http_v2);
}
