use crate::cli::style::ERROR_STYLE;
use crate::cli::style::INFO_STYLE;
use crate::cli::style::WARN_STYLE;
use anyhow::Context as _;
use pesde::Url;
use pesde::manifest::DependencyType;
use pesde::names::PackageName;
use pesde::names::WallyPackageName;
use pesde::source::DependencySpecifiers;
use pesde::source::PackageSources;
use pesde::source::Realm;
use pesde::source::git::GitPackageSource;
use pesde::source::git::specifier::GitDependencySpecifier;
#[expect(deprecated)]
use pesde::source::legacy_pesde::LegacyPesdePackageSource;
#[expect(deprecated)]
use pesde::source::legacy_pesde::specifier::LegacyPesdeDependencySpecifier;
#[expect(deprecated)]
use pesde::source::legacy_pesde::target::TargetKind;
use pesde::source::path::PathPackageSource;
use pesde::source::path::RelativeOrAbsolutePath;
use pesde::source::path::specifier::PathDependencySpecifier;
use pesde::source::wally::WallyPackageSource;
use pesde::source::wally::specifier::WallyDependencySpecifier;
use semver::VersionReq;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::LazyLock;

pub mod auth;
pub mod commands;
pub mod config;
pub mod install;
pub mod reporters;
pub mod style;

pub const PESDE_DIR: &str = concat!(".", env!("CARGO_PKG_NAME"));

fn base_dir() -> anyhow::Result<PathBuf> {
	Ok(match std::env::var("PESDE_HOME") {
		Ok(base) => PathBuf::from(base),
		_ => dirs::home_dir()
			.context("failed to get home directory")?
			.join(PESDE_DIR),
	})
}

pub fn config_path() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("config.toml"))
}

pub fn data_dir() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("data"))
}

#[derive(Debug, Clone)]
struct VersionedPackageName<Name: FromStr>(Name, Option<VersionReq>);

impl<Name> FromStr for VersionedPackageName<Name>
where
	Name: FromStr,
	Name::Err: Into<anyhow::Error>,
{
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let mut parts = s.splitn(2, '@');
		let name = parts.next().unwrap();
		let version = parts.next().map(FromStr::from_str).transpose()?;

		Ok(Self(name.parse().map_err(Into::into)?, version))
	}
}

#[derive(Debug, Clone)]
enum AnyPackageIdentifier {
	PesdePackageName(VersionedPackageName<PackageName>),
	WallyPackageName(VersionedPackageName<WallyPackageName>),
	Git((Url, String)),
	Path(RelativeOrAbsolutePath),
}

impl AnyPackageIdentifier {
	async fn source_and_specifier(
		&self,
		realm: Option<Realm>,
		get_index: impl AsyncFnOnce(bool) -> anyhow::Result<(String, Url)>,
	) -> anyhow::Result<(PackageSources, DependencySpecifiers)> {
		Ok(match self {
			#[expect(deprecated)]
			AnyPackageIdentifier::PesdePackageName(VersionedPackageName(name, version)) => {
				let (index_name, index_url) = get_index(true).await?;
				let source =
					PackageSources::LegacyPesde(LegacyPesdePackageSource::from_url(index_url));
				let specifier = DependencySpecifiers::LegacyPesde(LegacyPesdeDependencySpecifier {
					name: name.clone(),
					version: version.clone().unwrap_or(VersionReq::STAR),
					index: index_name,
					target: match realm {
						Some(Realm::Shared) => TargetKind::Roblox,
						Some(Realm::Server) => TargetKind::RobloxServer,
						None => TargetKind::Luau,
					},
				});

				(source, specifier)
			}
			AnyPackageIdentifier::WallyPackageName(VersionedPackageName(name, version)) => {
				let (index_name, index_url) = get_index(false).await?;
				let source = PackageSources::Wally(WallyPackageSource::from_url(index_url));
				let specifier = DependencySpecifiers::Wally(WallyDependencySpecifier {
					name: name.clone(),
					version: version.clone().unwrap_or(VersionReq::STAR),
					index: index_name,
					realm: realm.context("wally packages require a realm")?,
				});

				(source, specifier)
			}
			AnyPackageIdentifier::Git((url, ver)) => (
				PackageSources::Git(GitPackageSource::from_url(url.clone())),
				DependencySpecifiers::Git(GitDependencySpecifier {
					repo: url.clone(),
					rev: ver.clone(),
					path: None,
					realm,
				}),
			),
			AnyPackageIdentifier::Path(path) => (
				PackageSources::Path(PathPackageSource),
				DependencySpecifiers::Path(PathDependencySpecifier {
					path: path.clone(),
					realm,
				}),
			),
		})
	}
}

impl FromStr for AnyPackageIdentifier {
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if let Some(rest) = s.strip_prefix("path:") {
			Ok(AnyPackageIdentifier::Path(rest.parse().unwrap()))
		} else if s.contains(':') {
			let (repo, rev) = s
				.split_once('#')
				.context("invalid format. expected url separated by #")?;

			Ok(AnyPackageIdentifier::Git((repo.parse()?, rev.to_string())))
		} else if let Some(name) = s
			.strip_prefix("wally#")
			// pesde names cannot contain `-`, so if the string contains it we can assume it's a wally name
			.or_else(|| s.contains('-').then_some(s))
		{
			Ok(AnyPackageIdentifier::WallyPackageName(name.parse()?))
		} else {
			Ok(AnyPackageIdentifier::PesdePackageName(s.parse()?))
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

pub fn dep_type_to_key(dep_type: DependencyType) -> &'static str {
	match dep_type {
		DependencyType::Standard => "dependencies",
		DependencyType::Dev => "dev_dependencies",
		DependencyType::Peer => "peer_dependencies",
	}
}

pub static GITHUB_URL: LazyLock<Url> = LazyLock::new(|| "https://github.com".parse().unwrap());
