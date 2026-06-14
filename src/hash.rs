//! Hashing
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use digest::DynDigest;
use sha2::Sha512;

use crate::ser_display_deser_fromstr;

/// A raw hash digest
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "sqlx", sqlx(transparent))]
pub struct RawHash(Arc<[u8]>);
ser_display_deser_fromstr!(RawHash);

impl RawHash {
	/// Returns the raw bytes of this digest
	#[must_use]
	pub fn as_bytes(&self) -> &[u8] {
		&self.0
	}
}

impl<T: Into<Arc<[u8]>>> From<T> for RawHash {
	fn from(value: T) -> Self {
		Self(value.into())
	}
}

impl AsRef<[u8]> for RawHash {
	fn as_ref(&self) -> &[u8] {
		&self.0
	}
}

impl Display for RawHash {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", fast32::base32::CROCKFORD_LOWER.encode(&self.0))
	}
}

impl FromStr for RawHash {
	type Err = fast32::DecodeError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		fast32::base32::CROCKFORD_LOWER
			.decode(s.as_bytes())
			.map(Into::into)
	}
}

/// Hash algorithms that are supported for verifying the integrity of data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum HashAlgorithm {
	/// The SHA-256 hash algorithm
	#[default]
	Sha512,
}
ser_display_deser_fromstr!(HashAlgorithm);

impl HashAlgorithm {
	/// Returns a hasher for this hash algorithm
	#[must_use]
	pub fn hasher(self) -> Box<dyn DynDigest + Send> {
		match self {
			HashAlgorithm::Sha512 => Box::new(Sha512::default()),
		}
	}

	/// Returns the optimal prefix length of the hash for storage in the CAS
	#[must_use]
	pub fn optimal_prefix_length(self) -> usize {
		2
	}
}

impl Display for HashAlgorithm {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			HashAlgorithm::Sha512 => write!(f, "sha512"),
		}
	}
}

impl FromStr for HashAlgorithm {
	type Err = errors::HashAlgorithmFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"sha512" => Ok(HashAlgorithm::Sha512),
			_ => Err(
				errors::HashAlgorithmFromStrErrorKind::UnknownHashAlgorithm(s.to_string()).into(),
			),
		}
	}
}

/// A hash of some data, consisting of the hash algorithm and the hash value
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hash {
	algorithm: HashAlgorithm,
	hash: RawHash,
}
ser_display_deser_fromstr!(Hash);

impl Hash {
	/// Creates a new Hash from the given algorithm and hash value
	#[must_use]
	pub fn new(algorithm: HashAlgorithm, hash: impl Into<RawHash>) -> Option<Self> {
		let hash = hash.into();
		if hash.as_bytes().len() != algorithm.hasher().output_size() {
			return None;
		}

		Some(Self { algorithm, hash })
	}

	/// Creates a new Hash from the given algorithm and bytes
	#[must_use]
	pub fn from_bytes(algorithm: HashAlgorithm, bytes: impl AsRef<[u8]>) -> Self {
		let mut hasher = algorithm.hasher();
		hasher.update(bytes.as_ref());
		Self::new(algorithm, hasher.finalize()).unwrap()
	}

	/// Returns the hash algorithm used to create this hash
	#[must_use]
	pub fn algorithm(&self) -> HashAlgorithm {
		self.algorithm
	}

	/// Returns the hash value
	#[must_use]
	pub fn hash(&self) -> &RawHash {
		&self.hash
	}
}

impl Display for Hash {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}:{}", self.algorithm, self.hash)
	}
}

impl FromStr for Hash {
	type Err = errors::HashFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (algorithm, hash) = s
			.split_once(':')
			.ok_or(errors::HashFromStrErrorKind::InvalidHashFormat)?;

		// prevent mismatches between serialized and deserialized hashes due to case differences in the hash value
		if hash
			.chars()
			.any(|c| c.is_ascii_alphabetic() && !c.is_ascii_lowercase())
		{
			return Err(errors::HashFromStrErrorKind::InvalidHashFormat.into());
		}

		let algorithm: HashAlgorithm = algorithm.parse()?;
		let mut data = Vec::with_capacity(algorithm.hasher().output_size());
		fast32::base32::CROCKFORD_LOWER.decode_into(hash.as_bytes(), &mut data)?;

		let hash = Self::new(algorithm, data);
		Ok(hash.ok_or(errors::HashFromStrErrorKind::InvalidHashFormat)?)
	}
}

/// Errors that can occur when interacting with hashes
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a hash algorithm from a string
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = HashAlgorithmFromStrError))]
	#[non_exhaustive]
	pub enum HashAlgorithmFromStrErrorKind {
		/// Unknown hash algorithm
		#[error("unknown hash algorithm `{0}`")]
		UnknownHashAlgorithm(String),
	}

	/// Errors that can occur when parsing a hash from a string
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = HashFromStrError))]
	#[non_exhaustive]
	pub enum HashFromStrErrorKind {
		/// Invalid hash format
		#[error("invalid hash format")]
		InvalidHashFormat,

		/// Error parsing the hash algorithm        
		#[error("error parsing hash algorithm")]
		HashAlgorithmFromStr(#[from] HashAlgorithmFromStrError),

		/// Error parsing the hash value
		#[error("error parsing hash value")]
		InvalidHashValue(#[from] fast32::DecodeError),
	}
}
