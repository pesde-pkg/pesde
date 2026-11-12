use std::cmp::Ordering;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use crate::__private::ser_display_deser_fromstr;
use crate::PackageName;
use crate::Url;
use crate::registry::manifest::wally_compat::WallyPackageName;
use crate::semver::Version;

use crate::Importer;
use crate::source::SourceId;
use crate::vfs::VfsPath;

/// This struct implements Ord for sorted user display,
/// but [PackageId::compare] should be used for actual comparisons.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PackageId {
	Pesde(Url, PackageName, Version),
	Wally(Url, WallyPackageName, Version),
	LegacyPesde(Url, PackageName, Version, crate::legacy::TargetKind),
	Git(Url, Arc<str>, VfsPath),
	Workspace(Importer),
}
ser_display_deser_fromstr!(PackageId);

impl PackageId {
	/// Returns the source id of the package id.
	#[must_use]
	pub fn source_id(&self) -> SourceId {
		match self {
			PackageId::Pesde(url, _, _) => SourceId::Pesde(url.clone()),
			PackageId::Wally(url, _, _) => SourceId::Wally(url.clone()),
			PackageId::LegacyPesde(url, _, _, _) => SourceId::LegacyPesde(url.clone()),
			PackageId::Git(url, _, _) => SourceId::Git(url.clone()),
			PackageId::Workspace(_) => SourceId::Workspace,
		}
	}

	/// Compares two package IDs.
	/// If the package IDs are not of the same source, None is returned.
	/// Incomparable package IDs (e.g. git or workspace) return Ordering::Equal.
	#[must_use]
	pub fn compare(&self, other: &Self) -> PackageIdComparison {
		match (self, other) {
			(PackageId::Pesde(_, _, prev_version), PackageId::Pesde(_, _, new_version)) => {
				PackageIdComparison::Comparable(prev_version.cmp(new_version))
			}

			(PackageId::Wally(_, _, prev_version), PackageId::Wally(_, _, new_version)) => {
				PackageIdComparison::Comparable(prev_version.cmp(new_version))
			}

			(
				PackageId::LegacyPesde(_, _, prev_version, _),
				PackageId::LegacyPesde(_, _, new_version, _),
			) => PackageIdComparison::Comparable(prev_version.cmp(new_version)),

			(PackageId::Git(..), PackageId::Git(..)) => PackageIdComparison::Incomparable,

			(PackageId::Workspace(..), PackageId::Workspace(..)) => {
				PackageIdComparison::Incomparable
			}

			_ => PackageIdComparison::NotSameSource,
		}
	}
}

/// The result of comparing 2 package IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PackageIdComparison {
	Comparable(Ordering),
	Incomparable,
	NotSameSource,
}

impl Display for PackageId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			PackageId::Pesde(url, name, version) => {
				write!(f, "pesde:{url}:{name}@{version}")
			}
			PackageId::Wally(url, name, version) => {
				write!(f, "wally:{url}:{name}@{version}")
			}
			PackageId::LegacyPesde(url, name, version, target) => {
				write!(f, "legacy_pesde:{url}:{name}@{version} {target}")
			}
			PackageId::Git(url, revision, path) => {
				let path = if path.is_empty() {
					format_args!("")
				} else {
					format_args!(":{path}")
				};
				write!(f, "git:{url}#{revision}{path}")
			}
			PackageId::Workspace(importer) => {
				write!(f, "workspace:{importer}")
			}
		}
	}
}

impl FromStr for PackageId {
	type Err = Box<str>;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (source, rest) = s
			.split_once(':')
			.ok_or_else(|| format!("invalid package id: `{s}`"))?;

		match source {
			"pesde" => {
				let (url, rest) = rest
					.rsplit_once(':')
					.ok_or_else(|| format!("invalid package id: `{s}`"))?;
				let (name, version) = rest
					.split_once('@')
					.ok_or_else(|| format!("invalid package id: `{s}`"))?;

				let url = url
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;
				let name = name
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;
				let version = version
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;

				Ok(PackageId::Pesde(url, name, version))
			}
			"wally" => {
				let (url, rest) = rest
					.rsplit_once(':')
					.ok_or_else(|| format!("invalid package id: `{s}`"))?;
				let (name, version) = rest
					.split_once('@')
					.ok_or_else(|| format!("invalid package id: `{s}`"))?;

				let url = url
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;
				let name = name
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;
				let version = version
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;

				Ok(PackageId::Wally(url, name, version))
			}
			"legacy_pesde" => {
				let (url, rest) = rest
					.rsplit_once(':')
					.ok_or_else(|| format!("invalid package id: `{s}`"))?;
				let (name, rest) = rest
					.split_once('@')
					.ok_or_else(|| format!("invalid package id: `{s}`"))?;
				let (version, target) = rest
					.split_once(' ')
					.ok_or_else(|| format!("invalid package id: `{s}`"))?;

				let url = url
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;
				let name = name
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;
				let version = version
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;
				let target = target
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;

				Ok(PackageId::LegacyPesde(url, name, version, target))
			}
			"git" => {
				let (url, rest) = rest
					.split_once('#')
					.ok_or_else(|| format!("invalid package id: `{s}`"))?;

				let (revision, path) = match rest.split_once(':') {
					Some((rev, path)) => (rev, path),
					None => (rest, ""),
				};

				let url = url
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;
				let revision = revision.into();
				let path = path
					.parse()
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;

				Ok(PackageId::Git(url, revision, path))
			}
			"workspace" => {
				let importer = rest
					.parse()
					.map(Importer::from_path)
					.map_err(|e| format!("invalid package id: `{s}` - {e}"))?;

				Ok(PackageId::Workspace(importer))
			}
			source => Err(format!("invalid package source: `{source}`").into()),
		}
	}
}
