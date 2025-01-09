use pesde::manifest::target::TargetKind;
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
