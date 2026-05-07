//! Signatures and public keys

use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use base64::Engine as _;

use crate::ser_display_deser_fromstr;

/// A key kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyKind {
	/// An SSH key
	Ssh(SshKeyKind),
}
ser_display_deser_fromstr!(KeyKind);

impl Display for KeyKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			KeyKind::Ssh(ssh_kind) => write!(f, "ssh-{ssh_kind}"),
		}
	}
}

impl FromStr for KeyKind {
	type Err = errors::KeyKindParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if let Some(ssh_kind) = s.strip_prefix("ssh-") {
			Ok(Self::Ssh(ssh_kind.parse()?))
		} else {
			Err(errors::KeyKindParseErrorKind::UnknownKeyKind(s.to_string()).into())
		}
	}
}

impl Default for KeyKind {
	fn default() -> Self {
		Self::Ssh(Default::default())
	}
}

/// An SSH key kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SshKeyKind {
	/// An Ed25519 key
	#[default]
	Ed25519,
}
ser_display_deser_fromstr!(SshKeyKind);

impl Display for SshKeyKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SshKeyKind::Ed25519 => write!(f, "ed25519"),
		}
	}
}

impl FromStr for SshKeyKind {
	type Err = errors::SshKeyKindParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"ed25519" => Ok(Self::Ed25519),
			_ => Err(errors::SshKeyKindParseErrorKind::UnknownSshKeyKind(s.to_string()).into()),
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
		let (kind, data) = s
			.split_once(' ')
			.ok_or(errors::PublicKeyParseErrorKind::InvalidFormat)?;

		Ok(Self {
			kind: kind.parse()?,
			data: base64::engine::general_purpose::STANDARD_NO_PAD
				.decode(data)?
				.into(),
		})
	}
}

impl PublicKey {
	/// Constructs a new public key
	#[must_use]
	pub fn new(kind: KeyKind, data: impl Into<Arc<[u8]>>) -> Self {
		Self {
			kind,
			data: data.into(),
		}
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SignatureKind {
	/// An SSH signature
	Ssh(SshSignatureKind),
}
ser_display_deser_fromstr!(SignatureKind);

impl Display for SignatureKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SignatureKind::Ssh(ssh_kind) => write!(f, "ssh-{ssh_kind}"),
		}
	}
}

impl FromStr for SignatureKind {
	type Err = errors::SignatureKindParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if let Some(ssh_kind) = s.strip_prefix("ssh-") {
			Ok(Self::Ssh(ssh_kind.parse()?))
		} else {
			Err(errors::SignatureKindParseErrorKind::UnknownSignatureKind(s.to_string()).into())
		}
	}
}

impl Default for SignatureKind {
	fn default() -> Self {
		Self::Ssh(Default::default())
	}
}

/// An SSH signature kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SshSignatureKind {
	/// An Ed25519 signature
	#[default]
	Ed25519,
}
ser_display_deser_fromstr!(SshSignatureKind);

impl Display for SshSignatureKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SshSignatureKind::Ed25519 => write!(f, "ed25519"),
		}
	}
}

impl FromStr for SshSignatureKind {
	type Err = errors::SshSignatureKindParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"ed25519" => Ok(Self::Ed25519),
			_ => Err(
				errors::SshSignatureKindParseErrorKind::UnknownSshSignatureKind(s.to_string())
					.into(),
			),
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
		let (kind, data) = s
			.split_once(' ')
			.ok_or(errors::SignatureParseErrorKind::InvalidFormat)?;

		Ok(Self {
			kind: kind.parse()?,
			data: base64::engine::general_purpose::STANDARD_NO_PAD
				.decode(data)?
				.into(),
		})
	}
}

impl Signature {
	/// The namespace used for SSH signatures
	pub const SSH_NAMESPACE: &str = "pesde signature";

	/// Constructs a new signature
	/// Does NOT validate if the data is valid for the given kind. This is left to the caller since validating it would be unnecessarily complex
	#[must_use]
	pub fn new(kind: SignatureKind, data: impl Into<Arc<[u8]>>) -> Self {
		Self {
			kind,
			data: data.into(),
		}
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
			(SignatureKind::Ssh(SshSignatureKind::Ed25519), KeyKind::Ssh(SshKeyKind::Ed25519)) => {
				let Ok(key_data) = public_key
					.data()
					.try_into()
					.map(ssh_key::public::KeyData::Ed25519)
					.map(ssh_key::PublicKey::from)
				else {
					return false;
				};

				// we skip the PEM format to save on some bytes since the signature data isn't meant to be human-readable, and the format is already implied by the signature kind
				use ssh_encoding::Decode as _;
				let Ok(signature) = ssh_key::SshSig::decode(&mut &*self.data) else {
					return false;
				};

				key_data
					.verify(Self::SSH_NAMESPACE, msg, &signature)
					.is_ok()
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

		/// The SSH key kind could not be parsed
		#[error("invalid SSH key kind format")]
		SshKeyKindParseError(#[from] SshKeyKindParseError),
	}

	/// Errors which can occur when parsing an SSH key kind
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = SshKeyKindParseError))]
	pub enum SshKeyKindParseErrorKind {
		/// The SSH key kind is in an invalid format
		#[error("invalid SSH key kind format")]
		InvalidFormat,

		/// The SSH key kind is unknown
		#[error("unknown SSH key kind `{0}`")]
		UnknownSshKeyKind(String),
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
		InvalidBase64(#[from] base64::DecodeError),
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

		/// The SSH signature kind is in an invalid format
		#[error("invalid SSH signature kind format")]
		SshSignatureKindParseError(#[from] SshSignatureKindParseError),
	}

	/// Errors which can occur when parsing an SSH signature kind
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = SshSignatureKindParseError))]
	pub enum SshSignatureKindParseErrorKind {
		/// The SSH signature kind is in an invalid format
		#[error("invalid SSH signature kind format")]
		InvalidFormat,

		/// The SSH signature kind is unknown
		#[error("unknown SSH signature kind `{0}`")]
		UnknownSshSignatureKind(String),
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
		InvalidBase64(#[from] base64::DecodeError),
	}
}
