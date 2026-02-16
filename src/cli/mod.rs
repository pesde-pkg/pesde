use crate::cli::config::read_config;
use crate::cli::style::ERROR_STYLE;
use crate::cli::style::INFO_STYLE;
use crate::cli::style::WARN_STYLE;
use anyhow::Context as _;
use pesde::DEFAULT_INDEX_NAME;
use pesde::GixUrl;
use pesde::Subproject;
use pesde::errors::ManifestReadErrorKind;
use pesde::manifest::DependencyType;
use pesde::names::PackageNames;
use pesde::source::git::specifier::GitVersionSpecifier;
use pesde::source::path::RelativeOrAbsolutePath;
use semver::Version;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::LazyLock;

pub mod auth;
pub mod commands;
pub mod config;
pub mod install;
pub mod reporters;
pub mod style;
#[cfg(feature = "version-management")]
pub mod version;

pub const PESDE_DIR: &str = concat!(".", env!("CARGO_PKG_NAME"));

fn base_dir() -> anyhow::Result<PathBuf> {
	Ok(match std::env::var("PESDE_HOME") {
		Ok(base) => PathBuf::from(base),
		_ => dirs::home_dir()
			.context("failed to get home directory")?
			.join(PESDE_DIR),
	})
}

pub fn bin_dir() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("bin"))
}

#[cfg(feature = "version-management")]
pub fn engines_dir() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("engines"))
}

pub fn config_path() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("config.toml"))
}

pub fn data_dir() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("data"))
}

#[derive(Debug, Clone)]
struct VersionedPackageName<V: FromStr = Version, N: FromStr = PackageNames>(N, Option<V>);

impl<V: FromStr<Err = E>, E: Into<anyhow::Error>, N: FromStr<Err = F>, F: Into<anyhow::Error>>
	FromStr for VersionedPackageName<V, N>
{
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let mut parts = s.splitn(2, '@');
		let name = parts.next().unwrap();
		let version = parts
			.next()
			.map(FromStr::from_str)
			.transpose()
			.map_err(Into::into)?;

		Ok(VersionedPackageName(
			name.parse().map_err(Into::into)?,
			version,
		))
	}
}

#[derive(Debug, Clone)]
enum AnyPackageIdentifier<V: FromStr = Version, N: FromStr = PackageNames> {
	PackageName(VersionedPackageName<V, N>),
	Git((GixUrl, GitVersionSpecifier)),
	Path(RelativeOrAbsolutePath),
}

impl<V: FromStr<Err = E>, E: Into<anyhow::Error>, N: FromStr<Err = F>, F: Into<anyhow::Error>>
	FromStr for AnyPackageIdentifier<V, N>
{
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if let Some(rest) = s.strip_prefix("path:") {
			Ok(AnyPackageIdentifier::Path(rest.parse().unwrap()))
		} else if s.contains(':') {
			let (repo, ver) = match s.split_once('#') {
				Some((repo, rev)) => (repo, GitVersionSpecifier::Rev(rev.to_string())),
				None => match s.split_once('@') {
					Some((repo, req)) => (
						repo,
						GitVersionSpecifier::VersionReq(
							req.parse().context("failed to parse version requirement")?,
						),
					),
					None => anyhow::bail!("invalid format. expected url separated by # or @"),
				},
			};

			Ok(AnyPackageIdentifier::Git((repo.parse()?, ver)))
		} else {
			Ok(AnyPackageIdentifier::PackageName(s.parse()?))
		}
	}
}

pub fn display_err(result: anyhow::Result<()>, prefix: &str) {
	if let Err(err) = result {
		eprintln!(
			"{}: {err}\n",
			ERROR_STYLE.apply_to(format!("error{prefix}"))
		);

		let cause = err.chain().skip(1).collect::<Vec<_>>();

		if !cause.is_empty() {
			eprintln!("{}:", ERROR_STYLE.apply_to("caused by"));
			for err in cause {
				eprintln!("\t- {err}");
			}
		}

		let backtrace = err.backtrace();
		match backtrace.status() {
			std::backtrace::BacktraceStatus::Disabled => {
				eprintln!(
					"\n{}: set RUST_BACKTRACE=1 for a backtrace",
					INFO_STYLE.apply_to("help")
				);
			}
			std::backtrace::BacktraceStatus::Captured => {
				eprintln!("\n{}:\n{backtrace}", WARN_STYLE.apply_to("backtrace"));
			}
			_ => {
				eprintln!("\n{}: not captured", WARN_STYLE.apply_to("backtrace"));
			}
		}
	}
}

pub async fn get_index(subproject: &Subproject, index: Option<&str>) -> anyhow::Result<GixUrl> {
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

	let index_name = index.unwrap_or(DEFAULT_INDEX_NAME);

	manifest
		.unwrap()
		.indices
		.pesde
		.get(index_name)
		.with_context(|| format!("index {index_name} not found in manifest"))
		.cloned()
}

pub fn dep_type_to_key(dep_type: DependencyType) -> &'static str {
	match dep_type {
		DependencyType::Standard => "dependencies",
		DependencyType::Dev => "dev_dependencies",
		DependencyType::Peer => "peer_dependencies",
	}
}

pub static GITHUB_URL: LazyLock<GixUrl> = LazyLock::new(|| "github.com".parse().unwrap());
