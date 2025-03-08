use crate::cli::{get_index, style::SUCCESS_STYLE};
use anyhow::Context as _;
use clap::Args;
use pesde::{
	names::PackageName,
	source::{
		pesde::PesdePackageSource,
		traits::{PackageSource as _, RefreshOptions},
	},
	Project,
};
use reqwest::{header::AUTHORIZATION, Method, StatusCode};

#[derive(Debug, Args)]
pub struct DeprecateCommand {
	/// Whether to undeprecate the package
	#[clap(long)]
	undo: bool,

	/// The index to deprecate the package in
	#[clap(short, long)]
	index: Option<String>,

	/// The package to deprecate
	#[clap(index = 1)]
	package: PackageName,

	/// The reason for deprecating the package
	#[clap(index = 2, required_unless_present = "undo")]
	reason: Option<String>,
}

impl DeprecateCommand {
	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
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
				"{}/v1/packages/{}/deprecate",
				config.api(),
				urlencoding::encode(&self.package.to_string()),
			),
		);

		if !self.undo {
			request = request.body(
				self.reason
					.map(|reason| reason.trim().to_string())
					.filter(|reason| !reason.is_empty())
					.context("deprecating must have non-empty a reason")?,
			);
		}

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
				anyhow::bail!("version is already {prefix}deprecated");
			}
			StatusCode::FORBIDDEN => {
				anyhow::bail!("unauthorized to {prefix}deprecate under this scope");
			}
			code if !code.is_success() => {
				anyhow::bail!("failed to {prefix}deprecate package: {code} ({text})");
			}
			_ => {
				println!("{}", SUCCESS_STYLE.apply_to(text));
			}
		}

		Ok(())
	}
}
