use crate::cli::get_index;
use clap::{Args, Subcommand};
use pesde::Project;

mod login;
mod logout;
mod token;
mod whoami;

#[derive(Debug, Args)]
pub struct AuthSubcommand {
	/// The index to use. Defaults to `default`, or the configured default index if current directory doesn't have a manifest
	#[arg(short, long)]
	pub index: Option<String>,

	#[clap(subcommand)]
	pub command: AuthCommands,
}

#[derive(Debug, Subcommand)]
pub enum AuthCommands {
	/// Sets a token for an index. Optionally gets it from GitHub
	Login(login::LoginCommand),
	/// Removes the stored token
	Logout(logout::LogoutCommand),
	/// Prints the username of the currently logged-in user
	#[clap(name = "whoami")]
	WhoAmI(whoami::WhoAmICommand),
	/// Prints the token for an index
	Token(token::TokenCommand),
}

impl AuthSubcommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let index_url = get_index(&project, self.index.as_deref()).await?;

		match self.command {
			AuthCommands::Login(login) => login.run(index_url, project, reqwest).await,
			AuthCommands::Logout(logout) => logout.run(index_url).await,
			AuthCommands::WhoAmI(whoami) => whoami.run(index_url, reqwest).await,
			AuthCommands::Token(token) => token.run(index_url).await,
		}
	}
}
