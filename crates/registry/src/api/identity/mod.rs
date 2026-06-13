mod error;
mod get_identity;
mod register_identity;
mod rotate_key;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_identity::http_v2)
		.service(register_identity::http_v2)
		.service(rotate_key::http_v2);
}
