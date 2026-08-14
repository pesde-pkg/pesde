//! Data models for the registry

use std::convert::Infallible;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::hasher::Hasher;
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
use crate::hash::Blake3Hash;
use crate::hash::Hash;
use crate::hash::Hasher as _;
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

/// An unvalidated record carrying a signature and a signer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnvalidatedSigned<T> {
	/// The signature
	pub sig: Signature,
	/// The person signing this
	pub signer: PublicKey,
	/// The body
	#[serde(flatten)]
	pub body: T,
}

/// The signed entry was illegal in some way, e.g. the signature didn't match
#[derive(Debug, Error)]
#[error("the signed entry was illegal")]
pub struct SignedValidationFailed;

/// A validated wrapper over [UnvalidatedSigned], allowing construction only if it's legal
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct Signed<T>(UnvalidatedSigned<T>);

impl<T: Serialize> Signed<T> {
	/// Validates the passed in [UnvalidatedSigned] and returns Some if it's valid
	pub fn new(input: UnvalidatedSigned<T>) -> Result<Self, SignedValidationFailed> {
		if !input
			.sig
			.verify(&input.signer, &canonical_bytes(&input.body))
		{
			return Err(SignedValidationFailed);
		}

		Ok(Self(input))
	}

	/// Returns the underlying [UnvalidatedSigned]
	pub fn into_inner(self) -> UnvalidatedSigned<T> {
		self.0
	}
}

impl<'de, T: Serialize + Deserialize<'de>> Deserialize<'de> for Signed<T> {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Self::new(UnvalidatedSigned::deserialize(deserializer)?).map_err(serde::de::Error::custom)
	}
}

/// The payload of [GenesisEntry]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeGenesisPayload {
	/// The scope name being created
	pub scope_name: Scope,
	/// The hash of the first entry in the scope's log
	pub first_entry_hash: Hash,
}

/// The scope creation entry in the registry's global log
pub type GenesisEntry = Entry<Signed<ScopeGenesisPayload>>;

/// Maximum amount of packages a [Grant] can have
pub const MAX_GRANT_PACKAGES: usize = 255;

/// Maximum length, in characters, of a deprecation reason
pub const MAX_REASON_LEN: usize = 255;

/// The grant a scope member possesses
/// An empty grant means the member can update all packages in the scope
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeGrant(pub BoundedBTreeSet<Name, MAX_GRANT_PACKAGES>);

impl ScopeGrant {
	/// Whether this grant allows the member to update this package
	#[must_use]
	pub fn covers(&self, package: &Name) -> bool {
		if self.0.is_empty() {
			return true;
		}

		self.0.contains(package)
	}
}

/// An operation to the scope issued by a regular user
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignedOpKind {
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
		/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[ScopeMembersTreeConfig]>
		scope_members_root: CurrentHash,
	},
	/// The owner is changing a member's grant
	UpdateMemberGrant {
		/// The member's key
		member: PublicKey,
		/// The new grant
		grant: ScopeGrant,
		/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[ScopeMembersTreeConfig]>
		scope_members_root: CurrentHash,
	},
	/// A member is rotating their key
	RotateKey {
		/// The key to replace the old key with
		new_key: PublicKey,
		/// The proof of possession of the new key
		new_key_proof: Signature,
		/// A value used to prevent playbacks, this is what [new_key_proof] signs
		nonce: Uuid,
		/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[ScopeMembersTreeConfig]>
		scope_members_root: CurrentHash,
	},
	/// The owner is removing a member
	RemoveMember {
		/// The member being removed. None if the member is removing themselves (signing key is who's leaving)
		#[serde(default, skip_serializing_if = "Option::is_none")]
		member: Option<PublicKey>,
		/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[ScopeMembersTreeConfig]>
		scope_members_root: CurrentHash,
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
	/// A new version of a package is being published
	PublishVersion {
		/// The package being published
		pkg: Name,
		/// The version being published
		version: PesdeVersionForRegistry,
		/// The hash of the archive being published
		archive_hash: Hash,
		/// The root of the tree holding package versions. [merkle_bplustree::MerkleBPlusTree]<[PackageVersionsTreeConfig]>
		versions_root: CurrentHash,
	},
	/// A package's yank status is being updated
	SetYanked {
		/// The package being updated
		pkg: Name,
		/// The version being updated
		version: PesdeVersionForRegistry,
		/// Whether it is yanked
		yanked: bool,
		/// The root of the tree holding package versions. [merkle_bplustree::MerkleBPlusTree]<[PackageVersionsTreeConfig]>
		versions_root: CurrentHash,
	},
	/// A package's deprecation status is being updated
	SetDeprecation {
		/// The package being updated
		pkg: Name,
		/// The reason this package is deprecated
		reason: BoundedString<MAX_REASON_LEN>,
		/// The root of the tree holding package deprecations. [merkle_bplustree::MerkleBPlusTree]<[PackageDeprecationsTreeConfig]>
		deprecations_root: CurrentHash,
	},
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
		/// The root of the tree holding package versions. [merkle_bplustree::MerkleBPlusTree]<[PackageVersionsTreeConfig]>
		versions_root: CurrentHash,
	},
}

/// The body of [ScopeOp]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeEntryBody<Op> {
	/// The name of the scope
	pub scope_name: Scope,
	/// The hash of the previous entry in this scope's log
	pub prev_hash: Hash,
	/// The operation this entry carries
	pub op: Op,
}

/// The payload of an entry in the scope's log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "payload_kind", rename_all = "snake_case")]
pub enum ScopeEntryPayload {
	/// An entry issued by a normal user
	Signed(Signed<ScopeEntryBody<SignedOpKind>>),
	/// An entry issued by the registry admin
	Admin(ScopeEntryBody<AdminOpKind>),
}

/// An entry in the scope's chain
pub type ScopeEntry = Entry<ScopeEntryPayload>;

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

/// The tree config for the Merkle B+Tree [ScopeEntryBody::scope_members_root] points to
pub struct ScopeMembersTreeConfig;
impl TreeConfig for ScopeMembersTreeConfig {
	type Key = PublicKey;
	type Value = ScopeGrant;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
}

/// The tree config for the Merkle B+Tree [ScopeEntryBody::versions_root] points to
pub struct PackageVersionsTreeConfig;
impl TreeConfig for PackageVersionsTreeConfig {
	type Key = PackageVersion;
	type Value = PackageVersionState;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 63>;
}

/// The tree config for the Merkle B+Tree [ScopeEntryBody::deprecations_root] points to
pub struct PackageDeprecationsTreeConfig;
impl TreeConfig for PackageDeprecationsTreeConfig {
	type Key = Name;
	type Value = BoundedString<MAX_REASON_LEN>;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
}

/// The response of a log head endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogHeadResponse {
	/// The accumulator of the log
	pub accumulator: MmrAccumulator,
	/// The MMR's current size
	pub mmr_size: u64,
	/// The consistency proof paths
	pub proof_paths: Vec<Vec<<CurrentMerkleHasher as Merge>::Item>>,
}

/// The response of a log inclusion endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogInclusionProofResponse {
	/// The proof path from the entry to the peaks
	pub proof: Vec<<CurrentMerkleHasher as Merge>::Item>,
}

/// A MMR accumulator
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MmrAccumulator {
	/// The peak hashes
	pub peaks: Arc<[CurrentHash]>,
}

/// The current hash used by the registry
pub type CurrentHash = Blake3Hash;

/// The [Merge] and [Hasher] implementation using the [CurrentHash]
#[derive(Debug)]
pub struct CurrentMerkleHasher;

impl Merge for CurrentMerkleHasher {
	type Item = CurrentHash;
	type Error = Infallible;

	fn leaf_hash(data: &[u8]) -> Result<Self::Item, Self::Error> {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x00]);
		hasher.update(data);
		Ok(hasher.finalize())
	}

	fn merge_pos(
		pos: u64,
		left: &Self::Item,
		right: &Self::Item,
	) -> Result<Self::Item, Self::Error> {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x01]);
		hasher.update(&pos.to_be_bytes());
		hasher.update(left.0.as_ref());
		hasher.update(right.0.as_ref());
		Ok(hasher.finalize())
	}
}

impl<K: Serialize, V: Serialize> Hasher<K, V> for CurrentMerkleHasher {
	type Output = CurrentHash;

	fn empty_hash() -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x10]);
		hasher.finalize()
	}

	fn hash_key(key: &K) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x11]);
		hasher.update(&canonical_bytes(key));
		hasher.finalize()
	}

	fn hash_value(value: &V) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x12]);
		hasher.update(&canonical_bytes(value));
		hasher.finalize()
	}

	fn hash_slot(key: &K, child: &Self::Output) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x13]);
		hasher.update(&canonical_bytes(key));
		hasher.update(child.0.as_ref());
		hasher.finalize()
	}

	fn merge_hashes(a: &Self::Output, b: &Self::Output) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(a.0.as_ref());
		hasher.update(b.0.as_ref());
		hasher.finalize()
	}

	fn hash_leaf(entry_count: usize, merkle_root: &Self::Output) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x14]);
		hasher.update(&(entry_count as u64).to_be_bytes());
		hasher.update(merkle_root.0.as_ref());
		hasher.finalize()
	}

	fn hash_internal(child_count: usize, slots_root: &Self::Output) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x15]);
		hasher.update(&(child_count as u64).to_be_bytes());
		hasher.update(slots_root.0.as_ref());
		hasher.finalize()
	}
}
