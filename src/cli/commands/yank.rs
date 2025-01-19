use crate::cli::{get_index, style::SUCCESS_STYLE};
use anyhow::Context;
use clap::Args;
use pesde::{
	manifest::target::TargetKind,
	names::PackageName,
	source::{
		pesde::PesdePackageSource,
		traits::{PackageSource, RefreshOptions},
	},
	Project,
};
use reqwest::{header::AUTHORIZATION, Method, StatusCode};
use semver::Version;
use std::{fmt::Display, str::FromStr};

#[derive(Debug, Clone)]
enum TargetKindOrAll {
	All,
	Specific(TargetKind),
}

impl Display for TargetKindOrAll {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			TargetKindOrAll::All => write!(f, "all"),
			TargetKindOrAll::Specific(kind) => write!(f, "{kind}"),
		}
	}
}

impl FromStr for TargetKindOrAll {
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if s.eq_ignore_ascii_case("all") {
			return Ok(TargetKindOrAll::All);
		}

		s.parse()
			.map(TargetKindOrAll::Specific)
			.context("failed to parse target kind")
	}
}

#[derive(Debug, Clone)]
struct YankId(PackageName, Version, TargetKindOrAll);

impl FromStr for YankId {
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (package, version) = s
			.split_once('@')
			.context("package is not in format of `scope/name@version target`")?;
		let target = match version.split(' ').nth(1) {
			Some(target) => target
				.parse()
				.context("package is not in format of `scope/name@version target`")?,
			None => TargetKindOrAll::All,
		};

		Ok(YankId(
			package.parse().context("failed to parse package name")?,
			version.parse().context("failed to parse version")?,
			target,
		))
	}
}

#[derive(Debug, Args)]
pub struct YankCommand {
	/// Whether to unyank the package
	#[clap(long)]
	undo: bool,

	/// The index to yank the package from
	#[clap(short, long)]
	index: Option<String>,

	/// The package to yank
	#[clap(index = 1)]
	package: YankId,
}

impl YankCommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let YankId(package, version, target) = self.package;

		let index_url = get_index(&project, self.index.as_deref()).await?;
		let source = PesdePackageSource::new(index_url.clone());
		source
			.refresh(&RefreshOptions {
				project: project.clone(),
			})
			.await
			.context("failed to refresh source")?;
		let config = source
			.config(&project)
			.await
			.context("failed to get index config")?;

		let mut request = reqwest.request(
			if self.undo {
				Method::DELETE
			} else {
				Method::PUT
			},
			format!(
				"{}/v1/packages/{}/{}/{}/yank",
				config.api(),
				urlencoding::encode(&package.to_string()),
				urlencoding::encode(&version.to_string()),
				urlencoding::encode(&target.to_string()),
			),
		);

		if let Some(token) = project.auth_config().tokens().get(&index_url) {
			tracing::debug!("using token for {index_url}");
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await.context("failed to send request")?;

		let status = response.status();
		let text = response
			.text()
			.await
			.context("failed to get response text")?;
		let prefix = if self.undo { "un" } else { "" };
		match status {
			StatusCode::CONFLICT => {
				anyhow::bail!("version is already {prefix}yanked");
			}
			StatusCode::FORBIDDEN => {
				anyhow::bail!("unauthorized to {prefix}yank under this scope");
			}
			code if !code.is_success() => {
				anyhow::bail!("failed to {prefix}yank package: {code} ({text})");
			}
			_ => {
				println!("{}", SUCCESS_STYLE.apply_to(text));
			}
		}

		Ok(())
	}
}
