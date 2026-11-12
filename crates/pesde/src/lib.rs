use thiserror::Error;

use crate::registry::PackageName;

mod project;
pub use project::*;
mod manifest;
pub mod vfs;
pub use manifest::*;
mod resolver;
pub use resolver::*;
pub mod bounded;
pub mod hash;
pub mod install;
pub mod legacy;
pub mod link;
pub mod registry;
pub mod reporters;
pub mod scripts;
pub mod signature;
pub mod source;
mod url;
pub use url::*;

pub use jiff;
pub use merkle_bplustree;
pub use merkleberg;
pub use semver;

/// The name of the manifest file
pub const MANIFEST_FILE_NAME: &str = "pesde.toml";
/// The name of the lockfile
pub const LOCKFILE_FILE_NAME: &str = "pesde.lock";

#[derive(Debug, Error, thiserror_ext::Box, thiserror_ext::Construct)]
#[thiserror_ext(newtype(name = Error))]
#[non_exhaustive]
pub enum ErrorKind {
	#[error("VFS error while accessing {0}")]
	Vfs(vfs::VfsPath, #[source] std::io::Error),

	#[error("manifest parsing error for `{importer}`")]
	Manifest {
		importer: Importer,
		#[source]
		source: manifest::ManifestParseError,
	},

	#[error("members at `{0}` and `{1}` have the same name (`{2}`)")]
	DuplicateMember(Importer, Importer, PackageName),

	#[error("lockfile parsing error")]
	Lockfile(#[from] lockfile::LockfileParseError),

	#[error("lockfile writing error")]
	LockfileWrite(#[source] toml::ser::Error),

	#[error("resolve error")]
	Resolve(#[from] resolver::ResolveError),

	#[error("source error")]
	Source(#[source] crate::source::SourceError),

	#[error("functionality is disabled due to reduced trust mode")]
	ReducedTrust,
}
pub use Error as PesdeError;

pub type Result<T, E = PesdeError> = std::result::Result<T, E>;

#[doc(hidden)]
pub mod __private {
	use crate::hash::Hash;
	use crate::hash::HashAlgorithm;
	pub use crate::manifest::parse_manifest;

	#[doc(hidden)]
	#[macro_export]
	macro_rules! __do_not_use__ser_display_deser_fromstr {
		($struct_name:ident$(<$($generic:ident),+>)?) => {
			impl$(<$($generic),+>)? ::serde::Serialize for $struct_name<$($($generic),+)?> {
				fn serialize<S>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error>
				where
					S: ::serde::ser::Serializer,
				{
					serializer.collect_str(self)
				}
			}

			impl<'de $(, $($generic),+)?> ::serde::Deserialize<'de> for $struct_name<$($($generic),+)?> {
				fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
				where
					D: ::serde::de::Deserializer<'de>,
				{
					let s = String::deserialize(deserializer)?;
					s.parse().map_err(::serde::de::Error::custom)
				}
			}
		};
	}
	#[doc = "Serialize as Display, Deserialize as FromStr"]
	pub use __do_not_use__ser_display_deser_fromstr as ser_display_deser_fromstr;

	#[must_use]
	pub fn is_fs_safe_path_char(c: char) -> bool {
		c != '/'
			&& c != '\\'
			&& c != '\0'
			&& c != ':'
			&& c != '*'
			&& c != '?'
			&& c != '"'
			&& c != '<'
			&& c != '>'
			&& c != '|'
			&& !c.is_control()
	}

	// this doesn't account for names that WE reserve (context dependent);
	// this checks for OS reserved names to ensure maximum compatibility with all platforms
	#[must_use]
	pub fn is_reserved_file_name(name: &str) -> bool {
		let stem = name.split('.').next().unwrap_or(name);

		const RESERVED: &[&str] = &[
			"con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
			"com8", "com9", "com¹", "com²", "com³", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6",
			"lpt7", "lpt8", "lpt9", "lpt¹", "lpt²", "lpt³",
		];

		RESERVED
			.iter()
			.any(|reserved| reserved.eq_ignore_ascii_case(stem))
	}

	// doesn't check for things like current or parent directory references,
	// but it does check that the value (supposed to be Component::Normal-like)
	// is valid for a filesystem path component across platforms
	#[must_use]
	pub fn is_fs_safe_path_component(component: &str) -> bool {
		!component.is_empty()
			&& component.chars().all(is_fs_safe_path_char)
			&& !component.ends_with(['.', ' '])
			&& !is_reserved_file_name(component)
	}

	#[must_use]
	pub fn str_as_fs_safe(s: &str) -> String {
		let hash = Hash::digest(HashAlgorithm::default(), s.as_bytes()).encoded();
		let mut string = s
			.replacen("https://", "", 1)
			.chars()
			.map(|c| if is_fs_safe_path_char(c) { c } else { '_' })
			.collect::<String>();

		if is_fs_safe_path_component(&string) {
			if string != s {
				// a little suffix to ensure uniqueness in case of collisions. as the encoding is ASCII, this need not be rechecked.
				string.push('_');
				string.push_str(&hash[..6]);
			}

			return string;
		}

		format!("_invalid_{hash}")
	}
}

mod sealed {
	pub trait Sealed {}
}
