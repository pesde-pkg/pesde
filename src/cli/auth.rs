use crate::cli::config::{read_config, write_config};
use anyhow::Context as _;
use gix::bstr::BStr;
use keyring::Entry;
use reqwest::header::AUTHORIZATION;
use serde::{ser::SerializeMap as _, Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::task::spawn_blocking;
use tracing::instrument;

#[derive(Debug, Clone, Default)]
pub struct Tokens(pub BTreeMap<gix::Url, String>);

impl Serialize for Tokens {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::ser::Serializer,
	{
		let mut map = serializer.serialize_map(Some(self.0.len()))?;
		for (k, v) in &self.0 {
			map.serialize_entry(&k.to_bstring().to_string(), v)?;
		}
		map.end()
	}
}

impl<'de> Deserialize<'de> for Tokens {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::de::Deserializer<'de>,
	{
		Ok(Tokens(
			BTreeMap::<String, String>::deserialize(deserializer)?
				.into_iter()
				.map(|(k, v)| gix::Url::from_bytes(BStr::new(&k)).map(|k| (k, v)))
				.collect::<Result<_, _>>()
				.map_err(serde::de::Error::custom)?,
		))
	}
}

#[instrument(level = "trace")]
pub async fn get_tokens() -> anyhow::Result<Tokens> {
	let config = read_config().await?;
	if !config.tokens.0.is_empty() {
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

pub async fn set_token(repo: &gix::Url, token: Option<&str>) -> anyhow::Result<()> {
	let mut tokens = get_tokens().await?;
	if let Some(token) = token {
		tokens.0.insert(repo.clone(), token.to_string());
	} else {
		tokens.0.remove(repo);
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
