//! Data models for the registry

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::Arc;

use jiff::Timestamp;
use merkleberg::Merge;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;

use crate::Url;
use crate::bounded::Bounded;
use crate::bounded::BoundedBTreeMap;
use crate::bounded::BoundedString;
use crate::bounded::BoundedVec;
use crate::hash::Hash;
use crate::hash::HashAlgorithm;
use crate::hash::RawHash;
use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::manifest::MAX_AUTHOR_LEN;
use crate::manifest::MAX_AUTHORS;
use crate::manifest::MAX_DESCRIPTION_LEN;
use crate::manifest::MAX_URL_LEN;
use crate::manifest::MAX_VERSION_LEN;
use crate::names::Name;
use crate::names::PackageName;
use crate::names::Scope;
use crate::signature::PublicKey;
use crate::signature::Signature;

/// Returns a canonical serialisation of the given struct for cryptographic purposes
#[must_use]
pub fn canonical_bytes(data: &impl Serialize) -> Vec<u8> {
	cbor_core::Value::serialized(data)
		.expect("failed to serialise body for signing")
		.encode()
}

/// An entry with an associated signature
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedEntry<T> {
	/// The signature over the canonical serialisation of body
	pub sig: Signature,
	/// The body being signed
	body: T,
}

impl<T> SignedEntry<T> {
	/// Returns the body, unvalidated against the signature
	#[must_use]
	pub fn unsafe_body(&self) -> &T {
		&self.body
	}

	/// Returns the body, unvalidated against the signature
	#[must_use]
	pub fn into_unsafe_body(self) -> T {
		self.body
	}
}

impl<T: Serialize> SignedEntry<T> {
	/// Constructs a new signed entry from the signature and body
	pub fn new(sig: Signature, body: T) -> Self {
		Self { sig, body }
	}

	/// Verifies the signature of this entry against the returned public key and returns the body if it matches
	#[must_use]
	pub fn verify(&self, public_key: impl FnOnce(&T) -> &PublicKey) -> Option<&T> {
		self.sig
			.verify(public_key(&self.body), &canonical_bytes(&self.body))
			.then_some(&self.body)
	}

	/// Verifies the signature of this entry against the returned public key and returns the body and signature if it matches
	#[must_use]
	pub fn into_verified(
		self,
		public_key: impl FnOnce(&T) -> &PublicKey,
	) -> Option<(Signature, T)> {
		self.sig
			.verify(public_key(&self.body), &canonical_bytes(&self.body))
			.then_some((self.sig, self.body))
	}

	/// Verifies the signature of this entry against the given public key and returns the body and signature if it matches
	#[must_use]
	pub fn into_verified_external(self, public_key: &PublicKey) -> Option<(Signature, T)> {
		self.sig
			.verify(public_key, &canonical_bytes(&self.body))
			.then_some((self.sig, self.body))
	}
}

/// A UUID which acts as a stable identifier for an identity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "sqlx", sqlx(transparent))]
#[serde(transparent)]
pub struct IdentityId(pub uuid::Uuid);

impl Display for IdentityId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

/// A member of a scope
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeMember {
	/// Write access to every package in the scope
	AllPackages,
	/// Write access to the listed packages
	Packages(BTreeSet<Name>),
}

impl ScopeMember {
	/// Whether this member may write to `package`
	#[must_use]
	pub fn can_write(&self, package: &Name) -> bool {
		match self {
			ScopeMember::AllPackages => true,
			ScopeMember::Packages(packages) => packages.contains(package),
		}
	}
}

/// The manifest for a scope, describing its owner and members
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeManifest {
	/// The sole identity permitted to manage this scope's manifest
	pub owner: IdentityId,
	/// Members with write access to some or all of the scope's packages
	pub members: BTreeMap<IdentityId, ScopeMember>,
}

impl ScopeManifest {
	/// Whether `of` may write to `package`
	#[must_use]
	pub fn can_write(&self, of: &IdentityId, package: &Name) -> bool {
		self.owner == *of
			|| self
				.members
				.get(of)
				.is_some_and(|member| member.can_write(package))
	}
}

/// Dependency specifiers stored by a pesde registry
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegistryDependencySpecifier {
	/// A pesde registry dependency
	Pesde(crate::source::pesde::specifier::RegistryPesdeDependencySpecifier),
	/// A Wally registry dependency
	Wally(crate::source::wally::specifier::RegistryWallyDependencySpecifier),
}

/// Maximum number of dependencies a published package may declare
pub const MAX_DEPENDENCIES: usize = u8::MAX as usize;

/// Maximum length, in characters, of a deprecation reason
pub const MAX_REASON_LEN: usize = 255;

/// The body of a Publish entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishBody {
	/// The name of the package being published
	pub name: Name,
	/// The version of the package being published
	pub version: Bounded<Version, MAX_VERSION_LEN>,
	/// The hash of the archive containing the package contents
	pub archive_hash: Hash,
	/// The description of the package
	#[serde(default, skip_serializing_if = "str::is_empty")]
	pub description: BoundedString<MAX_DESCRIPTION_LEN>,
	/// The license of the package
	#[serde(default, skip_serializing_if = "str::is_empty")]
	pub license: BoundedString<MAX_DESCRIPTION_LEN>,
	/// The authors of the package
	#[serde(default, skip_serializing_if = "<[_]>::is_empty")]
	pub authors: BoundedVec<BoundedString<MAX_AUTHOR_LEN>, MAX_AUTHORS>,
	/// The repository of the package
	pub repository: Option<Bounded<Url, MAX_URL_LEN>>,
	/// The dependencies of the package
	pub dependencies:
		BoundedBTreeMap<Alias, (RegistryDependencySpecifier, DependencyType), MAX_DEPENDENCIES>,
}

/// Whether a yank is being applied or retracted
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "sqlx", sqlx(rename_all = "snake_case"))]
#[serde(rename_all = "snake_case")]
pub enum YankRetraction {
	/// Apply the yank
	Add,
	/// Revoke the yank
	Revoke,
}

/// The body of a Yank entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YankBody {
	/// The name of the package being yanked
	pub name: Name,
	/// The version of the package being yanked
	pub version: Bounded<Version, MAX_VERSION_LEN>,
	/// Whether the version is being yanked or unyanked
	pub action: YankRetraction,
}

/// The body of a Deprecate entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeprecateBody {
	/// The name of the package being deprecated
	pub name: Name,
	/// The reason for deprecation, or empty if retracting
	#[serde(default, skip_serializing_if = "str::is_empty")]
	pub reason: BoundedString<MAX_REASON_LEN>,
}

/// The body of a ManifestUpdate entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeManifestUpdateBody {
	/// The complete new manifest, replacing the previous one entirely
	pub manifest: ScopeManifest,
}

/// The payload of a scope entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeEntryBody<P> {
	/// The scope this entry belongs to
	pub scope: Scope,
	/// The identity of the author
	pub author_identity: IdentityId,
	/// The payload of this entry
	pub payload: P,
}

/// The body of a RegisterIdentity entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterIdentityBody {
	/// The client-generated ID of this identity
	pub identity_id: IdentityId,
	/// The initial public key for this identity
	pub public_key: PublicKey,
}

/// The body of an IdentityRotation entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRotationBody {
	/// The identity being rotated
	pub identity_id: IdentityId,
	/// The new public key to associate with this identity after rotation
	pub new_public_key: PublicKey,
}

/// Rotation of the public key for an existing identity
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRotationEntry {
	/// Signature by the old key, authorising the rotation
	pub old_sig: Signature,
	/// Signature by the new key, proving possession
	pub new_sig: Signature,
	/// The body being signed by both keys
	body: IdentityRotationBody,
}

impl IdentityRotationEntry {
	/// Constructs a new rotation entry from both signatures and the body
	#[must_use]
	pub fn new(old_sig: Signature, new_sig: Signature, body: IdentityRotationBody) -> Self {
		Self {
			old_sig,
			new_sig,
			body,
		}
	}

	/// Returns the body, unvalidated against the signatures
	#[must_use]
	pub fn unsafe_body(&self) -> &IdentityRotationBody {
		&self.body
	}

	/// Verifies both signatures and returns them with the body if they match
	#[must_use]
	pub fn into_verified(
		self,
		old_key: &PublicKey,
	) -> Option<(Signature, Signature, IdentityRotationBody)> {
		let bytes = canonical_bytes(&self.body);
		(self.old_sig.verify(old_key, &bytes)
			&& self.new_sig.verify(&self.body.new_public_key, &bytes))
		.then_some((self.old_sig, self.new_sig, self.body))
	}
}

/// A forced scope ownership transfer done by the registry administrator, without the consent of the previous owner
/// Intended solely for administrative interventions including e.g. squatting or legal disputes
/// This entry should be brought up to the user interactively if encountered during installation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminScopeTransfer {
	/// The scope being transferred
	pub scope: Scope,
	/// The new manifest to install, including the new owner
	pub manifest: ScopeManifest,
}

/// A publish scope entry
pub type PublishScopeEntry = SignedEntry<ScopeEntryBody<PublishBody>>;
/// A yank scope entry
pub type YankScopeEntry = SignedEntry<ScopeEntryBody<YankBody>>;
/// A deprecate scope entry
pub type DeprecateScopeEntry = SignedEntry<ScopeEntryBody<DeprecateBody>>;
/// A manifest-update scope entry
pub type ManifestUpdateScopeEntry = SignedEntry<ScopeEntryBody<ScopeManifestUpdateBody>>;

/// A scope-chained entry of any kind, as it appears in the log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeEntry {
	/// Publishing a new package version
	Publish(PublishScopeEntry),
	/// Yanking of an existing package version
	Yank(YankScopeEntry),
	/// Deprecation of an existing package
	Deprecate(DeprecateScopeEntry),
	/// Complete replacement of the scope manifest
	ManifestUpdate(ManifestUpdateScopeEntry),
}

/// Registration of a new identity, anchoring its initial public key
pub type RegisterIdentityEntry = SignedEntry<RegisterIdentityBody>;
/// A forced scope ownership transfer initiated by the registry operator
pub type AdminScopeTransferEntry = AdminScopeTransfer;

/// An identity-chained entry of any kind, as it appears in the log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityEntry {
	/// Registration of a new identity, anchoring its initial public key
	Register(RegisterIdentityEntry),
	/// Rotation of the public key for an existing identity
	Rotation(IdentityRotationEntry),
}

/// All possible entry payloads in the registry log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryPayload {
	/// A scope-related entry
	Scope(ScopeEntry),
	/// An identity-related entry
	Identity(IdentityEntry),
	/// A forced scope ownership transfer initiated by the registry operator
	AdminScopeTransfer(AdminScopeTransferEntry),
}

/// An entry in the registry log, at a known leaf position
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry<T> {
	/// The leaf position of this entry
	pub pos: u64,
	/// The time of publishing of this entry
	pub published_at: Timestamp,
	/// The payload of this entry
	pub payload: T,
}

/// The response of the package version endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersionResponse {
	/// The publish entry for this version
	pub publish: Entry<PublishScopeEntry>,
	/// The yank entry, present only while the version is currently yanked
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub yank: Option<Entry<YankScopeEntry>>,
}

/// The response of the package versions endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersionsResponse {
	/// The versions
	pub versions: Vec<PackageVersionResponse>,
	/// The total amount of versions
	pub total: u64,
}

/// The response of the package info endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfoResponse {
	/// The package-level deprecation entry, present only while currently deprecated
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub deprecation: Option<Entry<DeprecateScopeEntry>>,
	/// The latest version of this package: the largest non-prereleased, non-yanked version
	/// In case nothing matches, the conditions are ignored as ordered in the text
	pub latest_version: Version,
}

/// A single hit from the package search endpoint
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResultItem {
	/// The name of the package
	pub name: PackageName,
	/// The version of the package
	pub version: Version,
	/// The description of the package
	pub description: String,
	/// The time of publishing of this package
	pub published_at: Timestamp,
}

/// The response of the log head endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogHeadResponse {
	/// The accumulator of the head entry in the log
	pub accumulator: MmrAccumulator,
	/// The MMR's current size
	pub mmr_size: u64,
	/// The consistency proof paths
	pub proof_paths: Vec<Vec<<CurrentMmrMerge as Merge>::Item>>,
}

/// The response of the log inclusion endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct InclusionProofResponse {
	/// The index of the entry the proof is for
	pub index: u64,
	/// The proof path from the entry to the peaks
	pub proof: Vec<<CurrentMmrMerge as Merge>::Item>,
}

/// MMR accumulator
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
pub struct Sha384Hash;
impl THashAlgorithm for Sha384Hash {
	const ALGORITHM: HashAlgorithm = HashAlgorithm::Sha384;
}

/// The current hash algorithm used by the registry
pub const CURRENT_HASH_ALGORITHM: HashAlgorithm = HashAlgorithm::Sha384;

/// The [Merge] implementation using the [CURRENT_HASH_ALGORITHM]
pub type CurrentMmrMerge = MmrMerge<Sha384Hash>;

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
		Ok(hasher.finalize().into())
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
		Ok(hasher.finalize().into())
	}
}
