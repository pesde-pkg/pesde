use std::num::NonZero;

use merkleberg::MMRIVER;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::MmrReadStore;
use serde::Deserialize;

pub trait FromLogError: From<merkleberg::Error> + From<anyhow::Error> {}

#[derive(Debug, Deserialize)]
pub struct LogHeadQuery {
	from_size: Option<NonZero<u64>>,
}

pub async fn log_head<E: FromLogError>(
	mmr_size: NonZero<u64>,
	mmr_store: Box<dyn MmrReadStore>,
	query: LogHeadQuery,
) -> Result<LogHeadResponse, E> {
	let mmr = MMRIVER::<CurrentMerkleHasher, _>::new(mmr_size.get(), mmr_store);

	let consistency_proof = match query.from_size {
		Some(from_size) => mmr
			.gen_consistency_proof(from_size.get())
			.await?
			.proof_paths()
			.to_vec(),
		None => Vec::new(),
	};

	Ok(LogHeadResponse {
		accumulator: MmrAccumulator {
			peaks: mmr.get_accumulator().await?.into(),
		},
		mmr_size: mmr.mmr_size(),
		consistency_proof,
	})
}

#[derive(Debug, Deserialize)]
pub struct LogEntryQuery {
	at_size: Option<NonZero<u64>>,
}

pub async fn log_entry<P: EntryPayload, E: FromLogError>(
	pos: u64,
	get_entry: impl AsyncFnOnce(u64) -> anyhow::Result<Option<Entry<P>>>,
	get_mmr: impl AsyncFnOnce() -> anyhow::Result<MMRIVER<CurrentMerkleHasher, Box<dyn MmrReadStore>>>,
	query: LogEntryQuery,
) -> Result<Option<LogEntryResponse<P>>, E> {
	if query.at_size.is_some_and(|s| pos >= s.get()) {
		return Err(merkleberg::Error::GenProofForInvalidLeaves.into());
	}

	let entry = get_entry(pos);
	let inclusion_proof = async {
		Ok(match query.at_size {
			Some(at_size) => {
				let mmr = get_mmr().await?;

				if mmr.mmr_size() < at_size.get() {
					return Err(merkleberg::Error::GenProofForInvalidLeaves.into());
				}

				let mmr = MMRIVER::<CurrentMerkleHasher, _>::new(at_size.get(), mmr.into_store());
				mmr.gen_inclusion_proof(pos).await?.proof().to_vec()
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
