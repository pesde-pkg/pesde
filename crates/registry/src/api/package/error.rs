use actix_web::HttpResponse;
use actix_web::ResponseError;
use bstr::BStr;
use pesde_registry_core::db::StoreError;

use crate::shared::error::Category;
use crate::shared::error::http_response;
use crate::shared::tree::FromTreeError;

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
	#[error(transparent)]
	Internal(#[from] anyhow::Error),

	#[error(transparent)]
	BodyLimit(#[from] actix_web::body::BodyLimitExceeded),

	#[error("the request body was malformed")]
	MalformedBody,

	#[error("the request body was malformed")]
	MalformedJson(#[from] serde_json::Error),

	#[error("the genesis entry is not valid for the provided scope")]
	BadGenesisEntry,

	#[error("a hash does not match the uploaded data")]
	HashMismatch,

	#[error("the uploaded archive is not a valid zip file")]
	InvalidZip(#[from] async_zip::error::ZipError),

	#[error("the uploaded archive is a zip64 file, which is not allowed")]
	Zip64NotAllowed,

	#[error("the path `{0}` in the uploaded archive is not allowed")]
	InvalidPath(Box<BStr>),

	#[error("the manifest file is not valid")]
	InvalidManifest(#[from] toml::de::Error),

	#[error("the manifest file has a state mismatch")]
	ManifestStateMismatch,

	#[error("not authorized to perform this action in the scope")]
	Unauthorized,

	#[error("the package version has already been published")]
	VersionAlreadyExists,

	#[error("invalid tree root")]
	ComputedRootDifferent,

	#[error("`prev_hash` doesn't match")]
	InvalidPrevHash,

	#[error("invalid scope state")]
	InvalidScopeState,
}

impl ResponseError for Error {
	fn error_response(&self) -> HttpResponse {
		let category = match self {
			Error::Internal(_) => Category::Internal,
			Error::BodyLimit(_) => Category::BadRequest,
			Error::MalformedBody => Category::BadRequest,
			Error::MalformedJson(_) => Category::BadRequest,
			Error::BadGenesisEntry => Category::BadRequest,
			Error::HashMismatch => Category::BadRequest,
			Error::InvalidZip(_) => Category::BadRequest,
			Error::Zip64NotAllowed => Category::BadRequest,
			Error::InvalidPath(_) => Category::BadRequest,
			Error::InvalidManifest(_) => Category::BadRequest,
			Error::ManifestStateMismatch => Category::BadRequest,
			Error::Unauthorized => Category::Unauthorized,
			Error::VersionAlreadyExists => Category::Conflict,
			Error::ComputedRootDifferent => Category::BadRequest,
			Error::InvalidPrevHash => Category::Conflict,
			Error::InvalidScopeState => Category::BadRequest,
		};
		http_response(category, self)
	}
}

impl From<StoreError> for Error {
	fn from(value: StoreError) -> Self {
		Self::Internal(value.0)
	}
}
impl FromTreeError for Error {
	fn root_mismatch() -> Self {
		Self::ComputedRootDifferent
	}
}
