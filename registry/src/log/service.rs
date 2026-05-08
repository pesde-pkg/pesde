use pesde::source::pesde::backend::Entry;
use pesde::source::pesde::backend::EntrySeq;

use crate::Repos;
use crate::util::AppResult;

pub struct LogService;

impl LogService {
	pub async fn entry(repos: &Repos, seq: EntrySeq) -> AppResult<Option<Entry>> {
		let entry = repos.log.entry(seq).await?;
		Ok(entry)
	}
}
