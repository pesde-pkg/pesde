use async_trait::async_trait;
use pesde::signature::Signature;
use pesde::source::pesde::registry::Entry;
use pesde::source::pesde::registry::IdentityEntry;
use pesde::source::pesde::registry::IdentityId;
use pesde::source::pesde::registry::IdentityRotationBody;
use pesde::source::pesde::registry::RegisterIdentityBody;

use crate::db::WriteStore;

#[derive(Debug, thiserror::Error)]
pub enum IdentityWriteError {
	#[error("the identity id has already been registered")]
	NonUniqueIdentityId,

	#[error("the public key has already been registered")]
	NonUniquePublicKey,

	#[error(transparent)]
	Internal(#[from] anyhow::Error),
}

#[async_trait]
pub trait Repository {
	async fn identity_entry(&self, id: &IdentityId)
	-> anyhow::Result<Option<Entry<IdentityEntry>>>;

	async fn insert_register(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &RegisterIdentityBody,
	) -> Result<(), IdentityWriteError>;

	async fn insert_rotation(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		old_sig: &Signature,
		new_sig: &Signature,
		body: &IdentityRotationBody,
	) -> Result<(), IdentityWriteError>;
}
