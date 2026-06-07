//! Signatures and public keys

use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use base64::Engine as _;

use crate::ser_display_deser_fromstr;

/// A key kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum KeyKind {
	/// An Ed25519 key
	#[default]
	Ed25519,
}
ser_display_deser_fromstr!(KeyKind);

impl KeyKind {
	/// Returns the size of the key data in bytes
	#[must_use]
	pub fn size(self) -> usize {
		match self {
			KeyKind::Ed25519 => 32,
		}
	}
}

impl Display for KeyKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			KeyKind::Ed25519 => write!(f, "ed25519"),
		}
	}
}

impl FromStr for KeyKind {
	type Err = errors::KeyKindParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"ed25519" => Ok(Self::Ed25519),
			_ => Err(errors::KeyKindParseErrorKind::UnknownKeyKind(s.to_string()).into()),
		}
	}
}

/// A public key
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublicKey {
	kind: KeyKind,
	data: Arc<[u8]>,
}
ser_display_deser_fromstr!(PublicKey);

impl Display for PublicKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{} {}",
			self.kind,
			base64::engine::general_purpose::STANDARD_NO_PAD.encode(&self.data)
		)
	}
}

impl FromStr for PublicKey {
	type Err = errors::PublicKeyParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (kind, key) = s
			.split_once(' ')
			.ok_or(errors::PublicKeyParseErrorKind::InvalidFormat)?;

		let kind: KeyKind = kind.parse()?;
		let mut data = vec![0; kind.size()];
		let len = base64::engine::general_purpose::STANDARD_NO_PAD.decode_slice(key, &mut data)?;
		if len != data.len() {
			return Err(errors::PublicKeyParseErrorKind::InvalidFormat.into());
		}

		Self::new(kind, data).ok_or_else(|| errors::PublicKeyParseErrorKind::InvalidFormat.into())
	}
}

impl PublicKey {
	/// Constructs a new public key
	#[must_use]
	pub fn new(kind: KeyKind, data: impl Into<Arc<[u8]>>) -> Option<Self> {
		let data = data.into();
		if data.len() != kind.size() {
			return None;
		}

		Some(Self { kind, data })
	}

	/// Returns the kind of key
	#[must_use]
	pub fn kind(&self) -> KeyKind {
		self.kind
	}

	/// Returns the raw key data
	#[must_use]
	pub fn data(&self) -> &[u8] {
		&self.data
	}
}

/// A signature kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SignatureKind {
	/// An Ed25519 signature in the SSH signature (SSHSIG) format, using SHA-512
	#[default]
	SshEd25519Sha512,
}
ser_display_deser_fromstr!(SignatureKind);

impl SignatureKind {
	/// Returns the size of the signature data in bytes
	#[must_use]
	pub fn size(self) -> usize {
		match self {
			SignatureKind::SshEd25519Sha512 => 64,
		}
	}
}

impl Display for SignatureKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SignatureKind::SshEd25519Sha512 => write!(f, "ssh-ed25519-sha512"),
		}
	}
}

impl FromStr for SignatureKind {
	type Err = errors::SignatureKindParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"ssh-ed25519-sha512" => Ok(Self::SshEd25519Sha512),
			_ => {
				Err(errors::SignatureKindParseErrorKind::UnknownSignatureKind(s.to_string()).into())
			}
		}
	}
}

/// A signature
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature {
	kind: SignatureKind,
	data: Arc<[u8]>,
}
ser_display_deser_fromstr!(Signature);

impl Display for Signature {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{} {}",
			self.kind,
			// TODO: decide on an engine. STANDARD is the most common since it is how most SSH signatures are represented, but it includes padding which is unnecessary
			// STANDARD_NO_PAD is the same without the padding, but it is less common and may be less recognizable to users
			// URL_SAFE variants are also available, but they're the least recognizable
			base64::engine::general_purpose::STANDARD_NO_PAD.encode(&self.data)
		)
	}
}

impl FromStr for Signature {
	type Err = errors::SignatureParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (kind, signature) = s
			.split_once(' ')
			.ok_or(errors::SignatureParseErrorKind::InvalidFormat)?;

		let kind: SignatureKind = kind.parse()?;
		let mut data = vec![0; kind.size()];
		let len =
			base64::engine::general_purpose::STANDARD_NO_PAD.decode_slice(signature, &mut data)?;
		if len != data.len() {
			return Err(errors::SignatureParseErrorKind::InvalidFormat.into());
		}

		Self::new(kind, data).ok_or_else(|| errors::SignatureParseErrorKind::InvalidFormat.into())
	}
}

impl Signature {
	/// The namespace used for SSH signatures
	pub const SSH_NAMESPACE: &str = "pesde signature";

	/// Constructs a new signature
	#[must_use]
	pub fn new(kind: SignatureKind, data: impl Into<Arc<[u8]>>) -> Option<Self> {
		let data = data.into();
		if data.len() != kind.size() {
			return None;
		}

		Some(Self { kind, data })
	}

	/// Returns the kind of signature
	#[must_use]
	pub fn kind(&self) -> SignatureKind {
		self.kind
	}

	/// Returns the data
	#[must_use]
	pub fn data(&self) -> &[u8] {
		&self.data
	}

	/// Verifies the signature
	/// Information about the validity of data (e.g. formats) is not important to this crate, so they are silently ignored by returning false on invalid data
	#[must_use]
	pub fn verify(&self, public_key: &PublicKey, msg: &[u8]) -> bool {
		match (self.kind, public_key.kind()) {
			(SignatureKind::SshEd25519Sha512, KeyKind::Ed25519) => {
				use signature::Verifier as _;

				let Ok(key_data) = public_key
					.data()
					.try_into()
					.map(ssh_key::public::KeyData::Ed25519)
				else {
					return false;
				};

				let Ok(signature) =
					ssh_key::Signature::new(ssh_key::Algorithm::Ed25519, &*self.data)
				else {
					return false;
				};

				let Ok(signed_data) = ssh_key::SshSig::signed_data(
					Self::SSH_NAMESPACE,
					ssh_key::HashAlg::Sha512,
					msg,
				) else {
					return false;
				};

				key_data.verify(&signed_data, &signature).is_ok()
			}
			#[expect(unreachable_patterns)]
			_ => false,
		}
	}
}

/// Errors related to signatures and public keys
pub mod errors {
	use thiserror::Error;

	/// Errors which can occur when parsing a key kind
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = KeyKindParseError))]
	pub enum KeyKindParseErrorKind {
		/// The key kind is in an invalid format
		#[error("invalid key kind format")]
		InvalidFormat,

		/// The key kind is unknown
		#[error("unknown key kind `{0}`")]
		UnknownKeyKind(String),
	}

	/// Errors which can occur when parsing a public key
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = PublicKeyParseError))]
	pub enum PublicKeyParseErrorKind {
		/// The public key is in an invalid format
		#[error("invalid public key format")]
		InvalidFormat,

		/// The key kind is not valid
		#[error("invalid key kind")]
		InvalidKeyKind(#[from] KeyKindParseError),

		/// The key data is not valid base64
		#[error("invalid base64 in public key data")]
		InvalidBase64(#[from] base64::DecodeSliceError),
	}

	/// Errors which can occur when parsing a signature kind
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = SignatureKindParseError))]
	pub enum SignatureKindParseErrorKind {
		/// The signature kind is in an invalid format
		#[error("invalid signature kind format")]
		InvalidFormat,

		/// The signature kind is unknown
		#[error("unknown signature kind `{0}`")]
		UnknownSignatureKind(String),
	}

	/// Errors which can occur when parsing a signature
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = SignatureParseError))]
	pub enum SignatureParseErrorKind {
		/// The signature is in an invalid format
		#[error("invalid signature format")]
		InvalidFormat,

		/// The signature kind is not valid
		#[error("invalid signature kind")]
		InvalidSignatureKind(#[from] SignatureKindParseError),

		/// The signature is not valid base64
		#[error("invalid base64 in signature data")]
		InvalidBase64(#[from] base64::DecodeSliceError),
	}
}
