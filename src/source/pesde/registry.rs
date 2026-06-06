//! Data models for the registry

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::marker::PhantomData;
use std::sync::Arc;

use bitflags::bitflags;
use merkleberg::Merge;
use merkleberg::mmriver::ConsistencyProof;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;

use crate::hash::Hash;
use crate::hash::HashAlgorithm;
use crate::names::Name;
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
}

/// A UUID which acts as a stable identifier for an identity
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IdentityId(pub uuid::Uuid);

bitflags! {
	/// Permissions for a scope member
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct ScopePermission: u64 {
		/// Permission to publish new packages and versions
		const Publish = 1 << 0;
		/// Permission to manage the retirement status of packages (deprecations) and versions (yanks)
		const Retire = 1 << 1;

		/// Unknown permission, added for backwards compatibility
		const _ = !0;
	}
}

/// An entry for a member in the scope manifest
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeMember {
	/// The permissions granted to this member
	pub permissions: ScopePermission,
}

/// The manifest for a scope, describing its owner, members, and their permissions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeManifest {
	/// The sole identity permitted to manage this scope's manifest
	pub owner: IdentityId,
	/// Members with restricted permissions
	pub members: BTreeMap<IdentityId, ScopeMember>,
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

/// The body of a Publish entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishBody {
	/// The name of the package being published
	pub name: Name,
	/// The version of the package being published
	pub version: Version,
	/// The hash of the archive containing the package contents
	pub archive_hash: Hash,
	/* TODO: other fields, e.g. dependencies */
}

/// The body of a Yank entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YankBody {
	/// The name of the package being yanked
	pub name: Name,
	/// The version of the package being yanked
	pub version: Version,
}

/// The body of a Deprecate entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeprecateBody {
	/// The name of the package being deprecated
	pub name: Name,
	/// The reason for deprecation
	pub reason: String,
}

/// The body of a ManifestUpdate entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeManifestUpdateBody {
	/// The complete new manifest, replacing the previous one entirely
	pub manifest: ScopeManifest,
}

/// The payload of a scope entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeEntryPayload {
	/// Publish a new package version
	Publish(PublishBody),
	/// Yank an existing package version
	Yank(YankBody),
	/// Deprecate an existing package
	Deprecate(DeprecateBody),
	/// Replace the scope manifest entirely
	ManifestUpdate(ScopeManifestUpdateBody),
}

/// The body of a scope entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeEntryBody {
	/// The scope this entry belongs to
	pub scope: Scope,
	/// The identity of the author
	pub author_identity: IdentityId,
	/// The payload of this entry
	pub payload: ScopeEntryPayload,
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

/// A scope-chained entry (publish, yank, or manifest update)
pub type ScopeEntry = SignedEntry<ScopeEntryBody>;
/// Registration of a new identity, anchoring its initial public key
pub type RegisterIdentityEntry = SignedEntry<RegisterIdentityBody>;
/// Rotation of the public key for an existing identity
pub type IdentityRotationEntry = SignedEntry<IdentityRotationBody>;
/// A forced scope ownership transfer initiated by the registry operator
pub type AdminScopeTransferEntry = AdminScopeTransfer;

/// All possible entry payloads in the registry log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryPayload {
	/// A scope-chained entry (publish, yank, or manifest update)
	Scope(ScopeEntry),
	/// Registration of a new identity, anchoring its initial public key
	RegisterIdentity(RegisterIdentityEntry),
	/// Rotation of the public key for an existing identity
	IdentityRotation(IdentityRotationEntry),
	/// A forced scope ownership transfer initiated by the registry operator
	AdminScopeTransfer(AdminScopeTransferEntry),
}

/// An entry in the registry log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
	/// The leaf position of this entry
	pub pos: u64,
	/// The payload of this entry
	pub payload: EntryPayload,
}

/// The MMR state as coming from the log head endpoint
#[derive(Debug, Serialize, Deserialize)]
pub enum LogHeadResponseState {
	/// There is only a TOFU MMR size, and no previous state to compare against
	OnlyNewState {
		/// The MMR's size
		mmr_size_to: u64,
	},
	/// There is a previous state, and a consistency proof that can be verified against it
	WithPreviousState {
		/// The consistency proof
		#[serde(flatten)]
		proof: ConsistencyProof<CurrentMmrMerge>,
	},
}

/// The response of the log head endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogHeadResponse {
	/// The accumulator of the head entry in the log
	pub accumulator: MmrAccumulator,
	/// The MMR state
	pub state: LogHeadResponseState,
}

/// MMR accumulator
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MmrAccumulator {
	/// The hash algorithm used for all peaks
	pub algorithm: HashAlgorithm,
	/// The peak hashes
	#[serde(
		serialize_with = "serialize_peaks",
		deserialize_with = "deserialize_peaks"
	)]
	pub peaks: Arc<[Arc<[u8]>]>,
}

fn serialize_peaks<S: serde::Serializer>(peaks: &[Arc<[u8]>], s: S) -> Result<S::Ok, S::Error> {
	peaks
		.iter()
		.map(hex::encode)
		.collect::<Vec<_>>()
		.serialize(s)
}

fn deserialize_peaks<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Arc<[Arc<[u8]>]>, D::Error> {
	Vec::<String>::deserialize(d)?
		.into_iter()
		.map(|s| hex::decode(&s).map(Into::into))
		.collect::<Result<Vec<_>, _>>()
		.map(Into::into)
		.map_err(serde::de::Error::custom)
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
pub struct Sha256Hash;
impl THashAlgorithm for Sha256Hash {
	const ALGORITHM: HashAlgorithm = HashAlgorithm::Sha256;
}

/// The current hash algorithm used by the registry
pub const CURRENT_HASH_ALGORITHM: HashAlgorithm = HashAlgorithm::Sha256;

/// The [Merge] implementation using the [CURRENT_HASH_ALGORITHM]
pub type CurrentMmrMerge = MmrMerge<Sha256Hash>;

#[doc(hidden)]
#[derive(Debug)]
pub struct MmrMerge<A: THashAlgorithm>(PhantomData<A>);

impl<A: THashAlgorithm> Merge for MmrMerge<A> {
	type Item = Arc<[u8]>;
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
		hasher.update(left);
		hasher.update(right);
		Ok(hasher.finalize().into())
	}
}
