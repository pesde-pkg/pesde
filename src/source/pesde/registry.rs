//! Data models for the registry

use std::collections::BTreeMap;
use std::marker::PhantomData;

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
	/// The body being signed
	pub body: T,
	/// The signature over the canonical serialisation of body
	pub sig: Signature,
}

impl<T: Serialize> SignedEntry<T> {
	/// Verifies the signature of this entry against the given public key
	#[must_use]
	pub fn verify(&self, public_key: &PublicKey) -> bool {
		self.sig.verify(public_key, &canonical_bytes(&self.body))
	}
}

/// The sequence number of a registry entry
/// This number is a globally increasing number in the log
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntrySeq(pub u64);

/// The sequence number of a scope entry
/// This number is a per-scope increasing number, unlike [EntrySeq] which is globally increasing
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeSeq(pub u64);

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
	/// The sequence number of this entry within the scope
	pub scope_seq: ScopeSeq,
	/// The identity of the author
	pub author_identity: IdentityId,
	/// The payload of this entry
	pub payload: ScopeEntryPayload,
}

/// The body of a RegisterIdentity entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterIdentityBody {
	/// The identity ID, which must equal Hash(public_key)
	/// Stored separately to ensure new hash algorithms can be adopted in the future without creating conflicts
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
	/// The globally unique sequence number of this entry
	pub seq: EntrySeq,
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
		proof: ConsistencyProof<Sha256Merge>,
	},
}

/// The response of the log head endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogHeadResponse {
	/// The sequence number of the head entry in the log
	pub seq: EntrySeq,
	/// The accumulator of the head entry in the log, as a list of hashes
	pub accumulator: Vec<Hash>,
	/// The MMR state
	pub state: LogHeadResponseState,
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
/// A [HashAlgorithm::Sha256] Merge implementation
pub type Sha256Merge = HashAlgorithmMerge<Sha256Hash>;

#[doc(hidden)]
#[derive(Debug)]
pub struct HashAlgorithmMerge<H: THashAlgorithm>(PhantomData<H>);

impl<H: THashAlgorithm> Merge for HashAlgorithmMerge<H> {
	type Item = Hash;
	type Error = errors::HashAlgorithmMergeError;

	fn leaf_hash(data: &[u8]) -> Result<Self::Item, Self::Error> {
		let mut hasher = H::ALGORITHM.hasher();
		hasher.update(&[LEAF_DOMAIN]);
		hasher.update(data);
		Ok(Hash::new(H::ALGORITHM, hasher.finalize()).unwrap())
	}

	fn merge_pos(
		pos: u64,
		left: &Self::Item,
		right: &Self::Item,
	) -> Result<Self::Item, Self::Error> {
		match (left.algorithm(), right.algorithm()) {
			(v, _) if v != H::ALGORITHM => Err(errors::HashAlgorithmMergeErrorKind::Invalid {
				expected: H::ALGORITHM,
				actual: v,
			}
			.into()),
			(l, r) if l != r => Err(errors::HashAlgorithmMergeErrorKind::Mismatch(l, r).into()),

			_ => {
				let mut hasher = H::ALGORITHM.hasher();
				hasher.update(&[NODE_DOMAIN]);
				hasher.update(&pos.to_be_bytes());
				hasher.update(left.hash());
				hasher.update(right.hash());
				Ok(Hash::new(H::ALGORITHM, hasher.finalize()).unwrap())
			}
		}
	}
}

/// Errors that can occur when interacting with registry structures
pub mod errors {
	use thiserror::Error;

	use crate::hash::HashAlgorithm;

	/// Errors that can occur when working with HashAlgorithm based Merge
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = HashAlgorithmMergeError))]
	pub enum HashAlgorithmMergeErrorKind {
		/// The hash algorithms are mismatched
		#[error("got algorithm `{}` as well as `{}`", .0, .1)]
		Mismatch(HashAlgorithm, HashAlgorithm),

		/// The hash algorithm is invalid
		#[error("algorithm `{actual}` isn't the expected `{expected}`")]
		Invalid {
			/// The algorithm that was expected
			expected: HashAlgorithm,
			/// The algorithm that was given
			actual: HashAlgorithm,
		},
	}
}
