use serde::{Deserialize, Serialize};

use crate::{hash::Hash, signature::PublicKey, source::pesde::registry::*};

/// The payload anchoring a scope's creation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename = "scope_genesis")]
pub struct ScopeGenesisPayload {
	/// The scope id being created
	pub scope_id: ScopeId,
	/// The owner of this scope
	pub owner: PublicKey,
	/// The hash of the first entry in the scope's log
	pub first_entry_hash: Hash,
}

impl WithSigner for ScopeGenesisPayload {
	fn signer(&self) -> &PublicKey {
		&self.owner
	}
}

/// The payload of an entry in the registry's global log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GlobalEntryPayload {
	/// A scope has been created
	ScopeGenesis(ScopeGenesisEntryPayload),
}

/// An entry in the registry's global log
pub type GlobalEntry = Entry<GlobalEntryPayload>;
/// A scope creation entry payload
pub type ScopeGenesisEntryPayload = Signed<ScopeGenesisPayload>;
