mod error;
mod get_entries;
mod get_entry;
mod get_transition;

pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.configure(get_entry::http_v2)
		.configure(get_entries::http_v2)
		.configure(get_transition::http_v2);
}

macro_rules! tree_handler {
	(($method:expr) $route:literal, http_v2 => $http_v2:ident) => {
		pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
			let method = $method;

			cfg.service(
				web::resource(concat!("/tree/scope_members/{root}", $route))
					.route(web::method(method.clone()).to($http_v2::<ScopeMembersTree>)),
			)
			.service(
				web::resource(concat!("/tree/package_versions/{root}", $route))
					.route(web::method(method.clone()).to($http_v2::<PackageVersionsTree>)),
			)
			.service(
				web::resource(concat!("/tree/package_deprecations/{root}", $route))
					.route(web::method(method).to($http_v2::<PackageDeprecationsTree>)),
			);
		}
	};
}
use tree_handler;
