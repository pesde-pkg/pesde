mod log;
mod package;
mod scope;

pub fn api(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(
		actix_web::web::scope("/v2")
			.configure(log::http_v2)
			.configure(package::http_v2)
			.configure(scope::http_v2),
	);
}
