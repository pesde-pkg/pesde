//! Hashing.
//! In pesde, hashes are always encoded as lowercase Crockford base32 because:
//! - base32 is a power of 2 base which makes interacting with it efficient
//! - the alphabet is filesystem friendly: ASCII and not case sensitive
//! - one character carries 5 bits as opposed to base16's 4
//! - Crockford's alphabet makes it harder to confuse hashes
use paste::paste;
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

/// A constructor of a hash allowing passing in data in chunks
pub trait Hasher: Write {
	/// The result this produces
	type Output;

	/// Appends a new chunk of data to the hasher state
	fn update(&mut self, input: &[u8]);

	/// Returns the [Self::Output] the inputs created
	#[must_use]
	fn finalize(self) -> Self::Output;
}

macro_rules! algos {
	($(
		$(#[$meta:meta])*
		$algo:ident
	),+) => {
		paste! {
			/// Hash algorithms that are supported for verifying the integrity of data
			#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
			#[non_exhaustive]
			pub enum HashAlgorithm {
				$(
					$(#[$meta])*
					$algo
				),+
			}
			ser_display_deser_fromstr!(HashAlgorithm);

			impl Display for HashAlgorithm {
				fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
					match self {
						$(Self::$algo => f.write_str(stringify!([< $algo:snake >]))),+
					}
				}
			}

			impl FromStr for HashAlgorithm {
				type Err = errors::HashAlgorithmFromStrError;

				fn from_str(s: &str) -> Result<Self, Self::Err> {
					match s {
						$(stringify!([< $algo:snake >]) => Ok(Self::$algo),)+
						s => Err(
							errors::HashAlgorithmFromStrErrorKind::UnknownHashAlgorithm(s.into()).into(),
						),
					}
				}
			}

			enum RuntimeHasher {
				$($algo([< $algo Hasher >])),+
			}

			impl Hasher for RuntimeHasher {
				type Output = Hash;

				fn update(&mut self, input: &[u8]) {
					match self {
						$(Self::$algo(hasher) => hasher.update(input)),+
					};
				}

				fn finalize(self) -> Hash {
					match self {
						$(Self::$algo(hasher) => hasher.finalize().into()),+
					}
				}
			}

			impl Write for RuntimeHasher {
				fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
					self.update(buf);
					Ok(buf.len())
				}

				fn flush(&mut self) -> std::io::Result<()> {
					Ok(())
				}
			}

			impl HashAlgorithm {
				/// Returns a hasher for this hash algorithm
				#[must_use]
				pub fn hasher(self) -> impl Hasher<Output = Hash> {
					match self {
						$(Self::$algo => RuntimeHasher::$algo(Default::default())),+
					}
				}
			}

			// TODO: convert this to be a const generic once adt_const_params is stable
			$(
				#[doc = concat!("A [HashAlgorithm::", stringify!($algo), "] hash")]
				#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
				#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
				#[cfg_attr(feature = "sqlx", sqlx(transparent))]
				pub struct [< $algo Hash >](pub Arc<[u8; { Self::ALGORITHM.output_size() }]>);
				ser_display_deser_fromstr!([< $algo Hash >]);

				impl [< $algo Hash >] {
					/// The hash algorithm used to create this hash
					pub const ALGORITHM: HashAlgorithm = HashAlgorithm::$algo;

					/// Creates a new [Self] from the given hash bytes
					pub fn new(bytes: impl Into<Arc<[u8]>>) -> Result<Self, errors::NewHashError> {
						let bytes = bytes.into();
						let bytes = Arc::<[u8; _]>::try_from(bytes)
							.map_err(|bytes| errors::NewHashErrorKind::InvalidSize {
								actual: bytes.len(),
								expected: Self::ALGORITHM.output_size()
							})?;

						Ok(Self(bytes))
					}

					/// Creates a new [Self] by digesting the given bytes
					#[must_use]
					pub fn digest(bytes: impl AsRef<[u8]>) -> Self {
						let mut hasher = Self::hasher();
						hasher.update(bytes.as_ref());
						hasher.finalize()
					}

					/// Creates a [Hasher] with [Self] as the output
					#[must_use]
					pub fn hasher() -> impl Hasher<Output = Self> {
						[< $algo Hasher >]::default()
					}
				}

				impl Display for [< $algo Hash >] {
					fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
						f.write_str(&encoding::STRICT_CROCKFORD_LOWER.encode(self.0.as_ref()))
					}
				}

				impl FromStr for [< $algo Hash >] {
					type Err = errors::HashDecodeError;

					fn from_str(s: &str) -> Result<Self, Self::Err> {
						let mut data = Vec::new();
						data.reserve_exact(Self::ALGORITHM.output_size());
						encoding::STRICT_CROCKFORD_LOWER.decode_into(s.as_bytes(), &mut data)?;

						Ok(Self::new(data)?)
					}
				}

				impl From<[< $algo Hash >]> for Hash {
					fn from(value: [< $algo Hash >]) -> Self {
						Hash::$algo(value)
					}
				}

				impl Write for [< $algo Hasher >] {
					fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
						self.update(buf);
						Ok(buf.len())
					}

					fn flush(&mut self) -> std::io::Result<()> {
						Ok(())
					}
				}
			)+

			/// A hash of some data, consisting of the hash algorithm and the hash value
			#[derive(Debug, Clone, PartialEq, Eq, Hash)]
			#[non_exhaustive]
			pub enum Hash {
				$(
					#[doc = concat!("[", stringify!([< $algo Hash >]), "]")]
					$algo([< $algo Hash >])
				),+
			}
			ser_display_deser_fromstr!(Hash);

			impl Hash {
				/// Creates a new [Self] from the given algorithm and bytes
				pub fn new(algorithm: HashAlgorithm, bytes: impl Into<Arc<[u8]>>) -> Result<Self, errors::NewHashError> {
					match algorithm {
						$(HashAlgorithm::$algo => [< $algo Hash >]::new(bytes).map(Self::$algo)),+
					}
				}

				/// Creates a new [Self] from the given algorithm and encoded bytes
				pub fn from_encoded(algorithm: HashAlgorithm, bytes: &str) -> Result<Self, errors::HashDecodeError> {
					match algorithm {
						$(HashAlgorithm::$algo => [< $algo Hash >]::from_str(bytes).map(Self::$algo)),+
					}
				}

				/// Creates a new [Self] by digesting the given bytes with the given algorithm
				#[must_use]
				pub fn digest(algorithm: HashAlgorithm, bytes: impl AsRef<[u8]>) -> Self {
					let mut hasher = algorithm.hasher();
					hasher.update(bytes.as_ref());
					hasher.finalize()
				}

				/// Returns the hash algorithm used to create this hash
				#[must_use]
				pub fn algorithm(&self) -> HashAlgorithm {
					match self {
						$(Self::$algo(_) => HashAlgorithm::$algo),+
					}
				}

				/// Returns the hash bytes
				#[must_use]
				pub fn bytes(&self) -> &[u8] {
					match self {
						$(Self::$algo(hash) => hash.0.as_ref()),+
					}
				}

				/// Consumes self and returns the hash bytes
				#[must_use]
				pub fn into_bytes(self) -> Arc<[u8]> {
					match self {
						$(Self::$algo(hash) => hash.0),+
					}
				}

				/// Returns the encoded hash without an algorithm prefix (unlike [Display])
				#[must_use]
				pub fn encoded(&self) -> String {
					match self {
						$(Self::$algo(hash) => hash.to_string()),+
					}
				}
			}

			impl Display for Hash {
				fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
					match self {
						$(Self::$algo(hash) => write!(f, "{}:{hash}", HashAlgorithm::$algo)),+
					}
				}
			}

			impl FromStr for Hash {
				type Err = errors::HashFromStrError;

				fn from_str(s: &str) -> Result<Self, Self::Err> {
					let (algorithm, hash) = s
						.split_once(':')
						.ok_or(errors::HashFromStrErrorKind::InvalidHashFormat)?;

					Ok(Self::from_encoded(algorithm.parse()?, hash)?)
				}
			}
		}
	};
}
algos!(
	/// The BLAKE3 hash algorithm
	#[default]
	Blake3
);

impl HashAlgorithm {
	/// Returns the length, in bytes, of this algorithm's output size
	#[must_use]
	pub const fn output_size(self) -> usize {
		match self {
			Self::Blake3 => blake3::OUT_LEN,
		}
	}
}

#[derive(Default)]
struct Blake3Hasher(blake3::Hasher);

impl Hasher for Blake3Hasher {
	type Output = Blake3Hash;

	fn update(&mut self, input: &[u8]) {
		self.0.update(input);
	}

	fn finalize(self) -> Self::Output {
		let bytes: [u8; _] = self.0.finalize().into();

		Blake3Hash(bytes.into())
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
		UnknownHashAlgorithm(Box<str>),
	}

	/// Errors that can occur when creating a [super::Hash]
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = NewHashError))]
	#[non_exhaustive]
	pub enum NewHashErrorKind {
		/// The hash wasn't of appropriate size
		#[error("hash size was `{actual}` but `{expected}` was expected")]
		InvalidSize {
			/// The size that the hash was
			actual: usize,
			/// The size that was expected
			expected: usize,
		},
	}

	/// Errors that can occur when decoding a [super::Hash]
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = HashDecodeError))]
	#[non_exhaustive]
	pub enum HashDecodeErrorKind {
		/// Error creating a [super::Hash]
		#[error(transparent)]
		NewHash(#[from] NewHashError),

		/// Error parsing the hash value
		#[error("error parsing hash value")]
		InvalidHashValue(#[from] fast32::DecodeError),
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

		/// Error decoding a [super::Hash]
		#[error(transparent)]
		HashDecode(#[from] HashDecodeError),
	}
}
