#![expect(deprecated)]
use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::manifest::target::Target;
use crate::manifest::target::TargetKind;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::fs::PackageFs;
use crate::source::ids::VersionId;
use crate::source::traits::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::fmt::Display;
use std::str::FromStr;

/// Packages' filesystems
pub mod fs;
/// Git index-based package source utilities
pub mod git_index;
/// Package identifiers for different contexts
pub mod ids;
/// Traits for sources and packages
pub mod traits;

/// Files that will not be stored when downloading a package. These are only files which break pesde's functionality, or are meaningless and possibly heavy (e.g. `.DS_Store`)
pub const IGNORED_FILES: &[&str] = &["foreman.toml", "aftman.toml", "rokit.toml", ".DS_Store"];

/// Files that should be ignored in some contexts, usually only pesde packages
pub const ADDITIONAL_FORBIDDEN_FILES: &[&str] = &["default.project.json"];

/// Directories that will not be stored when downloading a package. These are only directories which break pesde's functionality, or are meaningless and possibly heavy
pub const IGNORED_DIRS: &[&str] = &[".git"];

/// The result of resolving a package
pub type ResolveResult = (
	PackageSources,
	PackageRefs,
	BTreeMap<VersionId, BTreeMap<Alias, (DependencySpecifiers, DependencyType)>>,
	BTreeSet<TargetKind>,
);

/// A type of structure
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StructureKind {
	/// Linker files in the parent of the directory containing the package's contents
	Wally,
	/// `*_packages` directories inside the package's content directory
	PesdeV1,
}

impl Display for StructureKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			StructureKind::Wally => write!(f, "wally"),
			StructureKind::PesdeV1 => write!(f, "pesde_v1"),
		}
	}
}

impl FromStr for StructureKind {
	type Err = errors::StructureKindParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"wally" => Ok(StructureKind::Wally),
			"pesde_v1" => Ok(StructureKind::PesdeV1),
			_ => Err(errors::StructureKindParseErrorKind::UnknownKind(s.to_string()).into()),
		}
	}
}

impl Display for PackageSources {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Pesde(source) => write!(f, "pesde:{source}"),
			Self::Wally(source) => write!(f, "wally:{source}"),
			Self::Git(source) => write!(f, "git:{source}"),
			Self::Path(..) => write!(f, "path"),
		}
	}
}

impl FromStr for PackageSources {
	type Err = errors::PackageSourcesFromStr;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (tag, source) = s.split_once(':').unwrap_or((s, ""));

		Ok(match tag {
			"pesde" => Self::Pesde(source.parse()?),
			"wally" => Self::Wally(source.parse()?),
			"git" => Self::Git(source.parse()?),
			"path" if source.is_empty() => Self::Path(path::PathPackageSource),
			_ => return Err(errors::PackageSourcesFromStrKind::Unknown.into()),
		})
	}
}

macro_rules! impls {
	($($source:ident),+) => {
		paste::paste! {
			$(
				#[doc = concat!(stringify!($source), " package source")]
				pub mod [<$source:lower>];
			)+

			/// All possible dependency specifiers
			#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
			#[serde(untagged)]
			pub enum DependencySpecifiers {
				$(
					#[doc = concat!(stringify!($source), " dependency specifier")]
					$source([< $source:lower >]::specifier::[<$source DependencySpecifier>])
				),+
			}

			impl DependencySpecifier for DependencySpecifiers {}

			impl Display for DependencySpecifiers {
				fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
					match self {
						$(
							Self::$source(specifier) => write!(f, "{specifier}")
						),+
					}
				}
			}

			/// All possible package refs
			#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
			pub enum PackageRefs {
				$(
					#[doc = concat!(stringify!($source), " package ref")]
					$source([< $source:lower >]::pkg_ref::[<$source PackageRef>])
				),+
			}
			ser_display_deser_fromstr!(PackageRefs);

			impl PackageRef for PackageRefs {
				fn structure_kind(&self) -> StructureKind {
					match self {
						$(
							Self::$source(pkg_ref) => pkg_ref.structure_kind()
						),+
					}
				}
			}

			impl Display for PackageRefs {
				fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
					match self {
						$(
							PackageRefs::$source(pkg_ref) => write!(f, "{}:{pkg_ref}", stringify!($source:lower))
						),+
					}
				}
			}

			impl FromStr for PackageRefs {
				type Err = errors::PackageRefParseError;

				fn from_str(s: &str) -> Result<Self, Self::Err> {
					let (source, pkg_ref) = s
						.split_once(':')
						.ok_or(errors::PackageRefParseErrorKind::InvalidFormat)?;

					match source {
						$(
							stringify!($source:lower) => Ok(PackageRefs::$source(pkg_ref.parse().map_err(errors::PackageRefParseErrorKind::[< $source PackageRef >])?)),
						)+
						_ => Err(errors::PackageRefParseErrorKind::UnknownSource(source.to_string()).into()),
					}
				}
			}

			/// All possible package sources
			#[derive(Debug, Eq, PartialEq, Hash, Clone, PartialOrd, Ord)]
			pub enum PackageSources {
				$(
					#[doc = concat!(stringify!($source), " package source")]
					$source([< $source:lower >]::[<$source PackageSource>])
				),+
			}
			ser_display_deser_fromstr!(PackageSources);

			impl PackageSource for PackageSources {
				type Specifier = DependencySpecifiers;
				type Ref = PackageRefs;
				type RefreshError = errors::RefreshError;
				type ResolveError = errors::ResolveError;
				type DownloadError = errors::DownloadError;
				type GetTargetError = errors::GetTargetError;

				async fn refresh(&self, options: &RefreshOptions) -> Result<(), Self::RefreshError> {
					match self {
						$(
							PackageSources::$source(source) => source
								.refresh(options)
								.await
								.map_err(errors::RefreshErrorKind::$source)
						),+
					}
					.map_err(Into::into)
				}

				async fn resolve(
					&self,
					specifier: &Self::Specifier,
					options: &ResolveOptions,
				) -> Result<
					(
						PackageSources,
						PackageRefs,
						BTreeMap<VersionId, BTreeMap<Alias, (DependencySpecifiers, DependencyType)>>,
						BTreeSet<TargetKind>,
					),
					Self::ResolveError,
				> {
					match (self, specifier) {
						$(
							(PackageSources::$source(source), DependencySpecifiers::$source(specifier)) => {
								source.resolve(specifier, options).await.map_err(errors::ResolveErrorKind::$source)
							}
						)+

						_ => Err(errors::ResolveErrorKind::Mismatch.into()),
					}
					.map_err(Into::into)
				}

				async fn download<R: DownloadProgressReporter>(
					&self,
					pkg_ref: &Self::Ref,
					options: &DownloadOptions<'_, R>,
				) -> Result<PackageFs, Self::DownloadError> {
					match (self, pkg_ref) {
						$(
							(PackageSources::$source(source), PackageRefs::$source(pkg_ref)) => {
								source.download(pkg_ref, options).await.map_err(errors::DownloadErrorKind::$source)
							}
						)+

						_ => Err(errors::DownloadErrorKind::Mismatch.into()),
					}
					.map_err(Into::into)
				}

				async fn get_target(
					&self,
					pkg_ref: &Self::Ref,
					options: &GetTargetOptions<'_>,
				) -> Result<Target, Self::GetTargetError> {
					match (self, pkg_ref) {
						$(
							(PackageSources::$source(source), PackageRefs::$source(pkg_ref)) => source
								.get_target(pkg_ref, options)
								.await
								.map_err(errors::GetTargetErrorKind::$source),
						)+

						_ => Err(errors::GetTargetErrorKind::Mismatch.into()),
					}
					.map_err(Into::into)
				}
			}

			/// Errors that can occur when interacting with a package source
			pub mod errors {
				use thiserror::Error;

				/// Errors that can occur when parsing a structure kind
				#[derive(Debug, Error, thiserror_ext::Box)]
				#[thiserror_ext(newtype(name = StructureKindParseError))]
				pub enum StructureKindParseErrorKind {
					/// The structure kind is unknown
					#[error("unknown structure kind {0}")]
					UnknownKind(String),
				}

				/// Errors that can occur when parsing a Git package reference
				#[derive(Debug, Error, thiserror_ext::Box)]
				#[thiserror_ext(newtype(name = GitPackageRefParseError))]
				pub enum GitPackageRefParseErrorKind {
					/// The format of the Git package reference is invalid
					#[error("invalid Git package reference format")]
					InvalidFormat,

					/// An error occurred while parsing the structure kind
					#[error("failed to parse structure kind")]
					StructureKindParseError(#[from] StructureKindParseError),
				}

				/// Errors that can occur when parsing a package reference
				#[derive(Debug, Error, thiserror_ext::Box)]
				#[thiserror_ext(newtype(name = PackageRefParseError))]
				pub enum PackageRefParseErrorKind {
					/// The format of the package reference is invalid
					#[error("invalid package reference format")]
					InvalidFormat,

					/// The source of the package reference is unknown
					#[error("unknown package reference source {0}")]
					UnknownSource(String),

					$(
						#[doc = concat!(stringify!($source), " package reference parsing failed")]
						#[error("error parsing {} package reference", stringify!($source:lower))]
						[< $source PackageRef >](#[from] crate::source::[<$source:lower>]::pkg_ref::[<$source PackageRefParseError>])
					),+
				}

				/// Errors that occur when parsing package sources from string
				#[derive(Debug, Error, thiserror_ext::Box)]
				#[thiserror_ext(newtype(name = PackageSourcesFromStr))]
				#[non_exhaustive]
				pub enum PackageSourcesFromStrKind {
					/// The string has an invalid format
					#[error("input string is not properly formatted")]
					InvalidFormat,

					/// The source isn't known
					#[error("unknown source")]
					Unknown,

					/// Parsing the URL failed
					#[error("error parsing url")]
					UrlParse(#[from] crate::errors::GixUrlError),
				}

				/// Errors that occur when refreshing a package source
				#[derive(Debug, Error, thiserror_ext::Box)]
				#[thiserror_ext(newtype(name = RefreshError))]
				#[non_exhaustive]
				pub enum RefreshErrorKind {
					$(
						#[doc = concat!(stringify!($source), " package source failed to refresh")]
						#[error("error refreshing {} package", stringify!($source:lower))]
						$source(#[source] crate::source::[<$source:lower>]::errors::RefreshError)
					),+
				}

				/// Errors that can occur when resolving a package
				#[derive(Debug, Error, thiserror_ext::Box)]
				#[thiserror_ext(newtype(name = ResolveError))]
				#[non_exhaustive]
				pub enum ResolveErrorKind {
					/// The dependency specifier does not match the source (if using the CLI, this is a bug - file an issue)
					#[error("mismatched dependency specifier for source")]
					Mismatch,

					$(
						#[doc = concat!(stringify!($source), " package source failed to resolve")]
						#[error("error resolving {} package", stringify!($source:lower))]
						$source(#[source] crate::source::[<$source:lower>]::errors::ResolveError)
					),+
				}

				/// Errors that can occur when downloading a package
				#[derive(Debug, Error, thiserror_ext::Box)]
				#[thiserror_ext(newtype(name = DownloadError))]
				#[non_exhaustive]
				pub enum DownloadErrorKind {
					/// The package ref does not match the source (if using the CLI, this is a bug - file an issue)
					#[error("mismatched package ref for source")]
					Mismatch,

					$(
						#[doc = concat!(stringify!($source), " package source failed to download")]
						#[error("error downloading {} package", stringify!($source:lower))]
						$source(#[source] crate::source::[<$source:lower>]::errors::DownloadError)
					),+
				}

				/// Errors that can occur when getting a package's target
				#[derive(Debug, Error, thiserror_ext::Box)]
				#[thiserror_ext(newtype(name = GetTargetError))]
				#[non_exhaustive]
				pub enum GetTargetErrorKind {
					/// The package ref does not match the source (if using the CLI, this is a bug - file an issue)
					#[error("mismatched package ref for source")]
					Mismatch,

					$(
						#[doc = concat!(stringify!($source), " package source failed to get target")]
						#[error("error getting target for {} package", stringify!($source:lower))]
						$source(#[source] crate::source::[<$source:lower>]::errors::GetTargetError)
					),+
				}
			}
		}
	}
}

impls!(Pesde, Wally, Git, Path);

impl DependencySpecifiers {
	/// Returns whether this dependency specifier is for a local dependency
	#[must_use]
	pub fn is_local(&self) -> bool {
		matches!(self, DependencySpecifiers::Path(_))
	}
}

impl PackageRefs {
	/// Returns whether this package reference is local
	#[must_use]
	pub fn is_local(&self) -> bool {
		matches!(self, PackageRefs::Path(_))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::source::path::PathPackageSource;

	#[test]
	fn serde_package_sources() {
		let sources = [
			(
				PackageSources::Pesde("https://github.com/pesde-pkg/index".parse().unwrap()),
				"pesde:github.com/pesde-pkg/index",
			),
			(
				PackageSources::Wally("https://github.com/pesde-pkg/index".parse().unwrap()),
				"wally:github.com/pesde-pkg/index",
			),
			(
				PackageSources::Git("https://github.com/pesde-pkg/index".parse().unwrap()),
				"git:github.com/pesde-pkg/index",
			),
			(PackageSources::Path(PathPackageSource), "path"),
		];

		for (source, serialized) in sources {
			assert_eq!(source.to_string(), serialized);
			assert_eq!(PackageSources::from_str(serialized).unwrap(), source);
		}

		assert_eq!(
			PackageSources::from_str("path:").unwrap(),
			PackageSources::Path(PathPackageSource)
		);
		assert!(PackageSources::from_str("path:foo").is_err());
	}

	#[test]
	fn serde_package_refs() {
		let refs = [
			(
				PackageRefs::Pesde("foo/bar".parse().unwrap()),
				"pesde:foo/bar",
			),
			(
				PackageRefs::Wally("foo/bar".parse().unwrap()),
				"wally:foo/bar",
			),
			(
				PackageRefs::Git("abcdef+pesde_v1".parse().unwrap()),
				"git:abcdef+pesde_v1",
			),
			(
				PackageRefs::Path("/dev/null".parse().unwrap()),
				"path:/dev/null",
			),
		];

		for (pkg_ref, serialized) in refs {
			assert_eq!(pkg_ref.to_string(), serialized);
			assert_eq!(PackageRefs::from_str(serialized).unwrap(), pkg_ref);
		}
	}
}
