mod error;
mod get_entry;

pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.configure(get_entry::http_v2);
}

macro_rules! tree_handler {
	($route:literal, http_v2 => $http_v2:ident) => {
		pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
			cfg.service(
				web::resource(concat!("/tree/scope_members/{root}", $route))
					.get($http_v2::<ScopeMembersTree>),
			)
			.service(
				web::resource(concat!("/tree/package_versions/{root}", $route))
					.get($http_v2::<PackageVersionsTree>),
			)
			.service(
				web::resource(concat!("/tree/package_deprecations/{root}", $route))
					.get($http_v2::<PackageDeprecationsTree>),
			);
		}
	};
}
use tree_handler;
