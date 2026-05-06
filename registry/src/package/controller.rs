use actix_web::Scope;

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

	async fn versions() -> actix_web::Result<actix_web::HttpResponse> {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn version() -> actix_web::Result<actix_web::HttpResponse> {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn archive() -> actix_web::Result<actix_web::HttpResponse> {
		Ok(actix_web::HttpResponse::Ok().finish())
	}
}
