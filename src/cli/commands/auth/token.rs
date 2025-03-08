use crate::cli::auth::get_tokens;
use clap::Args;

#[derive(Debug, Args)]
pub struct TokenCommand;

impl TokenCommand {
	pub async fn run(self, index_url: gix::Url) -> anyhow::Result<()> {
		let tokens = get_tokens().await?;
		let Some(token) = tokens.0.get(&index_url) else {
			println!("not logged in into {index_url}");
			return Ok(());
		};

		println!("token for {index_url}: \"{token}\"");

		Ok(())
	}
}
