use serde::Deserialize;

/// A GitHub release
#[derive(Debug, Eq, PartialEq, Hash, Clone, Deserialize)]
pub struct Release {
	/// The tag name of the release
	pub tag_name: String,
	/// The assets of the release
	pub assets: Vec<Asset>,
}

/// An asset of a GitHub release
#[derive(Debug, Eq, PartialEq, Hash, Clone, Deserialize)]
pub struct Asset {
	/// The name of the asset
	pub name: String,
	/// The download URL of the asset
	pub url: url::Url,
}
