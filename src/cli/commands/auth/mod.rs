use crate::cli::config::read_config;
use anyhow::Context as _;
use clap::Subcommand;
use pesde::DEFAULT_URL_KEY;
use pesde::Subproject;
use pesde::Url;
use pesde::errors::ManifestReadErrorKind;

mod identity;
mod login;
mod logout;
mod token;
mod whoami;

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
	/// Manages the identity for a registry
	Identity(identity::IdentityCommand),
}

impl AuthCommands {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		match self {
			AuthCommands::Login(login) => login.run(subproject).await,
			AuthCommands::Logout(logout) => logout.run(subproject).await,
			AuthCommands::WhoAmI(whoami) => whoami.run(subproject).await,
			AuthCommands::Token(token) => token.run(subproject).await,
			AuthCommands::Identity(identity) => identity.run(subproject).await,
		}
	}
}

pub(super) async fn get_index(subproject: &Subproject, index: Option<&str>) -> anyhow::Result<Url> {
	let manifest = match subproject.deser_manifest().await {
		Ok(manifest) => Some(manifest),
		Err(e) => match e.into_inner() {
			ManifestReadErrorKind::Io(e) if e.kind() == std::io::ErrorKind::NotFound => None,
			e => return Err(e.into()),
		},
	};

	let index_url = match index {
		Some(index) => index.parse().ok(),
		None => match manifest {
			Some(_) => None,
			None => Some(read_config().await?.default_index),
		},
	};

	if let Some(url) = index_url {
		return Ok(url);
	}

	let index_name = index.unwrap_or(DEFAULT_URL_KEY);

	manifest
		.unwrap()
		.pesde_indices
		.get(index_name)
		.with_context(|| format!("index {index_name} not found in manifest"))
		.cloned()
}
