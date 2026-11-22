use std::fmt::Debug;

use super::*;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The response of a log head endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogHeadResponse<P: EntryPayload> {
	/// The accumulator of the log
	pub accumulator: Box<[CurrentHash]>,
	/// The latest entry of the log; the log holds `tip.entry.index + 1` entries.
	pub tip: LogTip<P>,
	/// The consistency proof paths, included if `from_size` is specified in the query.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub consistency_proof: Vec<Vec<<CurrentMerkleHasher as Merge>::Item>>,
}

/// The latest entry of a log, proven against the accumulator of the same response.
#[derive(Debug, Serialize, Deserialize)]
pub struct LogTip<P: EntryPayload> {
	/// The entry.
	pub entry: Entry<P>,
	/// The proof path from the entry to the peaks.
	pub inclusion_proof: Vec<<CurrentMerkleHasher as Merge>::Item>,
}

/// The response of a log entry endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LogEntryResponse<P: EntryPayload> {
	/// The entry
	pub entry: Entry<P>,
	/// The proof path from the entry to the peaks, included if `at_size` is specified in the query.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inclusion_proof: Vec<<CurrentMerkleHasher as Merge>::Item>,
}

/// The response of the tree entry endpoint
#[derive(Debug, Serialize, Deserialize)]
#[serde(
	tag = "kind",
	rename_all = "snake_case",
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

/// The response of the tree entries endpoint
#[derive(Debug, Serialize, Deserialize)]
#[serde(
	tag = "kind",
	rename_all = "snake_case",
	bound(
		serialize = "T::Key: Serialize, T::Value: Serialize",
		deserialize = "T::Key: serde::Deserialize<'de>, T::Value: serde::Deserialize<'de>"
	)
)]
pub enum TreeEntriesEndpointResponse<T: TreeConfig>
where
	T::Key: Debug,
	T::Value: Debug,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
{
	/// An entry within the requested range
	Entry {
		/// The key of the entry
		key: T::Key,
		/// The value of the entry
		value: T::Value,
	},
	/// A proof node
	Node(merkle_bplustree::proof::range::RangeProofNode<T>),
}

/// A tree transition operation's result.
#[derive(Debug, Serialize, Deserialize)]
#[serde(
	tag = "kind",
	rename_all = "snake_case",
	bound(
		serialize = "T::Key: Serialize, T::Value: Serialize",
		deserialize = "T::Key: serde::Deserialize<'de>, T::Value: serde::Deserialize<'de>"
	)
)]
pub enum TreeTransitionOperationResult<T: TreeConfig>
where
	T::Key: Debug,
	T::Value: Debug,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
{
	/// A value was set.
	Modified(merkle_bplustree::proof::modify::ModifyProof<T>),
	/// A value was removed.
	Removed {
		proof: merkle_bplustree::proof::delete::DeleteProof<T>,
		value_hash: CurrentHash,
	},
	/// A value was not removed because the key did not exist.
	NotRemoved(merkle_bplustree::proof::exclusion::ExclusionProof<T>),
}

/// The result of a tree transition, returned from that endpoint
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(
	serialize = "T::Key: Serialize, T::Value: Serialize",
	deserialize = "T::Key: serde::Deserialize<'de>, T::Value: serde::Deserialize<'de>"
))]
pub struct TreeTransitionResult<T: TreeConfig + Serialize + DeserializeOwned>
where
	T::Key: Debug,
	T::Value: Debug,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
{
	/// The new tree root
	pub root: T,
	/// The result of the operation.
	pub operation: TreeTransitionOperationResult<T>,
}
