use crate::cli::auth::set_token;
use crate::cli::commands::auth::get_index;
use clap::Args;
use pesde::Subproject;

#[derive(Debug, Args)]
pub struct LogoutCommand {
	/// The index to use. Defaults to `default`, or the configured default index if current directory doesn't have a manifest
	#[arg(short, long)]
	index: Option<String>,
}

impl LogoutCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let index_url = get_index(&subproject, self.index.as_deref()).await?;

		set_token(&index_url, None).await?;

		println!("logged out of {index_url}");

		Ok(())
	}
}
