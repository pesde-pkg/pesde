#![expect(deprecated)]
use anyhow::Context as _;
use clap::Args;
use console::style;
use reqwest::header::ACCEPT;
use serde::Deserialize;
use std::thread::spawn;
use tokio::time::sleep;
use url::Url;

use crate::cli::{
	auth::{get_token_login, get_tokens, set_token},
	style::URL_STYLE,
};
use pesde::{
	GixUrl, Project,
	engine::source::github::GITHUB_URL,
	source::{
		pesde::PesdePackageSource,
		traits::{PackageSource as _, RefreshOptions},
	},
};

#[derive(Debug, Args)]
pub struct LoginCommand {
	/// The token to use for authentication, skipping login
	#[arg(short, long)]
	token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
	device_code: String,
	user_code: String,
	verification_uri: Url,
	expires_in: u64,
	interval: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "error")]
enum AccessTokenError {
	AuthorizationPending,
	SlowDown { interval: u64 },
	ExpiredToken,
	AccessDenied,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AccessTokenResponse {
	Success { access_token: String },

	Error(AccessTokenError),
}

impl LoginCommand {
	pub async fn authenticate_device_flow(
		&self,
		index_url: GixUrl,
		project: &Project,
		reqwest: &reqwest::Client,
	) -> anyhow::Result<String> {
		println!("logging in into {index_url}");

		let source = PesdePackageSource::new(index_url);
		source
			.refresh(&RefreshOptions {
				project: project.clone(),
			})
			.await
			.context("failed to refresh index")?;

		let config = source
			.config(project)
			.await
			.context("failed to read index config")?;
		let Some(client_id) = config.github_oauth_client_id else {
			anyhow::bail!("index not configured for Github oauth.");
		};

		let response = reqwest
			.post(Url::parse_with_params(
				"https://github.com/login/device/code",
				&[("client_id", &client_id)],
			)?)
			.header(ACCEPT, "application/json")
			.send()
			.await
			.context("failed to send device code request")?
			.error_for_status()
			.context("failed to get device code response")?
			.json::<DeviceCodeResponse>()
			.await
			.context("failed to parse device code response")?;

		println!(
			"copy your one-time code: {}\npress enter to open {} in your browser...",
			style(response.user_code).bold(),
			URL_STYLE.apply_to(response.verification_uri.as_str())
		);

		spawn(move || {
			{
				let mut input = String::new();
				std::io::stdin()
					.read_line(&mut input)
					.expect("failed to read input");
			}

			match open::that(response.verification_uri.as_str()) {
				Ok(_) => (),
				Err(e) => {
					eprintln!("failed to open browser: {e}");
				}
			}
		});

		let mut time_left = response.expires_in;
		let mut interval = std::time::Duration::from_secs(response.interval);

		while time_left > 0 {
			sleep(interval).await;
			time_left = time_left.saturating_sub(interval.as_secs());

			let response = reqwest
				.post(Url::parse_with_params(
					"https://github.com/login/oauth/access_token",
					[
						("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
						("client_id", &client_id),
						("device_code", &response.device_code),
					],
				)?)
				.header(ACCEPT, "application/json")
				.send()
				.await
				.context("failed to send access token request")?
				.error_for_status()
				.context("failed to get access token response")?
				.json::<AccessTokenResponse>()
				.await
				.context("failed to parse access token response")?;

			match response {
				AccessTokenResponse::Success { access_token } => {
					return Ok(access_token);
				}
				AccessTokenResponse::Error(e) => match e {
					AccessTokenError::AuthorizationPending => {}
					AccessTokenError::SlowDown {
						interval: new_interval,
					} => {
						interval = std::time::Duration::from_secs(new_interval);
					}
					AccessTokenError::ExpiredToken => {
						break;
					}
					AccessTokenError::AccessDenied => {
						anyhow::bail!("access denied, re-run the login command");
					}
				},
			}
		}

		anyhow::bail!("code expired, please re-run the login command");
	}

	pub async fn run(
		self,
		index_url: GixUrl,
		project: Project,
		reqwest: reqwest::Client,
	) -> anyhow::Result<()> {
		let token_given = self.token.is_some();
		let token = match self.token {
			Some(token) => token,
			None => {
				self.authenticate_device_flow(index_url.clone(), &project, &reqwest)
					.await?
			}
		};

		let token = if token_given {
			println!("set token for {index_url}");
			token
		} else {
			let token = format!("Bearer {token}");
			println!(
				"logged in as {} for {index_url}",
				style(get_token_login(&reqwest, &token).await?).bold()
			);

			token
		};

		set_token(&index_url, Some(token.clone())).await?;

		// Also save the token for GitHub API requests if we authenticated via GitHub OAuth
		if !token_given {
			let tokens = get_tokens().await?;
			if !tokens.contains_key(&GITHUB_URL) {
				set_token(&GITHUB_URL, Some(token)).await?;
			}
		}

		Ok(())
	}
}
