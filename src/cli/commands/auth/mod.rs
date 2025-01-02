use crate::cli::config::read_config;
use anyhow::Context;
use clap::{Args, Subcommand};
use pesde::{errors::ManifestReadError, Project, DEFAULT_INDEX_NAME};

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
		let manifest = match project.deser_manifest().await {
			Ok(manifest) => Some(manifest),
			Err(e) => match e {
				ManifestReadError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => None,
				e => return Err(e.into()),
			},
		};

		let index_url = match self.index.as_deref() {
			Some(index) => match index.try_into() {
				Ok(url) => Some(url),
				Err(_) => None,
			},
			None => match manifest {
				Some(_) => None,
				None => Some(read_config().await?.default_index),
			},
		};

		let index_url = match index_url {
			Some(url) => url,
			None => {
				let index_name = self.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME);

				manifest
					.unwrap()
					.indices
					.remove(index_name)
					.with_context(|| format!("index {index_name} not found in manifest"))?
			}
		};

		match self.command {
			AuthCommands::Login(login) => login.run(index_url, project, reqwest).await,
			AuthCommands::Logout(logout) => logout.run(index_url).await,
			AuthCommands::WhoAmI(whoami) => whoami.run(index_url, reqwest).await,
			AuthCommands::Token(token) => token.run(index_url).await,
		}
	}
}
