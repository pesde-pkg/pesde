use std::borrow::Borrow;
use std::ops::Deref;

use serde::Deserialize;
use serde::Serialize;

use super::*;
use crate::bounded::BoundedBTreeSet;
use crate::bounded::BoundedStr;
use crate::hash::HashDecodeError;
use paste::paste;

macro_rules! name_part {
	($name:ident $min:expr=>32) => {
		paste! {
			#[doc = concat!("A validated package ", stringify!([< $name:lower ]>), "(", stringify!($min), "-32 chars, a-z/0-9/_, no leading/trailing _, not all digits)")]
			#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
			pub struct $name(Arc<str>);
			ser_display_deser_fromstr!($name);

			impl $name {
				pub fn as_str(&self) -> &str {
					self
				}
			}

			impl Display for $name {
				fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
					f.write_str(self)
				}
			}

			impl FromStr for $name {
				type Err = [< $name FromStrError >];

				fn from_str(s: &str) -> Result<Self, Self::Err> {
					if !($min..=32).contains(&s.len()) {
						return Err(Self::Err::InvalidLength(s.into()));
					}

					if s.chars().all(|c| c.is_ascii_digit()) {
						return Err(Self::Err::OnlyDigits(s.into()));
					}

					if s.starts_with('_') || s.ends_with('_') {
						return Err(Self::Err::PrePostfixUnderscore(s.into()));
					}

					if !s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
						return Err(Self::Err::InvalidCharacters(s.into()));
					}

					Ok(Self(s.into()))
				}
			}

			#[doc = concat!("Errors that can occur when parsing a [", stringify!($name), "] from str")]
			#[derive(Debug, Error)]
			pub enum [< $name FromStrError >] {
				/// The name is outside the allowed characters: a-z, 0-9, and _
				#[error("name `{0}` contains characters outside a-z, 0-9, and _")]
				InvalidCharacters(Box<str>),

				/// The name contains only digits
				#[error("name `{0}` contains only digits")]
				OnlyDigits(Box<str>),

				/// The name starts or ends with an underscore
				#[error("name `{0}` starts or ends with an underscore")]
				PrePostfixUnderscore(Box<str>),

				/// The name's scope part is not within 3-32 characters long
				#[error("name `{0}` is not within {min}-32 characters long", min = $min)]
				InvalidLength(Box<str>),
			}

			impl Deref for $name {
				type Target = str;

				fn deref(&self) -> &Self::Target {
					&self.0
				}
			}

			#[doc = concat!("The ", stringify!([< $name:lower >]), " id; hash of the ", stringify!([< $name:lower >]))]
			#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
			pub struct [< $name Id >](CurrentHash);
			ser_display_deser_fromstr!([< $name Id >]);

			impl [< $name Id >] {
				/// Creates a new [Self] from a hash
				#[must_use]
				pub fn new(hash: CurrentHash) -> Self {
					Self(hash)
				}

				#[doc = concat!("Creates a new [Self] from a [", stringify!($name), "]")]
				#[must_use]
				pub fn from(name: &$name) -> Self {
					let mut hasher = CurrentHash::hasher();
					hasher.update(name.as_bytes());
					Self::new(hasher.finalize())
				}

				/// The underlying hash
				#[must_use]
				pub fn hash(&self) -> &CurrentHash {
					&self.0
				}
			}

			impl Display for [< $name Id >] {
				fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
					self.0.fmt(f)
				}
			}

			impl FromStr for [< $name Id >] {
				type Err = <CurrentHash as FromStr>::Err;

				fn from_str(s: &str) -> Result<Self, Self::Err> {
					s.parse().map(Self)
				}
			}
		}
	};
}

name_part!(Scope 3=>32);
name_part!(LocalName 1=>32);

/// The Merkle B+Tree that contains scope members along their grants
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeMembersTree(pub CurrentHash);
impl TreeConfig for ScopeMembersTree {
	type Key = ScopeMember;
	type Value = ScopeMemberValue;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
}

/// A key indicating a scope member
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct ScopeMember(pub PublicKey);
impl crate::sealed::Sealed for ScopeMember {}
impl Canonical for ScopeMember {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		cbor_core::map! {
			"key" => self.0.to_string(),
		}
		.write_to(writer)
	}
}

/// A scope member
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeMemberValue {
	/// The grant this scope member possesses
	pub grant: ScopeGrant,
}
impl crate::sealed::Sealed for ScopeMemberValue {}
impl Canonical for ScopeMemberValue {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		cbor_core::map! {
			"grant" => self.grant.as_cbor(),
		}
		.write_to(writer)
	}
}

/// Maximum amount of packages a [ScopeGrant] can have
pub const MAX_GRANT_PACKAGES: usize = 255;

/// The grant a scope member possesses
/// An empty grant means the member can update all packages in the scope
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeGrant(pub BoundedBTreeSet<ScopeGrantEntry, MAX_GRANT_PACKAGES>);

impl ScopeGrant {
	/// Whether this grant allows the member to update this package
	#[must_use]
	pub fn covers(&self, package: &LocalNameId) -> bool {
		if self.0.is_empty() {
			return true;
		}

		self.0.contains(package)
	}

	pub(crate) fn as_cbor(&self) -> cbor_core::Value<'_> {
		cbor_core::Value::Array(self.0.iter().map(|e| e.pkg.0.to_string().into()).collect())
	}
}

hash_with_plaintext!(
	/// An entry within a [ScopeGrant]
	ScopeGrantEntry ord true {
		/// The package id this grant is for
		hash pkg: LocalNameId,
		/// The plaintext name of this grant
		plaintext plaintext: Option<LocalName>,
	},
	ScopeGrantEntryError where |pkg: &LocalNameId, plaintext: &Option<LocalName>|
		plaintext.as_ref().is_none_or(|plaintext| CurrentHash::digest(plaintext.as_bytes()) == pkg.0)
			.ok_or(ScopeGrantEntryError)
);
/// The plaintext hash of the entry was invalid
#[derive(Debug, Clone, Copy, Error)]
#[error("invalid plaintext hash")]
pub struct ScopeGrantEntryError;

/// The Merkle B+Tree that contains package versions along their state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageVersionsTree(pub CurrentHash);
impl TreeConfig for PackageVersionsTree {
	type Key = PackageVersion;
	type Value = PackageVersionValue;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
}

/// A package version, i.e. its local id and version
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageVersion(pub LocalNameId, pub PesdeStyleVersion);
ser_display_deser_fromstr!(PackageVersion);
impl crate::sealed::Sealed for PackageVersion {}
impl Canonical for PackageVersion {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		cbor_core::map! {
			"pkg" => self.0.to_string(),
			"version" => self.1.to_string(),
		}
		.write_to(writer)
	}
}

impl Display for PackageVersion {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{}", self.0, self.1)
	}
}

impl FromStr for PackageVersion {
	type Err = PackageVersionFromStr;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (pkg, version) = s
			.split_once('@')
			.ok_or_else(|| Self::Err::InvalidFormat(s.into()))?;

		Ok(Self(pkg.parse().map(LocalNameId)?, version.parse()?))
	}
}

/// The value of a published package version.
/// Monitors must ensure archive_hash is never changed, unlike the mutable [Self::yank_state]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersionValue {
	/// The hash of the archive containing the package's contents
	pub archive_hash: Hash,
	/// The version's yank status
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub yank_state: Option<VersionYankState>,
}
impl crate::sealed::Sealed for PackageVersionValue {}
impl Canonical for PackageVersionValue {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		cbor_core::map! {
			"archive_hash" => self.archive_hash.to_string(),
			"yank_state" => self.yank_state.as_ref().map_or(const { cbor_core::Value::null() }, |s| cbor_core::Value::TextString(s.to_string().into())),
		}.write_to(writer)
	}
}

/// A yank state of a version. In the case of an admin yank, only an admin is able to unyank it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionYankState {
	/// The package is yanked with a normal yank and can be accessed if it has been observed
	Yanked,
	/// The package has been yanked by an admin; it is no longer accessible
	AdminYanked,
}
ser_display_deser_fromstr!(VersionYankState);

impl Display for VersionYankState {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Yanked => f.write_str("yanked"),
			Self::AdminYanked => f.write_str("admin_yanked"),
		}
	}
}

impl FromStr for VersionYankState {
	type Err = VersionYankStateFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"yanked" => Ok(Self::Yanked),
			"admin_yanked" => Ok(Self::AdminYanked),
			s => Err(VersionYankStateFromStrError::Unknown(s.into())),
		}
	}
}

/// The Merkle B+Tree that contains package deprecations along their reasons
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageDeprecationsTree(pub CurrentHash);
impl TreeConfig for PackageDeprecationsTree {
	type Key = PackageDeprecation;
	type Value = PackageDeprecationValue;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
}

/// Maximum length, in characters, of a deprecation reason
pub const MAX_REASON_LEN: usize = 255;

/// A package deprecation, i.e. a local name id
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageDeprecation(pub LocalNameId);
impl crate::sealed::Sealed for PackageDeprecation {}
impl Canonical for PackageDeprecation {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		cbor_core::map! {
			"pkg" => self.0.to_string(),
		}
		.write_to(writer)
	}
}

/// The value of a package deprecation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDeprecationValue {
	/// The reason of the deprecation
	pub reason: PackageDeprecationReason,
}
impl crate::sealed::Sealed for PackageDeprecationValue {}
impl Canonical for PackageDeprecationValue {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		cbor_core::map! {
			"reason_hash" => self.reason.hash.to_string(),
		}
		.write_to(writer)
	}
}

hash_with_plaintext!(
	/// The reason of the deprecation
	PackageDeprecationReason {
		/// The hash of the reason
		hash hash: Hash,
		/// The plaintext of the reason
		plaintext plaintext: BoundedStr<MAX_REASON_LEN>,
	},
	PackageDeprecationReasonError where |hash: &Hash, plaintext: &str| {
		if !plaintext.is_empty() && Hash::digest(hash.algorithm(), plaintext) != *hash {
			return Err(PackageDeprecationReasonError::InvalidHash);
		}

		if *hash == Hash::digest(hash.algorithm(), []) {
			return Err(PackageDeprecationReasonError::IsEmpty);
		}

		Ok(())
	}
);
/// Errors that can occur when making a deprecation reason
#[derive(Debug, Clone, Copy, Error)]
pub enum PackageDeprecationReasonError {
	/// The plaintext hash didn't match
	#[error("invalid plaintext hash")]
	InvalidHash,

	/// The hash was of an empty value
	#[error("reason mustn't be empty")]
	IsEmpty,
}

macro_rules! hash_with_plaintext {
	(
		$(#[$meta:meta])*
		$struct_name:ident $(ord $ord:literal)? {
			$(#[$hash_meta:meta])*
			hash $hash_name:ident: $hash_ty:ty,
			$(#[$plaintext_meta:meta])*
			plaintext $plaintext_name:ident: $plaintext_ty:ty,
		},
		$err_ty:ty where $check:expr
	) => {
		$(#[$meta])*
		#[derive(Debug, Clone, Serialize)]
		pub struct $struct_name {
			$hash_name: $hash_ty,
			$plaintext_name: $plaintext_ty,
		}

		impl $struct_name {
			/// Creates a new [Self]
			pub fn new($hash_name: $hash_ty, $plaintext_name: $plaintext_ty) -> Result<Self, $err_ty> {
				($check)(&$hash_name, &$plaintext_name).map(|_| Self {
					$hash_name,
					$plaintext_name,
				})
			}

			$(#[$hash_meta])*
			pub fn $hash_name(&self) -> &$hash_ty {
				&self.$hash_name
			}

			$(#[$plaintext_meta])*
			pub fn $plaintext_name(&self) -> &$plaintext_ty {
				&self.$plaintext_name
			}
		}

		$(hash_with_plaintext!(@ord $ord, $struct_name, $hash_name);)?

		impl PartialEq for $struct_name {
			fn eq(&self, other: &Self) -> bool {
				self.$hash_name.eq(&other.$hash_name)
			}
		}
		impl Eq for $struct_name {}

		impl Borrow<$hash_ty> for $struct_name {
			fn borrow(&self) -> &$hash_ty {
				&self.$hash_name
			}
		}

		impl<'de> Deserialize<'de> for $struct_name {
			fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
			where
				D: serde::Deserializer<'de>
			{
				#[derive(Deserialize)]
				struct Raw {
					$hash_name: $hash_ty,
					$plaintext_name: $plaintext_ty,
				}

				let raw = Raw::deserialize(deserializer)?;
				Self::new(raw.$hash_name, raw.$plaintext_name).map_err(serde::de::Error::custom)
			}
		}
	};
	(@ord $ord:literal, $struct_name:ident, $hash_name:ident) => {
		impl Ord for $struct_name {
			fn cmp(&self, other: &Self) -> std::cmp::Ordering {
				self.$hash_name.cmp(&other.$hash_name)
			}
		}

		impl PartialOrd for $struct_name {
			fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
				Some(self.cmp(other))
			}
		}
	}
}
use hash_with_plaintext;

/// Errors that can occur when parsing a [PackageVersion] from str
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PackageVersionFromStr {
	/// The value wasn't in format `name@version`
	#[error("value `{0}` wasn't in format of `name@version`")]
	InvalidFormat(Box<str>),

	/// An error occurred while parsing the local name id
	#[error("error parsing local name")]
	ParseLocalNameId(#[from] HashDecodeError),

	/// An error occurred while parsing the version
	#[error("error parsing version")]
	ParseVersion(#[from] PesdeStyleVersionFromStrError),
}

/// Errors that can occur when parsing a [VersionYankState] from str
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VersionYankStateFromStrError {
	/// The value was unknown
	#[error("value `{0}` isn't a known yank state")]
	Unknown(Box<str>),
}
