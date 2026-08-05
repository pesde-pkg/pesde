//! Data models for the registry

use std::convert::Infallible;
use std::fmt::Display;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use merkleberg::Merge;
use semver::Prerelease;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::bounded::Bounded;
use crate::bounded::BoundedBTreeSet;
use crate::bounded::BoundedString;
use crate::hash::Hash;
use crate::hash::HashAlgorithm;
use crate::hash::RawHash;
use crate::names::Name;
use crate::names::Scope;
use crate::ser_display_deser_fromstr;
use crate::signature::PublicKey;
use crate::signature::Signature;

/// Returns a canonical serialisation of the given struct for cryptographic purposes
#[must_use]
pub fn canonical_bytes(data: &impl Serialize) -> Vec<u8> {
	cbor_core::Value::serialized(data)
		.expect("failed to serialise body for signing")
		.encode()
}

/// An entry in a log, at a known leaf position
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry<T> {
	/// The leaf position of this entry
	pub pos: u64,
	/// The time of publishing of this entry
	/// This value is server authoritative because of time sync issues a client provided value would pose
	pub published_at: Timestamp,
	/// The payload of this entry
	pub payload: T,
}

/// A structure that carries an owner key
pub trait WithOwner {
	/// The owner's key
	fn owner(&self) -> &PublicKey;
}

/// An unvalidated record carrying a signature and a signer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnvalidatedSigned<T: WithOwner> {
	/// The signature
	pub sig: Signature,
	/// The person signing this if it isn't the owner
	#[serde(skip_serializing_if = "Option::is_none")]
	pub signer: Option<PublicKey>,
	/// The body
	#[serde(flatten)]
	pub body: T,
}

/// The signed entry was illegal in some way, e.g. the signature didn't match or it doubly specified an owner
#[derive(Debug, Error)]
#[error("the signed entry was illegal")]
pub struct SignedValidationFailed;

/// A validated wrapper over [UnvalidatedSigned], allowing construction only if it's legal
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct Signed<T: WithOwner>(UnvalidatedSigned<T>);

impl<T: WithOwner + Serialize> Signed<T> {
	/// Validates the passed in [UnvalidatedSigned] and returns Some if it's valid
	pub fn new(input: UnvalidatedSigned<T>) -> Result<Self, SignedValidationFailed> {
		if input
			.signer
			.as_ref()
			.is_some_and(|s| s == input.body.owner())
		{
			return Err(SignedValidationFailed);
		}

		if !input.sig.verify(
			input.signer.as_ref().unwrap_or(input.body.owner()),
			&canonical_bytes(&input.body),
		) {
			return Err(SignedValidationFailed);
		}

		Ok(Self(input))
	}

	/// Returns the underlying [UnvalidatedSigned]
	pub fn into_inner(self) -> UnvalidatedSigned<T> {
		self.0
	}
}

impl<'de, T: WithOwner + Serialize + Deserialize<'de>> Deserialize<'de> for Signed<T> {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Self::new(UnvalidatedSigned::deserialize(deserializer)?).map_err(serde::de::Error::custom)
	}
}

/// The body of [GenesisEntry]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisRevision {
	/// The scope name being created
	pub scope_name: Scope,
	/// The owner of the scope
	pub owner: PublicKey,
}

impl WithOwner for GenesisRevision {
	fn owner(&self) -> &PublicKey {
		&self.owner
	}
}

/// The scope creation entry in the registry's global log
pub type GenesisEntry = Entry<Signed<GenesisRevision>>;

/// Maximum amount of packages a [Grant] can have
pub const MAX_GRANT_PACKAGES: usize = 255;

/// Maximum length, in characters, of a deprecation reason
pub const MAX_REASON_LEN: usize = 255;

/// The grant a scope member possesses
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeGrant {
	/// The member can write to all packages
	AllPackages,
	/// The member can write to only the named packages
	Only(BoundedBTreeSet<Name, MAX_GRANT_PACKAGES>),
}

impl ScopeGrant {
	/// Whether this grant allows the member to update this package
	#[must_use]
	pub fn covers(&self, package: &Name) -> bool {
		match self {
			ScopeGrant::AllPackages => true,
			ScopeGrant::Only(packages) => packages.contains(package),
		}
	}
}

/// An operation to the scope issued by a registry admin
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdminOpKind {
	/// The admin is transferring ownership
	TransferOwnership {
		/// The new owner
		new_owner: PublicKey,
	},
	/// A package's yank status is being updated
	SetYanked {
		/// The package being updated
		pkg: Name,
		/// The version being updated
		version: PesdeVersionForRegistry,
		/// Whether it is yanked
		yanked: bool,
	},
}

/// An operation to the scope issued by a regular user
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignedOpKind {
	/// A member is rotating their key
	RotateKey {
		/// The key to be rotated (the one signing this entry)
		old_key: PublicKey,
		/// The key to replace the old key with
		new_key: PublicKey,
		/// The proof of possession of the new key
		new_key_proof: Signature,
		/// A value used to prevent playbacks, this is what [new_key_proof] signs
		nonce: Uuid,
	},
	/// A member is leaving this scope
	MemberLeave {
		/// The member leaving
		member: PublicKey,
	},
	/// The owner is transferring ownership
	TransferOwnership {
		/// The new owner
		new_owner: PublicKey,
		/// The proof of consent of the new owner
		new_owner_consent: Signature,
		/// A value used to prevent playbacks, this is what [new_owner_consent] signs
		nonce: Uuid,
	},
	/// A member is being added
	AddMember {
		/// The new member's key
		member: PublicKey,
		/// The grant they're being added with
		grant: ScopeGrant,
		/// The proof of consent of the new member
		consent: Signature,
		/// A value used to prevent playbacks, this is what [consent] signs
		nonce: Uuid,
	},
	/// The owner is changing a member's grant
	UpdateMemberGrant {
		/// The member's key
		member: PublicKey,
		/// The new grant
		grant: ScopeGrant,
	},
	/// The owner is removing a member
	RemoveMember {
		/// The member being removed
		member: PublicKey,
	},
	/// A new version of a package is being published
	PublishVersion {
		/// The package being published
		pkg: Name,
		/// The version being published
		version: PesdeVersionForRegistry,
		/// The hash of manifest being published
		manifest: Hash,
		/// The hash of the archive being published
		archive_hash: Hash,
	},
	/// A package's yank status is being updated
	SetYanked {
		/// The package being updated
		pkg: Name,
		/// The version being updated
		version: PesdeVersionForRegistry,
		/// Whether it is yanked
		yanked: bool,
	},
	/// A package's deprecation status is being updated
	SetDeprecation {
		/// The package being updated
		pkg: Name,
		/// The reason this package is deprecated
		reason: BoundedString<MAX_REASON_LEN>,
	},
}

/// An operation to the scope
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op_kind", rename_all = "snake_case")]
pub enum ScopeOp {
	/// An entry issued by a normal user
	Signed(Signed<ScopeEntryBody<SignedOpKind>>),
	/// An entry issued by the registry admin
	Admin(ScopeEntryBody<AdminOpKind>),
}

/// The body of [ScopeOp]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeEntryBody<Op> {
	/// The name of the scope
	pub scope_name: Scope,
	/// The scope's owner
	pub scope_owner: PublicKey,
	/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[PublicKey], [ScopeGrant]>
	pub scope_members_root: RawHash,
	/// The hash of the previous entry in this scope's log
	pub prev_hash: RawHash,
	/// The root of the tree holding package versions. [merkle_bplustree::MerkleBPlusTree]<[PackageVersion], [PackageVersionState]>
	pub versions_root: RawHash,
	/// The root of the tree holding package deprecations. [merkle_bplustree::MerkleBPlusTree]<[Name], [BoundedString<MAX_REASON_LEN>]>
	pub deprecations_root: RawHash,
	/// The operation this entry carries
	pub op: Op,
}

impl WithOwner for ScopeEntryBody<SignedOpKind> {
	fn owner(&self) -> &PublicKey {
		&self.scope_owner
	}
}

/// An entry in the scope's chain
pub type ScopeChainEntry = Entry<ScopeOp>;

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
	pre: Prerelease,
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
			pre: semver_version.pre,
		})
	}
}

/// Maximum length, in characters, of a serialised version
pub const MAX_VERSION_LEN: usize = 255;

/// A [PesdeStyleVersion] with a maximum length
pub type PesdeVersionForRegistry = Bounded<PesdeStyleVersion, MAX_VERSION_LEN>;

/// The key to the map a [ScopeEntryBody::versions_root] points to. Pair of package name and version
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageVersion {
	/// The package name this keys
	pub name: Name,
	/// The package version this keys
	pub version: PesdeStyleVersion,
}
ser_display_deser_fromstr!(PackageVersion);

impl Display for PackageVersion {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{}", self.name, self.version)
	}
}

/// Errors that can occur when parsing a [PackageVersion] from str
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PackageVersionFromStrError {
	/// The input string wasn't in the form of `name@version`
	#[error("`{0}` can't be parsed as `name@version`")]
	BadInput(Box<str>),

	/// The name was invalid
	#[error("failed to parse name")]
	MalformedName(#[from] crate::names::errors::PackageNameError),

	/// The version was invalid
	#[error("failed to parse version")]
	MalformedVersion(#[from] PesdeStyleVersionFromStrError),
}

impl FromStr for PackageVersion {
	type Err = PackageVersionFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some((name, version)) = s.split_once('@') else {
			return Err(Self::Err::BadInput(s.into()));
		};

		Ok(Self {
			name: name.parse()?,
			version: version.parse()?,
		})
	}
}

/// The value to the map a [ScopeEntryBody::versions_root] points to.
/// Monitors must ensure archive_hash is never changed, unlike the mutable [Self::yank_state]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersionState {
	/// The hash of the archive containing the package's contents
	pub archive_hash: Hash,
	/// The version's yank status
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub yank_state: Option<VersionYankState>,
}

/// A yank state of a version. In the case of an admin yank, only an admin is able to unyank it
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionYankState {
	/// The package is yanked with a normal yank and can be accessed if it has been observed
	Yanked,
	/// The package has been yanked by an admin; it is no longer accessible
	AdminYanked,
}

/// The response of a log head endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogHeadResponse {
	/// The accumulator of the log
	pub accumulator: MmrAccumulator,
	/// The MMR's current size
	pub mmr_size: u64,
	/// The consistency proof paths
	pub proof_paths: Vec<Vec<<CurrentMmrMerge as Merge>::Item>>,
}

/// The response of a log inclusion endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogInclusionProofResponse {
	/// The proof path from the entry to the peaks
	pub proof: Vec<<CurrentMmrMerge as Merge>::Item>,
}

/// A MMR accumulator
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MmrAccumulator {
	/// The hash algorithm used for all peaks
	pub algorithm: HashAlgorithm,
	/// The peak hashes
	pub peaks: Arc<[RawHash]>,
}

const LEAF_DOMAIN: u8 = 0x00;
const NODE_DOMAIN: u8 = 0x01;

// TODO: remove this once adt_const_params is stable
#[doc(hidden)]
pub trait THashAlgorithm {
	const ALGORITHM: HashAlgorithm;
}

#[doc(hidden)]
#[derive(Debug)]
pub struct Blake3Hash;
impl THashAlgorithm for Blake3Hash {
	const ALGORITHM: HashAlgorithm = HashAlgorithm::Blake3;
}

/// The current hash algorithm used by the registry
pub const CURRENT_HASH_ALGORITHM: HashAlgorithm = HashAlgorithm::Blake3;

/// The [Merge] implementation using the [CURRENT_HASH_ALGORITHM]
pub type CurrentMmrMerge = MmrMerge<Blake3Hash>;

#[doc(hidden)]
#[derive(Debug)]
pub struct MmrMerge<A: THashAlgorithm>(PhantomData<A>);

impl<A: THashAlgorithm> Merge for MmrMerge<A> {
	type Item = RawHash;
	type Error = Infallible;

	fn leaf_hash(data: &[u8]) -> Result<Self::Item, Self::Error> {
		let mut hasher = A::ALGORITHM.hasher();
		hasher.update(&[LEAF_DOMAIN]);
		hasher.update(data);
		Ok(hasher.finalize().into_hash())
	}

	fn merge_pos(
		pos: u64,
		left: &Self::Item,
		right: &Self::Item,
	) -> Result<Self::Item, Self::Error> {
		let mut hasher = A::ALGORITHM.hasher();
		hasher.update(&[NODE_DOMAIN]);
		hasher.update(&pos.to_be_bytes());
		hasher.update(left.as_bytes());
		hasher.update(right.as_bytes());
		Ok(hasher.finalize().into_hash())
	}
}
