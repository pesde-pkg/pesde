use reqwest::Body;
use reqwest::header::CONTENT_ENCODING;
use reqwest::header::CONTENT_TYPE;
use rusty_s3::actions::PutObject;
use semver::Version;
use std::path::PathBuf;
use tokio::io::AsyncBufRead;
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

const ARCHIVE_CONTENT_TYPE: &str = "application/gzip";
const README_CONTENT_TYPE: &str = "text/markdown; charset=utf-8";

pub enum BlobStorage {
	FS(PathBuf),
	S3 {
		bucket: Bucket,
		credentials: Credentials,
		reqwest: reqwest::Client,
	},
}

pub enum BlobResponse {
	File {
		file: fs::File,
		content_type: &'static str,
	},
	Url(String),
}

impl From<BlobResponse> for HttpResponse {
	fn from(response: BlobResponse) -> HttpResponse {
		match response {
			BlobResponse::File { file, content_type } => HttpResponse::Ok()
				.content_type(content_type)
				.body(BodyStream::new(ReaderStream::new(file))),
			BlobResponse::Url(url) => HttpResponse::TemporaryRedirect()
				.insert_header((header::LOCATION, url))
				.finish(),
		}
	}
}

fn object_key(prefix: &str, name: &PackageName, version: &Version) -> String {
	format!("{prefix}/{}/{}/{version}", name.scope(), name.name())
}

impl BlobStorage {
	pub async fn get_package_archive(
		&self,
		name: &PackageName,
		version: &Version,
	) -> anyhow::Result<Option<BlobResponse>> {
		self.get_object("packages", name, version, "application/octet-stream")
			.await
	}

	pub async fn put_package_archive<R: AsyncBufRead + Unpin + Send + 'static>(
		&self,
		name: &PackageName,
		version: &Version,
		data: R,
	) -> anyhow::Result<()> {
		self.put_object(
			"packages",
			name,
			version,
			data,
			ARCHIVE_CONTENT_TYPE,
			Some("gzip"),
		)
		.await
	}

	pub async fn get_package_readme(
		&self,
		name: &PackageName,
		version: &Version,
	) -> anyhow::Result<Option<BlobResponse>> {
		self.get_object("readmes", name, version, README_CONTENT_TYPE)
			.await
	}

	pub async fn put_package_readme<R: AsyncBufRead + Unpin + Send + 'static>(
		&self,
		name: &PackageName,
		version: &Version,
		data: R,
	) -> anyhow::Result<()> {
		self.put_object("readmes", name, version, data, README_CONTENT_TYPE, None)
			.await
	}

	async fn get_object(
		&self,
		prefix: &str,
		name: &PackageName,
		version: &Version,
		content_type: &'static str,
	) -> anyhow::Result<Option<BlobResponse>> {
		match self {
			BlobStorage::FS(root) => {
				match fs::File::open(root.join(object_key(prefix, name, version))).await {
					Ok(file) => Ok(Some(BlobResponse::File { file, content_type })),
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
					Err(e) => Err(e.into()),
				}
			}
			BlobStorage::S3 {
				bucket,
				credentials,
				..
			} => {
				let key = object_key(prefix, name, version);
				let object_url =
					GetObject::new(bucket, Some(credentials), &key).sign(S3_SIGN_DURATION);
				Ok(Some(BlobResponse::Url(object_url.to_string())))
			}
		}
	}

	async fn put_object<R: AsyncBufRead + Unpin + Send + 'static>(
		&self,
		prefix: &str,
		name: &PackageName,
		version: &Version,
		mut data: R,
		content_type: &str,
		content_encoding: Option<&str>,
	) -> anyhow::Result<()> {
		match self {
			BlobStorage::FS(root) => {
				let path = root.join(object_key(prefix, name, version));
				if let Some(parent) = path.parent() {
					fs::create_dir_all(parent).await?;
				}

				let mut file = fs::File::create(path).await?;
				tokio::io::copy_buf(&mut data, &mut file).await?;

				Ok(())
			}
			BlobStorage::S3 {
				bucket,
				credentials,
				reqwest,
			} => {
				let key = object_key(prefix, name, version);
				let object_url =
					PutObject::new(bucket, Some(credentials), &key).sign(S3_SIGN_DURATION);

				let mut request = reqwest
					.put(object_url)
					.header(CONTENT_TYPE, content_type)
					.body(Body::wrap_stream(ReaderStream::new(data)));
				if let Some(encoding) = content_encoding {
					request = request.header(CONTENT_ENCODING, encoding);
				}

				request.send().await?.error_for_status()?;

				Ok(())
			}
		}
	}
}
