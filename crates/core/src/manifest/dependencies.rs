use serde::{Deserialize, Serialize};

use super::MAX_URL_LEN;
use crate::{
	bounded::{BoundedUrl, BoundedVersionReq},
	manifest::wally_compat::WallyPackageName,
	registry::PackageName,
};

pub const MAX_VERSION_REQ_LEN: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResolvedDependencySpecifier {
	Pesde {
		name: PackageName,
		req: BoundedVersionReq<MAX_VERSION_REQ_LEN>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		registry: Option<BoundedUrl<MAX_URL_LEN>>,
	},
	Wally {
		name: WallyPackageName,
		req: BoundedVersionReq<MAX_VERSION_REQ_LEN>,
		registry: BoundedUrl<MAX_URL_LEN>,
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
