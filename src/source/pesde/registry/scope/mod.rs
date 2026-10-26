use std::borrow::Borrow;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::Serialize;

use crate::bounded::BoundedBTreeSet;
use crate::bounded::errors::ParseBoundedError;
use crate::hash::errors::HashDecodeError;
use crate::names::LocalName;
use crate::source::pesde::registry::*;

mod op;
pub use op::*;

/// Consent data without any validity guarantees
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnvalidatedConsent {
	/// A unique value to make this one-use
	pub nonce: Uuid,
	/// The person consenting
	pub consenter: PublicKey,
}

/// Consent that is known to be signed by its consenter for the scope
#[derive(Debug, Clone)]
pub struct Consent<T: Tagged>(UnvalidatedSigned<UnvalidatedConsent>, PhantomData<T>);

impl<T: Tagged> Consent<T> {
	/// Returns Ok if the [ScopeId] and [UnvalidatedConsent] matched what is expected
	pub fn new(
		scope_id: &ScopeId,
		unvalidated: UnvalidatedSigned<UnvalidatedConsent>,
	) -> Result<Self, BadSignature> {
		let UnvalidatedConsent { nonce, consenter } = &unvalidated.body;
		let signed_bytes = cbor_core::map! {
			"tag" => T::TAG,
			"scope_id" => scope_id.to_string(),
			"nonce" => nonce.to_string(),
			"consenter" => consenter.to_string(),
		}
		.encode();

		if !unvalidated.sig.verify(consenter, &signed_bytes) {
			return Err(BadSignature);
		}

		Ok(Self(unvalidated, PhantomData))
	}

	/// Returns the underlying [UnvalidatedSigned]
	#[must_use]
	pub fn inner(&self) -> &UnvalidatedSigned<UnvalidatedConsent> {
		&self.0
	}

	/// Returns the consenter
	#[must_use]
	pub fn consenter(&self) -> &PublicKey {
		&self.inner().body.consenter
	}

	fn as_cbor(&self) -> cbor_core::Value<'_> {
		cbor_core::map! {
			"nonce" => self.inner().body.nonce.to_string(),
			"consenter" => self.inner().body.consenter.to_string(),
			"sig" => self.inner().sig.to_string(),
		}
	}
}

impl<T: Tagged> Serialize for Consent<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.inner().serialize(serializer)
	}
}

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

	fn as_cbor(&self) -> cbor_core::Value<'_> {
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
		plaintext.as_ref().is_none_or(|plaintext| CurrentHash::digest(plaintext.as_str()) == pkg.0)
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

/// An opinionated subset of (Cargo) SemVer.
/// Differences from [Version]:
/// - build metadata is not allowed: it is ambiguous (can't specify it) and overall has little to no purpose
/// - only lowercase ASCII is allowed: while without this requirement versions can be deterministically chosen, they are surprising to users: `1.2.3-hello` is not `1.2.3-Hello`
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PesdeStyleVersion {
	/// [Version::major]
	pub major: u64,
	/// [Version::minor]
	pub minor: u64,
	/// [Version::patch]
	pub patch: u64,
	/// [Version::pre]
	pre: Bounded<Prerelease, 10>,
}
ser_display_deser_fromstr!(PesdeStyleVersion);

impl PesdeStyleVersion {
	/// [Version::pre]
	#[must_use]
	pub fn pre(&self) -> &Prerelease {
		&self.pre
	}
}

impl Display for PesdeStyleVersion {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let pre = if self.pre.is_empty() {
			format_args!("")
		} else {
			format_args!("-{}", self.pre)
		};

		write!(f, "{}.{}.{}{}", self.major, self.minor, self.patch, pre)
	}
}

impl FromStr for PesdeStyleVersion {
	type Err = PesdeStyleVersionFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let semver_version = Version::parse(s)?;
		if !semver_version.build.is_empty() {
			return Err(Self::Err::HasBuildMetadata);
		}

		if semver_version.pre.chars().any(|c| c.is_ascii_uppercase()) {
			return Err(Self::Err::UpperPrerelease);
		}

		Ok(Self {
			major: semver_version.major,
			minor: semver_version.minor,
			patch: semver_version.patch,
			pre: Bounded::new(semver_version.pre)
				.map_err(PesdeStyleVersionFromStrError::PrereleaseLength)?,
		})
	}
}

/// Maximum length, in characters, of a serialised version
pub const MAX_VERSION_LEN: usize = 255;

/// A [PesdeStyleVersion] with a maximum length
pub type PesdeVersionForRegistry = Bounded<PesdeStyleVersion, MAX_VERSION_LEN>;

/// A package version, i.e. its local id and version
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageVersion(pub LocalNameId, pub PesdeVersionForRegistry);
ser_display_deser_fromstr!(PackageVersion);

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

impl Canonical for PackageVersionValue {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		cbor_core::map! {
			"archive_hash" => self.archive_hash.to_string(),
			"yank_state" => self.yank_state.as_ref().map_or(const { cbor_core::Value::null() }, |s| cbor_core::Value::TextString(s.to_string().into())),
		}.write_to(writer)
	}
}

/// A yank state of a version. In the case of an admin yank, only an admin is able to unyank it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "sqlx", sqlx(rename_all = "snake_case"))]
pub enum VersionYankState {
	/// The package is yanked with a normal yank and can be accessed if it has been observed
	Yanked,
	/// The package has been yanked by an admin; it is no longer accessible
	AdminYanked,
}

impl Display for VersionYankState {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Yanked => f.write_str("yanked"),
			Self::AdminYanked => f.write_str("admin_yanked"),
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
		plaintext plaintext: Bounded<Box<str>, MAX_REASON_LEN>,
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

/// Errors that can occur when parsing a [PesdeStyleVersion] from str
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PesdeStyleVersionFromStrError {
	/// The version was not valid SemVer
	#[error("failed to parse input as semver")]
	SemVer(#[from] semver::Error),

	/// The version contained build metadata
	#[error("pesde style versions mustn't contain build metadata")]
	HasBuildMetadata,

	/// The version's prerelease wasn't lowercase
	#[error("pesde style versions' prereleases must be lowercase")]
	UpperPrerelease,

	/// The version's prerelease was too long
	#[error("pesde style versions' prereleases must be shorter")]
	PrereleaseLength(#[source] crate::bounded::errors::TooLongError),
}

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
	ParseVersion(#[from] ParseBoundedError<PesdeStyleVersionFromStrError>),
}
