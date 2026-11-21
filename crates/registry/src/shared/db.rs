use std::sync::Arc;

use pesde::merkleberg::MMRIVER;
use pesde::merkleberg::MMRStoreReadOps;
use pesde::merkleberg::MMRStoreWriteOps;
use pesde::merkleberg::leaf_index_to_mmr_size;
use pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ScopeWriteTransaction;
use pesde_registry_core::db::StoreError;
use serde::Serialize;

#[cfg(not(any(feature = "mysql")))]
compile_error!("no db backends enabled");

pub async fn connect(url: &str) -> anyhow::Result<Arc<dyn Backend>> {
	let scheme = url
		.split_once(':')
		.map_or("", |(scheme, _)| scheme)
		.to_ascii_lowercase();
	let scheme = scheme.as_str();

	#[cfg(feature = "mysql")]
	if pesde_registry_backend_mysql::URL_SCHEMES.contains(&scheme) {
		return pesde_registry_backend_mysql::connect(url).await;
	}

	anyhow::bail!("unsupported database protocol `{scheme}`")
}

pub async fn append_leaf<
	S: MMRStoreReadOps<CurrentHash, Error = StoreError>
		+ MMRStoreWriteOps<CurrentHash, Error = StoreError>,
>(
	store: S,
	body: &Entry<impl EntryPayload + Serialize>,
) -> anyhow::Result<()> {
	let mut mmr = MMRIVER::<CurrentMerkleHasher, _>::new(leaf_index_to_mmr_size(body.index), store);
	mmr.push(&body.canonical_bytes()).await?;
	mmr.commit().await?;
	Ok(())
}

pub async fn run_tx<T, E: From<anyhow::Error>>(
	mut tx: Box<dyn ScopeWriteTransaction>,
	cb: impl AsyncFnOnce(&mut dyn ScopeWriteTransaction) -> Result<T, E>,
) -> Result<T, E> {
	match cb(&mut *tx).await {
		Ok(t) => {
			tx.commit().await?;
			Ok(t)
		}
		Err(e) => {
			if let Err(e) = tx.rollback().await {
				tracing::error!(?e, "failed to rollback transaction");
			}
			Err(e)
		}
	}
}
