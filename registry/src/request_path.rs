use pesde::{
	manifest::target::TargetKind,
	source::{ids::VersionId, pesde::IndexFile},
};
use semver::Version;
use serde::{Deserialize, Deserializer};

#[derive(Debug)]
pub enum LatestOrSpecificVersion {
	Latest,
	Specific(Version),
}

impl<'de> Deserialize<'de> for LatestOrSpecificVersion {
	fn deserialize<D>(deserializer: D) -> Result<LatestOrSpecificVersion, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;
		if s.eq_ignore_ascii_case("latest") {
			return Ok(LatestOrSpecificVersion::Latest);
		}

		s.parse()
			.map(LatestOrSpecificVersion::Specific)
			.map_err(serde::de::Error::custom)
	}
}

#[derive(Debug)]
pub enum AnyOrSpecificTarget {
	Any,
	Specific(TargetKind),
}

impl<'de> Deserialize<'de> for AnyOrSpecificTarget {
	fn deserialize<D>(deserializer: D) -> Result<AnyOrSpecificTarget, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;
		if s.eq_ignore_ascii_case("any") {
			return Ok(AnyOrSpecificTarget::Any);
		}

		s.parse()
			.map(AnyOrSpecificTarget::Specific)
			.map_err(serde::de::Error::custom)
	}
}

pub fn resolve_version_and_target(
	file: &IndexFile,
	version: LatestOrSpecificVersion,
	target: AnyOrSpecificTarget,
) -> Option<&VersionId> {
	let version = match version {
		LatestOrSpecificVersion::Latest => match file.entries.keys().map(|k| k.version()).max() {
			Some(latest) => latest.clone(),
			None => return None,
		},
		LatestOrSpecificVersion::Specific(version) => version,
	};

	let mut versions = file
		.entries
		.iter()
		.filter(|(v_id, _)| *v_id.version() == version);

	match target {
		AnyOrSpecificTarget::Any => versions.min_by_key(|(v_id, _)| *v_id.target()),
		AnyOrSpecificTarget::Specific(kind) => {
			versions.find(|(_, entry)| entry.target.kind() == kind)
		}
	}
	.map(|(v_id, _)| v_id)
}

#[derive(Debug)]
pub enum AllOrSpecificTarget {
	All,
	Specific(TargetKind),
}

impl<'de> Deserialize<'de> for AllOrSpecificTarget {
	fn deserialize<D>(deserializer: D) -> Result<AllOrSpecificTarget, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;
		if s.eq_ignore_ascii_case("all") {
			return Ok(AllOrSpecificTarget::All);
		}

		s.parse()
			.map(AllOrSpecificTarget::Specific)
			.map_err(serde::de::Error::custom)
	}
}
