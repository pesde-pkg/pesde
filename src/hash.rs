use std::fmt::Display;
use std::str::FromStr;

use digest::DynDigest;
use sha2::Sha256;

use crate::ser_display_deser_fromstr;

/// Hash algorithms that are supported for verifying the integrity of data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum HashAlgorithm {
	/// The SHA-256 hash algorithm
	#[default]
	Sha256,
}
ser_display_deser_fromstr!(HashAlgorithm);

impl HashAlgorithm {
	/// Returns a hasher for this hash algorithm
	#[must_use]
	pub fn hasher(self) -> Box<dyn DynDigest + Send> {
		match self {
			HashAlgorithm::Sha256 => Box::new(Sha256::default()),
		}
	}
}

impl Display for HashAlgorithm {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			HashAlgorithm::Sha256 => write!(f, "sha256"),
		}
	}
}

impl FromStr for HashAlgorithm {
	type Err = errors::HashAlgorithmFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"sha256" => Ok(HashAlgorithm::Sha256),
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
	hash: String,
}
ser_display_deser_fromstr!(Hash);

impl Hash {
	/// Creates a new Hash from the given algorithm and hash value
	#[must_use]
	pub fn new(algorithm: HashAlgorithm, hash: String) -> Self {
		Self { algorithm, hash }
	}

	/// Creates a new Hash from the given algorithm and hash bytes
	#[must_use]
	pub fn from_hash_bytes(algorithm: HashAlgorithm, bytes: impl AsRef<[u8]>) -> Self {
		Self {
			algorithm,
			hash: hex::encode(bytes),
		}
	}

	/// Creates a new Hash from the given algorithm and bytes
	#[must_use]
	pub fn from_bytes(algorithm: HashAlgorithm, bytes: impl AsRef<[u8]>) -> Self {
		let mut hasher = algorithm.hasher();
		hasher.update(bytes.as_ref());
		Self::from_hash_bytes(algorithm, hasher.finalize())
	}

	/// Returns the hash algorithm used to create this hash
	#[must_use]
	pub fn algorithm(&self) -> HashAlgorithm {
		self.algorithm
	}

	/// Returns the hash value
	#[must_use]
	pub fn hash(&self) -> &str {
		&self.hash
	}

	/// Returns the optimal prefix length of the hash for storage in the CAS
	#[must_use]
	pub fn optimal_prefix_length(&self) -> usize {
		2
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

		Ok(Self {
			algorithm: algorithm.parse()?,
			hash: hash.to_string(),
		})
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
	}
}
