mod deprecate;
mod get_archive;
mod get_version;
mod get_versions;
mod publish;
mod yank;

pub fn http_v2() -> actix_web::Scope {
	actix_web::web::scope("/v2")
		.service(deprecate::http_v2)
		.service(get_archive::http_v2)
		.service(get_version::http_v2)
		.service(get_versions::http_v2)
		.service(publish::http_v2)
		.service(yank::http_v2)
}
