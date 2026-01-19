use crate::cli::config::{read_config, write_config};
use anyhow::Context as _;
use keyring::Entry;
use pesde::GixUrl;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use std::collections::BTreeMap;
use tokio::task::spawn_blocking;
use tracing::instrument;

pub type Tokens = BTreeMap<GixUrl, String>;

#[instrument(level = "trace")]
pub async fn get_tokens() -> anyhow::Result<Tokens> {
	let config = read_config().await?;
	if !config.tokens.is_empty() {
		tracing::debug!("using tokens from config");
		return Ok(config.tokens);
	}

	let keyring_tokens = spawn_blocking(|| match Entry::new("tokens", env!("CARGO_PKG_NAME")) {
		Ok(entry) => match entry.get_password() {
			Ok(token) => serde_json::from_str(&token)
				.map(Some)
				.context("failed to parse tokens"),
			Err(keyring::Error::PlatformFailure(_) | keyring::Error::NoEntry) => Ok(None),
			Err(e) => Err(e.into()),
		},
		Err(keyring::Error::PlatformFailure(_)) => Ok(None),
		Err(e) => Err(e.into()),
	})
	.await
	.unwrap()?;

	if let Some(tokens) = keyring_tokens {
		tracing::debug!("using tokens from keyring");
		return Ok(tokens);
	}

	Ok(Tokens::default())
}

#[instrument(level = "trace")]
pub async fn set_tokens(tokens: Tokens) -> anyhow::Result<()> {
	let json = serde_json::to_string(&tokens).context("failed to serialize tokens")?;

	let to_keyring = spawn_blocking(move || {
		let entry = Entry::new("tokens", env!("CARGO_PKG_NAME"))?;

		match entry.set_password(&json) {
			Ok(()) => Ok::<_, anyhow::Error>(true),
			Err(keyring::Error::PlatformFailure(_) | keyring::Error::NoEntry) => Ok(false),
			Err(e) => Err(e.into()),
		}
	})
	.await
	.unwrap()?;

	if to_keyring {
		tracing::debug!("tokens saved to keyring");
		return Ok(());
	}

	tracing::debug!("saving tokens to config");

	let mut config = read_config().await?;
	config.tokens = tokens;
	write_config(&config).await
}

pub async fn set_token(repo: &GixUrl, token: Option<String>) -> anyhow::Result<()> {
	let mut tokens = get_tokens().await?;
	if let Some(token) = token {
		tokens.insert(repo.clone(), token);
	} else {
		tokens.remove(repo);
	}
	set_tokens(tokens).await
}

#[derive(Debug, Deserialize)]
struct UserResponse {
	login: String,
}

#[instrument(level = "trace")]
pub async fn get_token_login(
	reqwest: &reqwest::Client,
	access_token: &str,
) -> anyhow::Result<String> {
	let response = reqwest
		.get("https://api.github.com/user")
		.header(AUTHORIZATION, access_token)
		.send()
		.await
		.context("failed to send user request")?
		.error_for_status()
		.context("failed to get user")?
		.json::<UserResponse>()
		.await
		.context("failed to parse user response")?;

	Ok(response.login)
}
