use serde::Deserialize;
use serde::Serialize;

use super::MAX_URL_LEN;
use super::wally_compat::WallyPackageName;
use crate::bounded::BoundedUrl;
use crate::bounded::BoundedVersionReq;
use crate::registry::PackageName;

pub const MAX_VERSION_REQ_LEN: usize = 255;

/// A resolved dependency specifier. This means that this is NOT a list of all possible
/// dependency specifiers/sources, but the ones a registry sees. For instance, path
/// dependencies cannot be represented here, unless they are configured to publish
/// as one of these and are transformed by the client. This means that we can show
/// some dev dependencies, e.g. for reverse dependencies, but not all of them.
/// This is the best that can be done as path packages may very well not be published anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResolvedDependencySpecifier {
	Pesde {
		name: PackageName,
		req: BoundedVersionReq<MAX_VERSION_REQ_LEN>,
		/// `None` means that this is a package in the same registry as the current package.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		registry: Option<BoundedUrl<MAX_URL_LEN>>,
	},
	Wally {
		name: WallyPackageName,
		req: BoundedVersionReq<MAX_VERSION_REQ_LEN>,
		/// The index where this package is located.
		index: BoundedUrl<MAX_URL_LEN>,
	},
}

/// A dependency type
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DependencyType {
	/// A standard dependency
	Standard,
	/// A peer dependency
	Peer,
	/// A dev dependency
	Dev,
}

impl DependencyType {
	/// All possible dependency types
	pub const VARIANTS: &'static [DependencyType] = &[
		DependencyType::Standard,
		DependencyType::Peer,
		DependencyType::Dev,
	];
}
