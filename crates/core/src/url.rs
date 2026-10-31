use thiserror::Error;

use crate::__private::ser_display_deser_fromstr;
use std::{fmt::Display, ops::Deref, str::FromStr, sync::Arc};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct UrlInner {
	url: url::Url,
}

/// A thin wrapper around `url::Url` to provide cheap cloning and normalisation
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Url(Arc<UrlInner>);
ser_display_deser_fromstr!(Url);

impl Url {
	/// Creates a new [`Url`] from a [`url::Url`]
	#[expect(clippy::unnecessary_wraps)]
	#[must_use]
	pub fn new(url: url::Url) -> Result<Self, NewUrlError> {
		let inner = UrlInner { url };
		Ok(Url(Arc::new(inner)))
	}

	/// Returns the underlying [url::Url]
	#[must_use]
	pub fn as_url(&self) -> &url::Url {
		&self.0.url
	}
}

impl FromStr for Url {
	type Err = UrlFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(Url::new(url::Url::parse(s)?)?)
	}
}

impl Display for Url {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.as_url())
	}
}

impl Deref for Url {
	type Target = url::Url;

	fn deref(&self) -> &Self::Target {
		self.as_url()
	}
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NewUrlError {}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UrlFromStrError {
	#[error("failed to parse url")]
	ParseUrl(#[from] url::ParseError),

	#[error(transparent)]
	NewUrl(#[from] NewUrlError),
}
