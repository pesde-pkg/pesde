use std::fmt::Debug;

use crate::source::pesde::registry::*;
use serde::{Deserialize, Serialize};

/// A MMR accumulator
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MmrAccumulator {
	/// The peak hashes
	pub peaks: Arc<[CurrentHash]>,
}

/// The response of a log head endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogHeadResponse {
	/// The accumulator of the log
	pub accumulator: MmrAccumulator,
	/// The MMR's current size
	pub mmr_size: u64,
	/// The consistency proof paths
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub consistency_proof: Vec<Vec<<CurrentMerkleHasher as Merge>::Item>>,
}

/// The response of a log entry endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogEntryResponse<P: EntryPayload> {
	/// The entry
	pub entry: Entry<P>,
	/// The proof path from the entry to the peaks, included if `at_size` is specified in the query
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inclusion_proof: Vec<<CurrentMerkleHasher as Merge>::Item>,
}

/// The response of the scope state endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct ScopeStateResponse {
	/// The owner of the scope
	pub owner: PublicKey,
	/// [ScopeMembersTree]
	pub scope_members_root: ScopeMembersTree,
	/// [PackageVersionsTree]
	pub package_versions_root: PackageVersionsTree,
	/// [PackageDeprecationsTree]
	pub package_deprecations_root: PackageDeprecationsTree,
}

/// The response of the tree entry endpoint
#[derive(Debug, Serialize, Deserialize)]
#[serde(
	tag = "kind",
	bound(
		serialize = "T::Key: Serialize, T::Value: Serialize",
		deserialize = "T::Key: serde::Deserialize<'de>, T::Value: serde::Deserialize<'de>"
	)
)]
pub enum TreeEntryEndpointResponse<T: TreeConfig>
where
	T::Key: Debug,
	T::Value: Debug,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
{
	/// The entry exists
	Included {
		/// Proof that the entry exists
		proof: merkle_bplustree::proof::inclusion::InclusionProof<T>,
		/// The entry's value
		value: T::Value,
	},
	/// The entry doesn't exist
	Excluded {
		/// Proof that the entry doesn't exist
		proof: merkle_bplustree::proof::exclusion::ExclusionProof<T>,
	},
}
