use crate::cli::GITHUB_URL;
use crate::cli::bin_dir;
use crate::cli::config::CliConfig;
use crate::cli::config::read_config;
use crate::cli::config::write_config;
use crate::cli::style::ADDED_STYLE;
use crate::cli::style::CLI_STYLE;
use crate::cli::style::REMOVED_STYLE;
use crate::cli::style::URL_STYLE;
use crate::util::no_build_metadata;
use anyhow::Context as _;
use async_stream::stream;
use console::Style;
use fs_err::tokio as fs;
use futures::StreamExt as _;
use itertools::Itertools as _;
use jiff::SignedDuration;
use pesde::AuthConfig;
use pesde::reporters::DownloadProgressReporter;
use pesde::reporters::DownloadsReporter;
use pesde::version_matches;
use reqwest::header::AUTHORIZATION;
use semver::Version;
use semver::VersionReq;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncBufRead;
use tokio::io::AsyncReadExt;
use tokio::task::JoinSet;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::instrument;

use super::engines_dir;

pub fn current_version() -> Version {
	Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
}

fn repo_parts() -> (&'static str, &'static str) {
	let mut parts = env!("CARGO_PKG_REPOSITORY").split('/').skip(3);
	(parts.next().unwrap(), parts.next().unwrap())
}

/// A GitHub release
#[derive(Debug, Eq, PartialEq, Hash, Clone, Deserialize)]
struct Release {
	/// The tag name of the release
	pub tag_name: String,
	/// The assets of the release
	pub assets: Vec<Asset>,
}

/// An asset of a GitHub release
#[derive(Debug, Eq, PartialEq, Hash, Clone, Deserialize)]
struct Asset {
	/// The name of the asset
	pub name: String,
	/// The download URL of the asset
	pub url: url::Url,
}

const CHECK_INTERVAL: SignedDuration = SignedDuration::from_hours(6);

pub async fn query_versions(
	reqwest: &reqwest::Client,
	auth_config: &AuthConfig,
) -> anyhow::Result<impl Iterator<Item = (Version, Vec<Asset>)>> {
	let mut parts = env!("CARGO_PKG_REPOSITORY").split('/').skip(3);
	let (owner, repo) = (parts.next().unwrap(), parts.next().unwrap());

	let mut request = reqwest.get(format!(
		"https://api.github.com/repos/{}/{}/releases",
		urlencoding::encode(owner),
		urlencoding::encode(repo)
	));

	if let Some(token) = auth_config.tokens().get(&GITHUB_URL) {
		request = request.header(AUTHORIZATION, token);
	}

	let versions = request
		.send()
		.await
		.and_then(|resp| resp.error_for_status())
		.context("failed to fetch releases from GitHub API")?
		.json::<Vec<Release>>()
		.await
		.context("failed to parse releases from GitHub API")?
		.into_iter()
		.filter_map(|release| {
			Version::parse(
				release
					.tag_name
					.strip_prefix('v')
					.unwrap_or(&release.tag_name),
			)
			.ok()
			.map(|version| (version, release.assets))
		});

	Ok(versions)
}

#[instrument(skip(reqwest, auth_config), level = "trace")]
pub async fn check_for_updates(
	reqwest: &reqwest::Client,
	auth_config: &AuthConfig,
) -> anyhow::Result<()> {
	let config = read_config().await?;

	let version = if let Some((_, version)) = config
		.last_checked_updates
		.filter(|(time, _)| jiff::Timestamp::now().duration_since(*time) < CHECK_INTERVAL)
	{
		tracing::debug!("using cached version");
		version
	} else {
		tracing::debug!("checking for updates");
		let include_pre = !current_version().pre.is_empty();
		let Some(version) = query_versions(reqwest, auth_config)
			.await?
			.map(|(version, _)| version)
			.filter(|version| include_pre || version.pre.is_empty())
			.max()
		else {
			tracing::debug!("no releases found");
			return Ok(());
		};

		write_config(&CliConfig {
			last_checked_updates: Some((jiff::Timestamp::now(), version.clone())),
			..config
		})
		.await?;

		version
	};
	let current_version = current_version();
	let version_no_metadata = no_build_metadata(&version);

	if version_no_metadata <= current_version {
		return Ok(());
	}

	let alert_style = Style::new().yellow();
	let changelog = format_args!("{}/releases/tag/v{version}", env!("CARGO_PKG_REPOSITORY"));

	let messages = [
		format_args!(
			"{} {} → {}",
			alert_style.apply_to("update available!").bold(),
			REMOVED_STYLE.apply_to(current_version),
			ADDED_STYLE.apply_to(version_no_metadata)
		),
		format_args!(
			"run {} to upgrade",
			CLI_STYLE.apply_to(concat!("`", env!("CARGO_BIN_NAME"), " self-upgrade`")),
		),
		format_args!(""),
		format_args!("changelog: {}", URL_STYLE.apply_to(changelog)),
	];

	let column = alert_style.apply_to("┃");

	let message = messages
		.into_iter()
		.format_with("\n", |s, f| f(&format_args!("{column}  {s}")));

	eprintln!("\n{message}\n");

	Ok(())
}

#[instrument(level = "trace")]
async fn installed_versions(path: &Path) -> anyhow::Result<BTreeSet<Version>> {
	let mut installed_versions = BTreeSet::new();

	let mut read_dir = match fs::read_dir(&path).await {
		Ok(read_dir) => read_dir,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(installed_versions),
		Err(e) => return Err(e).context("failed to read engines directory"),
	};

	while let Some(entry) = read_dir.next_entry().await? {
		let path = entry.path();

		let Some(version) = path.file_name().and_then(|s| s.to_str()) else {
			continue;
		};

		if let Ok(version) = Version::parse(version) {
			installed_versions.insert(version);
		}
	}

	Ok(installed_versions)
}

#[instrument(skip(reqwest, reporter, auth_config), level = "trace")]
pub async fn get_or_download_version(
	reqwest: &reqwest::Client,
	req: &VersionReq,
	reporter: Arc<impl DownloadsReporter>,
	auth_config: &AuthConfig,
) -> anyhow::Result<PathBuf> {
	let (owner, repo) = repo_parts();
	let path = engines_dir()?.join("github").join(owner).join(repo);

	let installed_versions = installed_versions(&path).await?;

	if let Some(version) = installed_versions
		.into_iter()
		.rfind(|version| version_matches(&req, version))
	{
		return Ok(path
			.join(version.to_string())
			.join(env!("CARGO_BIN_NAME"))
			.with_extension(std::env::consts::EXE_EXTENSION));
	}

	let Some((version, assets)) = query_versions(reqwest, auth_config)
		.await
		.context("failed to resolve versions")?
		.filter(|(version, _)| version_matches(req, version))
		.max_by(|(ver_a, _), (ver_b, _)| ver_a.cmp(ver_b))
	else {
		anyhow::bail!("no matching versions found");
	};

	let reporter = reporter.report_download(format!(
		"{} v{}",
		no_build_metadata(&version),
		env!("CARGO_BIN_NAME")
	));
	let reporter = Arc::new(reporter);

	let mut request = reqwest.get(
		assets
			.into_iter()
			.find(|asset| {
				asset.name
					== format!(
						"{}-{}-{}-{}.zip",
						env!("CARGO_BIN_NAME"),
						version,
						std::env::consts::OS,
						std::env::consts::ARCH
					)
			})
			.context("failed to find expected asset in release")?
			.url,
	);
	if let Some(token) = auth_config.tokens().get(&GITHUB_URL) {
		request = request.header(AUTHORIZATION, token);
	}

	let response = request
		.send()
		.await
		.and_then(|resp| resp.error_for_status())
		.context("failed to query release asset")?;
	let response = response_to_async_read(response, reporter);
	tokio::pin!(response);

	let mut archive = vec![];
	response.read_to_end(&mut archive).await?;

	let archive =
		async_zip::base::read::seek::ZipFileReader::new(Cursor::new(archive).compat()).await?;

	let path = path.join(version.to_string());
	fs::create_dir_all(&path)
		.await
		.context("failed to create engine container directory")?;
	let path = path
		.join(env!("CARGO_BIN_NAME"))
		.with_extension(std::env::consts::EXE_EXTENSION);

	let mut entry = archive
		.into_entry(0)
		.await
		.context("failed to read zip archive entry")?
		.compat();
	let mut file = tokio::fs::File::create(&path)
		.await
		.context("failed to create file for downloaded version")?;
	tokio::io::copy(&mut entry, &mut file).await?;

	make_executable(&path)
		.await
		.context("failed to make downloaded version executable")?;

	Ok(path)
}

#[instrument(level = "trace")]
pub async fn replace_pesde_bin_exe(with: &Path) -> anyhow::Result<()> {
	let bin_dir = bin_dir()?;
	let bin_exe_path = bin_dir
		.join(env!("CARGO_BIN_NAME"))
		.with_extension(std::env::consts::EXE_EXTENSION);

	let exists = fs::metadata(&bin_exe_path).await.is_ok();

	if cfg!(target_os = "linux") && exists {
		fs::remove_file(&bin_exe_path)
			.await
			.context("failed to remove existing executable")?;
	} else if exists {
		let tempfile = tempfile::Builder::new()
			.make(|_| Ok(()))
			.context("failed to create temporary file")?;
		let temp_path = tempfile.into_temp_path().to_path_buf();
		#[cfg(windows)]
		let temp_path = temp_path.with_extension("exe");

		match fs::rename(&bin_exe_path, &temp_path).await {
			Ok(_) => {}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(e).context("failed to rename existing executable"),
		}
	}

	if let Some(parent) = bin_exe_path.parent() {
		fs::create_dir_all(parent)
			.await
			.context("failed to create bin directory")?;
	}

	fs::copy(with, &bin_exe_path)
		.await
		.context("failed to copy executable to bin directory")?;

	make_executable(&bin_exe_path).await?;

	let bin_exe_path: Arc<Path> = bin_exe_path.into();

	let mut entries = fs::read_dir(bin_dir)
		.await
		.context("failed to read bin directory")?;

	let mut tasks = JoinSet::new();

	while let Some(entry) = entries
		.next_entry()
		.await
		.context("failed to read bin directory entry")?
	{
		if entry
			.file_type()
			.await
			.context("failed to get bin entry type")?
			.is_dir()
		{
			continue;
		}

		let path = entry.path();

		if path
			.file_stem()
			.is_some_and(|name| name.eq_ignore_ascii_case(env!("CARGO_BIN_NAME")))
		{
			continue;
		}

		let bin_exe_path = bin_exe_path.clone();

		tasks.spawn(async move {
			fs::remove_file(&path)
				.await
				.context("failed to remove bin directory entry")?;
			fs::hard_link(bin_exe_path, path)
				.await
				.context("failed to hard link new linker in bin directory")
		});
	}

	while let Some(res) = tasks.join_next().await {
		res.unwrap()?;
	}

	Ok(())
}

#[cfg_attr(windows, allow(clippy::unused_async))]
pub async fn make_executable(_path: impl AsRef<Path>) -> anyhow::Result<()> {
	#[cfg(unix)]
	{
		use anyhow::Context as _;
		use fs_err::tokio as fs;
		use std::os::unix::fs::PermissionsExt as _;

		let mut perms = fs::metadata(&_path)
			.await
			.context("failed to get bin link file metadata")?
			.permissions();
		perms.set_mode(perms.mode() | 0o111);
		fs::set_permissions(&_path, perms)
			.await
			.context("failed to set bin link file permissions")?;
	}

	Ok(())
}

fn response_to_async_read<R: DownloadProgressReporter>(
	response: reqwest::Response,
	reporter: Arc<R>,
) -> impl AsyncBufRead {
	let total_len = response.content_length().unwrap_or(0);
	reporter.report_progress(total_len, 0);

	let mut bytes_downloaded = 0;
	let mut stream = response.bytes_stream();
	let bytes = stream!({
		while let Some(chunk) = stream.next().await {
			let chunk = match chunk {
				Ok(chunk) => chunk,
				Err(err) => {
					yield Err(std::io::Error::other(err));
					continue;
				}
			};
			bytes_downloaded += chunk.len() as u64;
			reporter.report_progress(total_len, bytes_downloaded);
			yield Ok(chunk);
		}

		reporter.report_done();
	});

	tokio_util::io::StreamReader::new(bytes)
}
