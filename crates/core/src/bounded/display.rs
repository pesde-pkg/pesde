use std::{convert::Infallible, fmt::Display, ops::Deref, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bounded::BoundedSizeError;

macro_rules! bounded_impl {
	(fromstr $(#[$meta:meta])* $name:ident, $ty:ty) => {
		bounded_impl!($name, $ty, <$ty as FromStr>::Err, FromStr::from_str);
	};
	($(#[$meta:meta])* $name:ident, $ty:ty, $err:ty, $parse:expr) => {
		/// A value whose textual form is at most `N` characters
		$(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
		#[serde(transparent)]
		pub struct $name<const N: usize>($ty);

		impl<const N: usize> $name<N> {
			/// Wraps a value, ensuring its length is compatible
			pub fn new(value: impl Into<$ty>) -> Result<Self, BoundedSizeError> {
				let value = value.into();
				let actual = value.to_string().chars().count();
				if actual > N {
					return Err(BoundedSizeError { limit: N, actual });
				}

				Ok(Self(value))
			}

			/// Returns the inner value
			#[must_use]
			pub fn into_inner(self) -> $ty {
				self.0
			}
		}

		impl<const N: usize> Deref for $name<N> {
			type Target = $ty;

			fn deref(&self) -> &$ty {
				&self.0
			}
		}

		impl<const N: usize> Display for $name<N> {
			fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				self.0.fmt(f)
			}
		}

		impl<const N: usize> FromStr for $name<N> {
			type Err = BoundedFromStrError<$err>;

			fn from_str(s: &str) -> Result<Self, Self::Err> {
				let len = s.chars().count();
				if len > N {
					return Err(BoundedFromStrError::Size(BoundedSizeError {
						limit: N,
						actual: len,
					}));
				}

				$parse(s).map_err(BoundedFromStrError::Parse).map(Self)
			}
		}

		impl<'de, const N: usize> Deserialize<'de> for $name<N> {
			fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
				struct Visitor<const N: usize>;
				impl<const N: usize> serde::de::Visitor<'_> for Visitor<N> {
					type Value = $name<N>;

					fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
						write!(formatter, "a string of at most {N} characters")
					}

					fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
					where
						E: serde::de::Error,
					{
						match $name::<N>::from_str(v) {
							Ok(v) => Ok(v),
							Err(BoundedFromStrError::Size(e)) => {
								Err(serde::de::Error::invalid_length(e.actual, &self))
							}
							Err(BoundedFromStrError::Parse(e)) => Err(serde::de::Error::custom(e)),
						}
					}
				}

				deserializer.deserialize_str(Visitor::<N>)
			}
		}
	};
}

// Box<str> doesn't implement FromStr which is why this macro is non-trivial
bounded_impl!(
	#[derive(Default, PartialOrd, Ord)]
	BoundedStr,
	Box<str>,
	Infallible,
	|s: &str| Ok(s.into())
);
bounded_impl!(
	fromstr
	#[derive(PartialOrd, Ord)]
	BoundedUrl,
	crate::url::Url
);
bounded_impl!(
	fromstr
	BoundedVersionReq,
	semver::VersionReq
);

#[derive(Debug, Error)]
pub enum BoundedFromStrError<E> {
	#[error(transparent)]
	Size(#[from] BoundedSizeError),

	#[error(transparent)]
	Parse(E),
}
