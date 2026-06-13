use async_trait::async_trait;
use pesde::signature::Signature;
use pesde::source::pesde::registry::IdentityId;
use pesde::source::pesde::registry::ScopeEntryBody;
use pesde::source::pesde::registry::ScopeManifestUpdateBody;

use crate::db::WriteStore;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
	#[error("identity `{0}` is mentioned but not registered")]
	UnregisteredIdentity(IdentityId),

	#[error(transparent)]
	Internal(#[from] anyhow::Error),
}

#[async_trait]
pub trait Repository {
	async fn insert_manifest_update(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<ScopeManifestUpdateBody>,
	) -> Result<(), ManifestError>;
}
