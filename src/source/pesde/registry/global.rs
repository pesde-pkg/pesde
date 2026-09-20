use serde::{Deserialize, Serialize};

use crate::{hash::Hash, signature::PublicKey, source::pesde::registry::*};

/// The payload anchoring a scope's creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeGenesisPayload {
	/// The operation tag
	pub kind: OpTag<Self>,
	/// The scope id being created
	pub scope_id: ScopeId,
	/// The owner of this scope
	pub owner: PublicKey,
	/// The hash of the first entry in the scope's log
	pub first_entry_hash: Hash,
}
impl EntryPayload for Signed<ScopeGenesisPayload> {}

impl Tagged for ScopeGenesisPayload {
	const TAG: &'static str = "scope_genesis";
}

impl Signable for ScopeGenesisPayload {
	fn signer(&self) -> &PublicKey {
		&self.owner
	}
}

/// The payload of an entry in the registry's global log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GlobalEntryPayload {
	/// A scope has been created
	ScopeGenesis(Signed<ScopeGenesisPayload>),
}
impl EntryPayload for GlobalEntryPayload {}

/// An entry in the registry's global log
pub type GlobalEntry = Entry<GlobalEntryPayload>;
