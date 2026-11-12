use serde::Deserialize;
use serde::Serialize;

use super::super::*;
use crate::hash::Hash;
use crate::signature::PublicKey;

/// The payload anchoring a scope's creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeGenesisPayload {
	/// The scope id being created
	pub scope_id: ScopeId,
	/// The owner of this scope
	pub owner: PublicKey,
	/// The hash of the first entry in the scope's log
	pub first_entry_hash: Hash,
}
impl crate::sealed::Sealed for ScopeGenesisPayload {}
impl Tagged for ScopeGenesisPayload {
	const TAG: &'static str = "scope_genesis";
}

impl Canonical for ScopeGenesisPayload {
	fn write_canonical_to(&self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
		let Self {
			scope_id,
			owner,
			first_entry_hash,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"owner" => owner.to_string(),
			"first_entry_hash" => first_entry_hash.to_string(),
		}
		.write_to(writer)
	}
}

impl Signable for ScopeGenesisPayload {
	fn signer(&self) -> &PublicKey {
		&self.owner
	}

	fn signing_bytes(&self) -> Vec<u8> {
		self.canonical_bytes()
	}
}
impl EntryPayload for Signed<ScopeGenesisPayload> {}

/// The payload of an entry in the registry's global log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ns", rename_all = "snake_case")]
pub enum GlobalEntryPayload {
	/// A scope has been created
	ScopeGenesis(Signed<ScopeGenesisPayload>),
}
impl crate::sealed::Sealed for GlobalEntryPayload {}
impl EntryPayload for GlobalEntryPayload {}

impl Canonical for GlobalEntryPayload {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		match self {
			Self::ScopeGenesis(p) => p.write_canonical_to(writer),
		}
	}
}

/// An entry in the registry's global log
pub type GlobalEntry = Entry<GlobalEntryPayload>;
