use crate::{
	cli::{
		bin_dir,
		config::{CliConfig, read_config, write_config},
		style::{ADDED_STYLE, CLI_STYLE, REMOVED_STYLE, URL_STYLE},
	},
	util::no_build_metadata,
};
use anyhow::Context as _;
use console::Style;
use fs_err::tokio as fs;
use jiff::SignedDuration;
use pesde::{
	AuthConfig,
	engine::{
		EngineKind,
		source::{
			EngineSources,
			traits::{DownloadOptions, EngineSource as _, ResolveOptions},
		},
	},
	reporters::DownloadsReporter,
	version_matches,
};
use semver::{Version, VersionReq};
use std::{
	collections::BTreeSet,
	env::current_exe,
	path::{Path, PathBuf},
	sync::Arc,
};
use tokio::task::JoinSet;
use tracing::instrument;

use super::engines_dir;

pub fn current_version() -> Version {
	Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
}

const CHECK_INTERVAL: SignedDuration = SignedDuration::from_hours(6);

pub async fn find_latest_version(
	reqwest: &reqwest::Client,
	include_pre: bool,
	auth_config: &AuthConfig,
) -> anyhow::Result<Version> {
	let include_pre = include_pre || !current_version().pre.is_empty();

	let version = EngineSources::pesde()
		.resolve(
			&VersionReq::STAR,
			&ResolveOptions {
				reqwest: reqwest.clone(),
				auth_config: auth_config.clone(),
			},
		)
		.await
		.context("failed to resolve version")?
		.into_keys()
		.filter(|ver| include_pre || ver.pre.is_empty())
		// since the iterator is from a BTreeMap it is already sorted
		.next_back()
		.context("no versions found")?;

	Ok(version)
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
		let version = find_latest_version(reqwest, false, auth_config).await?;

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
	let changelog = format!("{}/releases/tag/v{version}", env!("CARGO_PKG_REPOSITORY"));

	let messages = [
		format!(
			"{} {} → {}",
			alert_style.apply_to("update available!").bold(),
			REMOVED_STYLE.apply_to(current_version),
			ADDED_STYLE.apply_to(version_no_metadata)
		),
		format!(
			"run {} to upgrade",
			CLI_STYLE.apply_to(concat!("`", env!("CARGO_BIN_NAME"), " self-upgrade`")),
		),
		"".to_string(),
		format!("changelog: {}", URL_STYLE.apply_to(changelog)),
	];

	let column = alert_style.apply_to("┃");

	let message = messages
		.into_iter()
		.map(|s| format!("{column}  {s}"))
		.collect::<Vec<_>>()
		.join("\n");

	eprintln!("\n{message}\n");

	Ok(())
}

#[instrument(level = "trace")]
pub async fn get_installed_versions(engine: EngineKind) -> anyhow::Result<BTreeSet<Version>> {
	let source = engine.source();
	let path = engines_dir()?.join(source.directory());
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
pub async fn get_or_download_engine(
	reqwest: &reqwest::Client,
	engine: EngineKind,
	req: VersionReq,
	reporter: Arc<impl DownloadsReporter>,
	auth_config: &AuthConfig,
) -> anyhow::Result<(PathBuf, Version)> {
	let source = engine.source();
	let path = engines_dir()?.join(source.directory());

	let installed_versions = get_installed_versions(engine).await?;

	let max_matching = installed_versions
		.iter()
		.filter(|v| version_matches(&req, v))
		.next_back();
	if let Some(version) = max_matching {
		return Ok((
			path.join(version.to_string())
				.join(source.expected_file_name())
				.with_extension(std::env::consts::EXE_EXTENSION),
			version.clone(),
		));
	}

	let mut versions = source
		.resolve(
			&req,
			&ResolveOptions {
				reqwest: reqwest.clone(),
				auth_config: auth_config.clone(),
			},
		)
		.await
		.context("failed to resolve versions")?;
	let (version, engine_ref) = versions.pop_last().context("no matching versions found")?;

	let reporter = reporter.report_download(format!("{engine} v{}", no_build_metadata(&version)));

	let archive = source
		.download(
			&engine_ref,
			&DownloadOptions {
				reqwest: reqwest.clone(),
				reporter: reporter.into(),
				version: version.clone(),
				auth_config: auth_config.clone(),
			},
		)
		.await
		.context("failed to download engine")?;

	let path = path.join(version.to_string());
	fs::create_dir_all(&path)
		.await
		.context("failed to create engine container folder")?;
	let path = path
		.join(source.expected_file_name())
		.with_extension(std::env::consts::EXE_EXTENSION);

	let mut file = fs::File::create(&path)
		.await
		.context("failed to create new file")?;

	tokio::io::copy(
		&mut archive
			.find_executable(source.expected_file_name())
			.await
			.context("failed to find executable")?,
		&mut file,
	)
	.await
	.context("failed to write to file")?;

	make_executable(&path)
		.await
		.context("failed to make downloaded version executable")?;

	if engine != EngineKind::Pesde {
		make_linker_if_needed(engine).await?;
	}

	Ok((path, version))
}

#[instrument(level = "trace")]
pub async fn replace_pesde_bin_exe(with: &Path) -> anyhow::Result<()> {
	let bin_dir = bin_dir()?;
	let bin_name = EngineKind::Pesde.to_string();
	let bin_exe_path = bin_dir
		.join(&bin_name)
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
		.context("failed to copy executable to bin folder")?;

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
			.is_some_and(|name| name.eq_ignore_ascii_case(&bin_name))
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

#[instrument(level = "trace")]
pub async fn make_linker_if_needed(engine: EngineKind) -> anyhow::Result<()> {
	let linker = bin_dir()?
		.join(engine.to_string())
		.with_extension(std::env::consts::EXE_EXTENSION);

	let exe = current_exe().context("failed to get current exe path")?;

	if let Some(parent) = linker.parent() {
		fs::create_dir_all(parent)
			.await
			.context("failed to create linker directory")?;
	}

	match fs::hard_link(exe, linker).await {
		Ok(_) => {}
		Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
		e => e.context("failed to hard link engine executable")?,
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
