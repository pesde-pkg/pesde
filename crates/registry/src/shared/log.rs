use std::num::NonZero;

use pesde::merkleberg::MMRIVER;
use pesde::merkleberg::MMRStoreReadOps;
use pesde::merkleberg::leaf_index_to_mmr_size;
use pesde::merkleberg::leaf_index_to_pos;
use pesde::registry::*;
use pesde_registry_core::db::StoreError;
use serde::Deserialize;

pub trait FromLogError: From<pesde::merkleberg::Error> + From<anyhow::Error> {}

#[derive(Debug, Deserialize)]
pub struct LogHeadQuery {
	from_size: Option<NonZero<u64>>,
}

pub async fn log_head<
	S: MMRStoreReadOps<CurrentHash, Error = StoreError>,
	P: EntryPayload,
	E: FromLogError,
>(
	size: NonZero<u64>,
	mmr_store: S,
	get_entry: impl AsyncFnOnce(u64) -> Result<Option<Entry<P>>, E>,
	query: LogHeadQuery,
) -> Result<LogHeadResponse<P>, E> {
	let tip_index = size.get() - 1;
	let mmr = MMRIVER::<CurrentMerkleHasher, _>::new(leaf_index_to_mmr_size(tip_index), mmr_store);

	let consistency_proof = match query.from_size {
		Some(from_size) => {
			if from_size > size {
				return Err(pesde::merkleberg::Error::GenProofForInvalidLeaves.into());
			}

			mmr.gen_consistency_proof(leaf_index_to_mmr_size(from_size.get() - 1))
				.await?
				.proof_paths()
				.to_vec()
		}
		None => Vec::new(),
	};

	let tip = get_entry(tip_index)
		.await?
		.ok_or_else(|| anyhow::anyhow!("the tip entry of the log is missing"))?;
	let inclusion_proof = mmr
		.gen_inclusion_proof(leaf_index_to_pos(tip_index))
		.await?
		.proof()
		.to_vec();

	Ok(LogHeadResponse {
		accumulator: mmr.get_accumulator().await?.into(),
		tip: LogTip {
			entry: tip,
			inclusion_proof,
		},
		consistency_proof,
	})
}

#[derive(Debug, Deserialize)]
pub struct LogEntryQuery {
	at_size: Option<NonZero<u64>>,
}

pub async fn log_entry<
	S: MMRStoreReadOps<CurrentHash, Error = StoreError>,
	P: EntryPayload,
	E: FromLogError,
>(
	index: u64,
	get_entry: impl AsyncFnOnce(u64) -> Result<Option<Entry<P>>, E>,
	get_mmr: impl AsyncFnOnce() -> Result<(NonZero<u64>, S), E>,
	query: LogEntryQuery,
) -> Result<Option<LogEntryResponse<P>>, E> {
	if query.at_size.is_some_and(|s| index >= s.get()) {
		return Err(pesde::merkleberg::Error::GenProofForInvalidLeaves.into());
	}

	let entry = get_entry(index);
	let inclusion_proof = async {
		Ok(match query.at_size {
			Some(at_size) => {
				let (size, mmr_store) = get_mmr().await?;

				if size < at_size {
					return Err(pesde::merkleberg::Error::GenProofForInvalidLeaves.into());
				}

				let mmr = MMRIVER::<CurrentMerkleHasher, _>::new(
					leaf_index_to_mmr_size(at_size.get() - 1),
					mmr_store,
				);
				mmr.gen_inclusion_proof(leaf_index_to_pos(index))
					.await?
					.proof()
					.to_vec()
			}
			None => Vec::new(),
		})
	};

	let (entry, inclusion_proof) = tokio::try_join!(entry, inclusion_proof)?;

	Ok(entry.map(|entry| LogEntryResponse {
		entry,
		inclusion_proof,
	}))
}
