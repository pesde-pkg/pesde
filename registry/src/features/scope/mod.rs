mod get_head;
mod update_manifest;

pub fn http_v2() -> actix_web::Scope {
	actix_web::web::scope("/v2")
		.service(get_head::http_v2)
		.service(update_manifest::http_v2)
}
