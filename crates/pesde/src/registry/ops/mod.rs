use crate::registry::Signed;
use serde::Deserialize;
use serde::Serialize;

mod global;
pub use global::*;
mod scope;
pub use scope::*;
mod consent;
pub use consent::*;

/// The body of a package publish operation.
/// The publish endpoint is special. The body is the JSON encoding of this struct, followed by '\n', and then the archive bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishPackageBody {
	/// The scope genesis entry, if this scope hasn't been created yet.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scope_genesis: Option<Signed<ScopeGenesisPayload>>,
	/// The package entry being published.
	pub publish_version: Signed<PublishVersionUserScopeOpBody>,
}
