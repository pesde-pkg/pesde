//! Hashing.
//! In pesde, hashes are always encoded as lowercase Crockford base32 because:
//! - base32 is a power of 2 base which makes interacting with it efficient
//! - the alphabet is filesystem friendly: ASCII and not case sensitive
//! - one character carries 5 bits as opposed to base16's 4
//! - Crockford's alphabet makes it harder to confuse hashes
use std::fmt::Display;
use std::io::Write;
use std::str::FromStr;
use std::sync::Arc;

use crate::ser_display_deser_fromstr;

mod encoding {
	fast32::make_base32_alpha!(
		STRICT_CROCKFORD_LOWER,
		STRICT_DEC_CROCKFORD_LOWER,
		b"0123456789abcdefghjkmnpqrstvwxyz"
	);
}

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
		write!(f, "{}", encoding::STRICT_CROCKFORD_LOWER.encode(&self.0))
	}
}

impl FromStr for RawHash {
	type Err = fast32::DecodeError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		encoding::STRICT_CROCKFORD_LOWER
			.decode(s.as_bytes())
			.map(Into::into)
	}
}

/// Hash algorithms that are supported for verifying the integrity of data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum HashAlgorithm {
	/// The BLAKE3 hash algorithm
	#[default]
	Blake3,
}
ser_display_deser_fromstr!(HashAlgorithm);

impl HashAlgorithm {
	/// Returns a hasher for this hash algorithm
	#[must_use]
	pub fn hasher(self) -> Hasher {
		match self {
			Self::Blake3 => Hasher(HasherInner::Blake3(blake3::Hasher::new())),
		}
	}

	/// Returns the length, in bytes, of this algorithm's output size
	#[must_use]
	pub const fn output_size(self) -> usize {
		match self {
			Self::Blake3 => blake3::OUT_LEN,
		}
	}

	/// Returns the optimal prefix length of the hash for storage in the CAS
	#[must_use]
	pub const fn optimal_prefix_parts(self) -> &'static [usize] {
		match self {
			Self::Blake3 => &[2],
		}
	}
}

impl Display for HashAlgorithm {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			HashAlgorithm::Blake3 => write!(f, "blake3"),
		}
	}
}

impl FromStr for HashAlgorithm {
	type Err = errors::HashAlgorithmFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"blake3" => Ok(HashAlgorithm::Blake3),
			_ => Err(
				errors::HashAlgorithmFromStrErrorKind::UnknownHashAlgorithm(s.to_string()).into(),
			),
		}
	}
}

enum HasherInner {
	Blake3(blake3::Hasher),
}

/// A constructor of a hash allowing passing in data in chunks
pub struct Hasher(HasherInner);

impl Hasher {
	/// Appends a new chunk of data to the hasher state
	pub fn update(&mut self, input: &[u8]) {
		match &mut self.0 {
			HasherInner::Blake3(blake3) => blake3.update(input),
		};
	}

	/// Returns the [Hash] the inputs created
	#[must_use]
	pub fn finalize(self) -> Hash {
		match self.0 {
			HasherInner::Blake3(blake3) => {
				let bytes: [u8; _] = blake3.finalize().into();

				Hash {
					algorithm: HashAlgorithm::Blake3,
					hash: bytes.into(),
				}
			}
		}
	}
}

impl Write for Hasher {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		self.update(buf);
		Ok(buf.len())
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
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
		if hash.as_bytes().len() != algorithm.output_size() {
			return None;
		}

		Some(Self { algorithm, hash })
	}

	/// Creates a new Hash from the given algorithm and bytes
	#[must_use]
	pub fn from_bytes(algorithm: HashAlgorithm, bytes: impl AsRef<[u8]>) -> Self {
		let mut hasher = algorithm.hasher();
		hasher.update(bytes.as_ref());
		hasher.finalize()
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

	/// Consumes self and returns the hash value
	#[must_use]
	pub fn into_hash(self) -> RawHash {
		self.hash
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

		let algorithm: HashAlgorithm = algorithm.parse()?;
		let mut data = Vec::with_capacity(algorithm.output_size());
		encoding::STRICT_CROCKFORD_LOWER.decode_into(hash.as_bytes(), &mut data)?;

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
