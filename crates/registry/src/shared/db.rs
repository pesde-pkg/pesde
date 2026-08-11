use merkleberg::MMRIVER;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::MmrWriteStore;
use serde::Serialize;

#[cfg(not(any(feature = "mysql")))]
compile_error!("no db backends enabled");

pub async fn connect(url: &str) -> Box<dyn Backend> {
	let scheme = url
		.split_once(':')
		.map_or("", |(scheme, _)| scheme)
		.to_ascii_lowercase();
	let scheme = scheme.as_str();

	#[cfg(feature = "mysql")]
	if pesde_registry_backend_mysql::URL_SCHEMES.contains(&scheme) {
		return Box::new(pesde_registry_backend_mysql::MySqlBackend::connect(url).await);
	}

	panic!("unsupported database protocol `{scheme}`")
}

pub async fn append_leaf(
	store: Box<dyn MmrWriteStore>,
	pos: u64,
	body: &impl Serialize,
) -> anyhow::Result<(Box<dyn MmrWriteStore>, u64)> {
	let mut mmr: MMRIVER<CurrentMerkleHasher, Box<dyn MmrWriteStore>> = MMRIVER::new(pos, store);
	mmr.push(&canonical_bytes(body)).await?;
	mmr.commit().await?;

	let next_pos = mmr.mmr_size();
	let mut store = mmr.into_store();
	store.set_size(next_pos).await?;
	Ok((store, next_pos))
}
