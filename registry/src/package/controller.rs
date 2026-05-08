use actix_web::Scope;

use crate::util::ControllerResult;

pub struct PackageController;

impl PackageController {
	pub fn v2() -> Scope {
		Scope::new("/package").service(
			Scope::new("/{scope}/{name}")
				.route("", actix_web::web::get().to(Self::versions))
				.service(
					Scope::new("/{version}")
						.route("", actix_web::web::get().to(Self::version))
						.route("/archive", actix_web::web::get().to(Self::archive)),
				),
		)
	}

	async fn versions() -> ControllerResult {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn version() -> ControllerResult {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn archive() -> ControllerResult {
		Ok(actix_web::HttpResponse::Ok().finish())
	}
}
