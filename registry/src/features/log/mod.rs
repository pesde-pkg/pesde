mod get_entry;
mod get_head;
mod get_inclusion;

pub fn http_v2() -> actix_web::Scope {
	actix_web::web::scope("/v2")
		.service(get_entry::http_v2)
		.service(get_head::http_v2)
		.service(get_inclusion::http_v2)
}
