//! Length limited types

use std::collections::BTreeMap;
use std::fmt::Display;
use std::ops::Deref;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

/// A value whose textual form is at most `N` characters
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(transparent)]
pub struct Bounded<T, const N: usize>(T);

/// A string of at most `N` characters
pub type BoundedString<const N: usize> = Bounded<String, N>;

impl<T: Display, const N: usize> Bounded<T, N> {
	/// Wraps a value ensuring its length is compatible
	pub fn new(value: T) -> Result<Self, errors::TooLongError> {
		let actual = value.to_string().chars().count();
		if actual > N {
			return Err(errors::TooLongErrorKind { limit: N, actual }.into());
		}

		Ok(Self(value))
	}

	/// Returns the inner value
	#[must_use]
	pub fn into_inner(self) -> T {
		self.0
	}
}

impl<T, const N: usize> Deref for Bounded<T, N> {
	type Target = T;

	fn deref(&self) -> &T {
		&self.0
	}
}

impl<T: Display, const N: usize> Display for Bounded<T, N> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl<T, const N: usize> FromStr for Bounded<T, N>
where
	T: FromStr + Display,
	T::Err: std::error::Error + Send + Sync + 'static,
{
	type Err = errors::ParseBoundedError<T::Err>;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let value: T = s.parse().map_err(errors::ParseBoundedError::Parse)?;
		Ok(Self::new(value)?)
	}
}

impl<'de, T: Deserialize<'de> + Display, const N: usize> Deserialize<'de> for Bounded<T, N> {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		Self::new(T::deserialize(deserializer)?).map_err(serde::de::Error::custom)
	}
}

/// A collection with a known length
#[expect(clippy::len_without_is_empty)]
pub trait Collection {
	/// Returns the number of elements
	fn len(&self) -> usize;
}

impl<T> Collection for Vec<T> {
	fn len(&self) -> usize {
		self.len()
	}
}

impl<K, V> Collection for BTreeMap<K, V> {
	fn len(&self) -> usize {
		self.len()
	}
}

/// A collection of at most `N` elements
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(transparent)]
pub struct BoundedCollection<C, const N: usize>(C);

/// A `Vec` of at most `N` elements
pub type BoundedVec<T, const N: usize> = BoundedCollection<Vec<T>, N>;

/// A `BTreeMap` of at most `N` entries
pub type BoundedBTreeMap<K, V, const N: usize> = BoundedCollection<BTreeMap<K, V>, N>;

impl<C: Collection, const N: usize> BoundedCollection<C, N> {
	/// Wraps a collection, ensuring its length is compatbile
	pub fn new(value: C) -> Result<Self, errors::TooManyError> {
		let actual = value.len();
		if actual > N {
			return Err(errors::TooManyErrorKind { limit: N, actual }.into());
		}

		Ok(Self(value))
	}

	/// Returns the inner collection
	#[must_use]
	pub fn into_inner(self) -> C {
		self.0
	}
}

impl<C, const N: usize> Deref for BoundedCollection<C, N> {
	type Target = C;

	fn deref(&self) -> &C {
		&self.0
	}
}

impl<'de, C: Deserialize<'de> + Collection, const N: usize> Deserialize<'de>
	for BoundedCollection<C, N>
{
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		Self::new(C::deserialize(deserializer)?).map_err(serde::de::Error::custom)
	}
}

/// Errors that can occur when constructing a length-bounded value
pub mod errors {
	use thiserror::Error;

	/// A value's textual form exceeds its character limit
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = TooLongError))]
	#[error("value exceeds the maximum length of {limit} characters (got {actual})")]
	pub struct TooLongErrorKind {
		/// The maximum allowed length
		pub limit: usize,
		/// The actual length
		pub actual: usize,
	}

	/// A collection exceeds its size limit
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = TooManyError))]
	#[error("collection exceeds the maximum size of {limit} (got {actual})")]
	pub struct TooManyErrorKind {
		/// The maximum allowed size
		pub limit: usize,
		/// The actual size
		pub actual: usize,
	}

	/// Parsing a length-bounded value from a string failed, generic over the
	/// inner value's parse error `E`
	#[derive(Debug, Error)]
	pub enum ParseBoundedError<E> {
		/// The string could not be parsed into the inner value
		#[error(transparent)]
		Parse(E),

		/// The parsed value exceeds its character limit
		#[error(transparent)]
		TooLong(#[from] TooLongError),
	}
}
