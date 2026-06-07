mod get_entry;
mod get_head;
mod get_inclusion;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_entry::http_v2)
		.service(get_head::http_v2)
		.service(get_inclusion::http_v2);
}
