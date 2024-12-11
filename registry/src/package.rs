use chrono::{DateTime, Utc};
use pesde::manifest::target::{Target, TargetKind};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct TargetInfo {
    kind: TargetKind,
    lib: bool,
    bin: bool,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    scripts: BTreeSet<String>,
}

impl From<Target> for TargetInfo {
    fn from(target: Target) -> Self {
        (&target).into()
    }
}

impl From<&Target> for TargetInfo {
    fn from(target: &Target) -> Self {
        TargetInfo {
            kind: target.kind(),
            lib: target.lib_path().is_some(),
            bin: target.bin_path().is_some(),
            scripts: target
                .scripts()
                .map(|scripts| scripts.keys().cloned().collect())
                .unwrap_or_default(),
        }
    }
}

impl Ord for TargetInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.kind.cmp(&other.kind)
    }
}

impl PartialOrd for TargetInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Serialize)]
pub struct PackageResponse {
    pub name: String,
    pub version: String,
    pub targets: BTreeSet<TargetInfo>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub published_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub license: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
}
