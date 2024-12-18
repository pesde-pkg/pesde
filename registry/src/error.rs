use actix_web::{body::BoxBody, HttpResponse, ResponseError};
use pesde::source::git_index::errors::{ReadFile, RefreshError, TreeError};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to parse query")]
    Query(#[from] tantivy::query::QueryParserError),

    #[error("error reading repo file")]
    ReadFile(#[from] ReadFile),

    #[error("error deserializing file")]
    Deserialize(#[from] toml::de::Error),

    #[error("failed to send request: {1}\nserver response: {0}")]
    ReqwestResponse(String, #[source] reqwest::Error),

    #[error("error sending request")]
    Reqwest(#[from] reqwest::Error),

    #[error("failed to parse archive entries")]
    Tar(#[from] std::io::Error),

    #[error("invalid archive")]
    InvalidArchive(String),

    #[error("failed to read index config")]
    Config(#[from] pesde::source::pesde::errors::ConfigError),

    #[error("git error")]
    Git(#[from] git2::Error),

    #[error("failed to refresh source")]
    Refresh(#[from] Box<RefreshError>),

    #[error("failed to serialize struct")]
    Serialize(#[from] toml::ser::Error),

    #[error("failed to serialize struct")]
    SerializeJson(#[from] serde_json::Error),

    #[error("failed to open git repo")]
    OpenRepo(#[from] gix::open::Error),

    #[error("failed to get root tree")]
    RootTree(#[from] TreeError),
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

impl ResponseError for Error {
    fn error_response(&self) -> HttpResponse<BoxBody> {
        match self {
            Error::Query(e) => HttpResponse::BadRequest().json(ErrorResponse {
                error: format!("failed to parse query: {e}"),
            }),
            Error::Tar(_) => HttpResponse::BadRequest().json(ErrorResponse {
                error: "corrupt archive".to_string(),
            }),
            Error::InvalidArchive(e) => HttpResponse::BadRequest().json(ErrorResponse {
                error: format!("archive is invalid: {e}"),
            }),
            e => {
                tracing::error!("unhandled error: {e:?}");
                HttpResponse::InternalServerError().finish()
            }
        }
    }
}

pub trait ReqwestErrorExt {
    async fn into_error(self) -> Result<Self, Error>
    where
        Self: Sized;
}

impl ReqwestErrorExt for reqwest::Response {
    async fn into_error(self) -> Result<Self, Error> {
        match self.error_for_status_ref() {
            Ok(_) => Ok(self),
            Err(e) => Err(Error::ReqwestResponse(self.text().await?, e)),
        }
    }
}
