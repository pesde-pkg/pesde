use std::sync::Arc;

use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ScopeWriteTransaction;

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
