use crate::cli::auth::{get_token_login, get_tokens};
use clap::Args;
use console::style;

#[derive(Debug, Args)]
pub struct WhoAmICommand;

impl WhoAmICommand {
	pub async fn run(self, index_url: gix::Url, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let tokens = get_tokens().await?;
		let Some(token) = tokens.0.get(&index_url) else {
			println!("not logged in into {index_url}");
			return Ok(());
		};

		println!(
			"logged in as {} into {index_url}",
			style(get_token_login(&reqwest, token).await?).bold()
		);

		Ok(())
	}
}
