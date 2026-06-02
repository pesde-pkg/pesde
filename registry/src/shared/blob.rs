use semver::Version;
use std::path::PathBuf;
use tokio_util::io::ReaderStream;

use actix_web::HttpResponse;
use actix_web::body::BodyStream;
use actix_web::http::header;
use fs_err::tokio as fs;
use pesde::names::PackageName;
use rusty_s3::Bucket;
use rusty_s3::Credentials;
use rusty_s3::S3Action as _;
use rusty_s3::actions::GetObject;
use std::time::Duration;

const S3_SIGN_DURATION: Duration = Duration::from_secs(60 * 15);

pub enum BlobStorage {
	FS(PathBuf),
	S3 {
		bucket: Bucket,
		credentials: Credentials,
	},
}

pub enum BlobResponse {
	File(fs::File),
	Url(String),
}

impl From<BlobResponse> for HttpResponse {
	fn from(response: BlobResponse) -> HttpResponse {
		match response {
			BlobResponse::File(file) => {
				let stream = ReaderStream::new(file);
				HttpResponse::Ok()
					.content_type("application/octet-stream")
					.body(BodyStream::new(stream))
			}
			BlobResponse::Url(url) => HttpResponse::TemporaryRedirect()
				.insert_header((header::LOCATION, url))
				.finish(),
		}
	}
}

impl BlobStorage {
	pub async fn get_package_archive(
		&self,
		name: &PackageName,
		version: &Version,
	) -> anyhow::Result<Option<BlobResponse>> {
		match self {
			BlobStorage::FS(root) => {
				let path = root
					.join("packages")
					.join(name.scope().as_str())
					.join(name.name().as_str())
					.join(version.to_string());

				match fs::File::open(path).await {
					Ok(file) => Ok(Some(BlobResponse::File(file))),
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
					Err(e) => Err(e.into()),
				}
			}
			BlobStorage::S3 {
				bucket,
				credentials,
			} => {
				let key = format!("packages/{}/{}/{version}", name.scope(), name.name());
				let object_url =
					GetObject::new(bucket, Some(credentials), &key).sign(S3_SIGN_DURATION);
				Ok(Some(BlobResponse::Url(object_url.to_string())))
			}
		}
	}
}
