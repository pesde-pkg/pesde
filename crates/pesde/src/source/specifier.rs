use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use crate::__private::ser_display_deser_fromstr;
use crate::Url;
use crate::registry::PackageName;
use crate::registry::manifest::wally_compat::WallyPackageName;
use crate::semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;

use crate::source::SourceId;
use crate::vfs::VfsPath;

/// A dependency specifier. This is not the user-facing type!
/// That is part of the internal manifest parsing; this is
/// what the client works with.
///
/// It is self-contained, meaning it contains the information
/// necessary to build the [PackageSource] used by it.
///
/// This is a superset of the [pesde_core::manifest::ResolvedDependencySpecifier].
/// It contains things such as path dependencies, which cannot be published (see doc of that type).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependencySpecifier {
	/// A dependency specifier for a pesde dependency.
	Pesde(PesdeDependencySpecifier),
	/// A dependency specifier for a Wally dependency.
	Wally(WallyDependencySpecifier),
	/// A dependency specifier for a legacy pesde dependency.
	LegacyPesde(LegacyPesdeDependencySpecifier),
	/// A dependency specifier for a Git dependency.
	Git(GitDependencySpecifier),
	/// A dependency specifier for a workspace dependency.
	Workspace(WorkspaceDependencySpecifier),
}

/// A dependency specifier for a pesde dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PesdeDependencySpecifier {
	/// The name of the pesde package.
	pub name: PackageName,
	/// The version requirement for the pesde package.
	pub req: VersionReq,
	/// The registry where the pesde package is located.
	pub registry: Url,
}

/// A dependency specifier for a Wally dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WallyDependencySpecifier {
	/// The name of the Wally package.
	pub name: WallyPackageName,
	/// The version requirement for the Wally package.
	pub req: VersionReq,
	/// The index where the Wally package is located.
	pub index: Url,
}

/// A dependency specifier for a legacy pesde dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[deprecated]
pub struct LegacyPesdeDependencySpecifier {
	/// The name of the legacy pesde package.
	pub name: PackageName,
	/// The version requirement for the legacy pesde package.
	pub req: VersionReq,
	/// The index where the legacy pesde package is located.
	pub index: Url,
	/// The target kind of the legacy pesde package.
	pub target: crate::legacy::TargetKind,
}

/// A dependency specifier for a Git dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GitDependencySpecifier {
	/// The URL of the Git repository.
	pub url: Url,
	/// The revision of the Git repository.
	pub rev: Arc<str>,
	/// The path within the Git repository.
	pub path: VfsPath,
}

/// A dependency specifier for a workspace dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceDependencySpecifier {
	/// The name of the package within the workspace.
	pub name: PackageName,
	/// The version specifier for the package within the workspace.
	#[serde(default, rename = "version")]
	pub version_specifier: WorkspaceVersionSpecifier,
}

/// A version specifier for [WorkspaceDependencySpecifier]s. This is utilised when publishing, as the
/// specifier is transformed into an ordinary [DependencySpecifier] when publishing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum WorkspaceVersionSpecifier {
	/// The "^" version type
	#[default]
	Caret,
	/// The "~" version type
	Tilde,
	/// The "=" version type
	Exact,
	/// The "*" version type
	Wildcard,
	/// A concrete version requirement.
	Concrete(VersionReq),
}
ser_display_deser_fromstr!(WorkspaceVersionSpecifier);

impl Display for WorkspaceVersionSpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			WorkspaceVersionSpecifier::Caret => write!(f, "^"),
			WorkspaceVersionSpecifier::Tilde => write!(f, "~"),
			WorkspaceVersionSpecifier::Exact => write!(f, "="),
			WorkspaceVersionSpecifier::Wildcard => write!(f, "*"),
			WorkspaceVersionSpecifier::Concrete(req) => req.fmt(f),
		}
	}
}

impl FromStr for WorkspaceVersionSpecifier {
	type Err = Box<str>;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"^" => Ok(Self::Caret),
			"~" => Ok(Self::Tilde),
			"=" => Ok(Self::Exact),
			"*" => Ok(Self::Wildcard),
			_ => {
				let req = s
					.parse()
					.map_err(|e| format!("invalid workspace version specifier: `{s}` - {e}"))?;
				Ok(Self::Concrete(req))
			}
		}
	}
}

impl DependencySpecifier {
	/// Returns the source id of the dependency specifier.
	#[must_use]
	pub fn source_id(&self) -> SourceId {
		match self {
			DependencySpecifier::Pesde(spec) => SourceId::Pesde(spec.registry.clone()),
			DependencySpecifier::Wally(spec) => SourceId::Wally(spec.index.clone()),
			DependencySpecifier::LegacyPesde(spec) => SourceId::LegacyPesde(spec.index.clone()),
			DependencySpecifier::Git(spec) => SourceId::Git(spec.url.clone()),
			DependencySpecifier::Workspace(_) => SourceId::Workspace,
		}
	}
}

impl Display for DependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			DependencySpecifier::Pesde(spec) => {
				write!(f, "pesde:{}:{}@{}", spec.registry, spec.name, spec.req)
			}
			DependencySpecifier::Wally(spec) => {
				write!(f, "wally:{}:{}@{}", spec.index, spec.name, spec.req)
			}
			DependencySpecifier::LegacyPesde(spec) => {
				write!(
					f,
					"legacy_pesde:{}:{}@{} {}",
					spec.index, spec.name, spec.req, spec.target
				)
			}
			DependencySpecifier::Git(spec) => {
				let path = if spec.path.is_empty() {
					format_args!("")
				} else {
					format_args!(":{}", spec.path)
				};
				write!(f, "git:{}#{}{path}", spec.url, spec.rev)
			}
			DependencySpecifier::Workspace(spec) => {
				write!(f, "workspace:{}", spec.name)
			}
		}
	}
}
