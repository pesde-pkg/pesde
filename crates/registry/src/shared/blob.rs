use pesde::hash::Hash;
use reqwest::Body;
use reqwest::header::CONTENT_TYPE;
use rusty_s3::actions::PutObject;
use std::path::PathBuf;
use tokio::io::AsyncBufRead;
use tokio_util::io::ReaderStream;

use actix_web::HttpResponse;
use actix_web::body::BodyStream;
use actix_web::http::header;
use fs_err::tokio as fs;
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

trait Stored {
	const CONTENT_TYPE: &'static str;

	fn path(&self) -> String;
}

struct PackageArchive<'a> {
	hash: &'a Hash,
}
impl Stored for PackageArchive<'_> {
	const CONTENT_TYPE: &'static str = "application/zstd";

	fn path(&self) -> String {
		let mut prefix = self.hash.encoded();
		let suffix = prefix.split_off(2);
		format!("packages/{}/{prefix}/{suffix}", self.hash.algorithm())
	}
}

impl BlobStorage {
	pub async fn get_package_archive(&self, hash: &Hash) -> anyhow::Result<Option<BlobResponse>> {
		self.get_object(PackageArchive { hash }).await
	}

	pub async fn put_package_archive<R: AsyncBufRead + Unpin + Send + 'static>(
		&self,
		hash: &Hash,
		data: R,
	) -> anyhow::Result<()> {
		self.put_object(PackageArchive { hash }, data).await
	}

	async fn get_object<S: Stored>(&self, stored: S) -> anyhow::Result<Option<BlobResponse>> {
		let path = stored.path();
		match self {
			BlobStorage::FS(root) => match fs::File::open(root.join(path)).await {
				Ok(file) => Ok(Some(BlobResponse::File {
					file,
					content_type: S::CONTENT_TYPE,
				})),
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
				Err(e) => Err(e.into()),
			},
			BlobStorage::S3 {
				bucket,
				credentials,
				..
			} => {
				let object_url =
					GetObject::new(bucket, Some(credentials), &path).sign(S3_SIGN_DURATION);
				Ok(Some(BlobResponse::Url(object_url.to_string())))
			}
		}
	}

	async fn put_object<S: Stored, R: AsyncBufRead + Unpin + Send + 'static>(
		&self,
		stored: S,
		mut data: R,
	) -> anyhow::Result<()> {
		let path = stored.path();
		match self {
			BlobStorage::FS(root) => {
				let path = root.join(path);
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
				let object_url =
					PutObject::new(bucket, Some(credentials), &path).sign(S3_SIGN_DURATION);

				reqwest
					.put(object_url)
					.header(CONTENT_TYPE, S::CONTENT_TYPE)
					.body(Body::wrap_stream(ReaderStream::new(data)))
					.send()
					.await?
					.error_for_status()?;

				Ok(())
			}
		}
	}
}
