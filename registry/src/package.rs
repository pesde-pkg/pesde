use crate::AppState;
use pesde::{
	manifest::{
		target::{Target, TargetKind},
		Alias, DependencyType,
	},
	names::PackageName,
	source::{
		git_index::{read_file, root_tree, GitBasedSource},
		ids::VersionId,
		pesde::{IndexFile, IndexFileEntry, PesdePackageSource, ScopeInfo, SCOPE_INFO_FILE},
		specifiers::DependencySpecifiers,
	},
};
use semver::Version;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tokio::task::spawn_blocking;

#[derive(Debug, Serialize, Eq, PartialEq)]
struct TargetInfoInner {
	lib: bool,
	bin: bool,
	#[serde(skip_serializing_if = "BTreeSet::is_empty")]
	scripts: BTreeSet<String>,
}

impl TargetInfoInner {
	fn new(target: &Target) -> Self {
		TargetInfoInner {
			lib: target.lib_path().is_some(),
			bin: target.bin_path().is_some(),
			scripts: target
				.scripts()
				.map(|scripts| scripts.keys().cloned().collect())
				.unwrap_or_default(),
		}
	}
}

#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct TargetInfo {
	kind: TargetKind,
	#[serde(skip_serializing_if = "std::ops::Not::not")]
	yanked: bool,
	#[serde(flatten)]
	inner: TargetInfoInner,
}

impl TargetInfo {
	fn new(target: &Target, yanked: bool) -> Self {
		TargetInfo {
			kind: target.kind(),
			yanked,
			inner: TargetInfoInner::new(target),
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

#[derive(Debug, Serialize, Ord, PartialOrd, Eq, PartialEq)]
#[serde(untagged)]
pub enum RegistryDocEntryKind {
	Page {
		name: String,
	},
	Category {
		#[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
		items: BTreeSet<RegistryDocEntry>,
		#[serde(default, skip_serializing_if = "std::ops::Not::not")]
		collapsed: bool,
	},
}

#[derive(Debug, Serialize, Ord, PartialOrd, Eq, PartialEq)]
pub struct RegistryDocEntry {
	label: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	position: Option<usize>,
	#[serde(flatten)]
	kind: RegistryDocEntryKind,
}

impl From<pesde::source::pesde::DocEntry> for RegistryDocEntry {
	fn from(entry: pesde::source::pesde::DocEntry) -> Self {
		Self {
			label: entry.label,
			position: entry.position,
			kind: match entry.kind {
				pesde::source::pesde::DocEntryKind::Page { name, .. } => {
					RegistryDocEntryKind::Page { name }
				}
				pesde::source::pesde::DocEntryKind::Category { items, collapsed } => {
					RegistryDocEntryKind::Category {
						items: items.into_iter().map(Into::into).collect(),
						collapsed,
					}
				}
			},
		}
	}
}

#[derive(Debug, Serialize)]
pub struct PackageResponseInner {
	published_at: jiff::Timestamp,
	#[serde(skip_serializing_if = "String::is_empty")]
	license: String,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	authors: Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	repository: Option<String>,
	#[serde(skip_serializing_if = "BTreeSet::is_empty")]
	docs: BTreeSet<RegistryDocEntry>,
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	dependencies: BTreeMap<Alias, (DependencySpecifiers, DependencyType)>,
}

impl PackageResponseInner {
	pub fn new(entry: &IndexFileEntry) -> Self {
		PackageResponseInner {
			published_at: entry.published_at,
			license: entry.license.clone().unwrap_or_default(),
			authors: entry.authors.clone(),
			repository: entry.repository.clone().map(|url| url.to_string()),
			docs: entry.docs.iter().cloned().map(Into::into).collect(),
			dependencies: entry.dependencies.clone(),
		}
	}
}

#[derive(Debug, Serialize)]
pub struct PackageResponse {
	name: String,
	version: String,
	targets: BTreeSet<TargetInfo>,
	#[serde(skip_serializing_if = "String::is_empty")]
	description: String,
	#[serde(skip_serializing_if = "String::is_empty")]
	deprecated: String,
	#[serde(flatten)]
	inner: PackageResponseInner,
}

impl PackageResponse {
	pub fn new(name: &PackageName, version_id: &VersionId, file: &IndexFile) -> Self {
		let entry = file.entries.get(version_id).unwrap();

		PackageResponse {
			name: name.to_string(),
			version: version_id.version().to_string(),
			targets: file
				.entries
				.iter()
				.filter(|(ver, _)| ver.version() == version_id.version())
				.map(|(_, entry)| TargetInfo::new(&entry.target, entry.yanked))
				.collect(),
			description: entry.description.clone().unwrap_or_default(),
			deprecated: file.meta.deprecated.clone(),
			inner: PackageResponseInner::new(entry),
		}
	}
}

#[derive(Debug, Serialize)]
struct PackageVersionsResponseVersionInner {
	target: TargetInfoInner,
	#[serde(skip_serializing_if = "std::ops::Not::not")]
	yanked: bool,
	#[serde(flatten)]
	inner: PackageResponseInner,
}

#[derive(Debug, Serialize, Default)]
struct PackageVersionsResponseVersion {
	#[serde(skip_serializing_if = "String::is_empty")]
	description: String,
	targets: BTreeMap<TargetKind, PackageVersionsResponseVersionInner>,
}

#[derive(Debug, Serialize)]
pub struct PackageVersionsResponse {
	name: String,
	#[serde(skip_serializing_if = "String::is_empty")]
	deprecated: String,
	versions: BTreeMap<Version, PackageVersionsResponseVersion>,
}

impl PackageVersionsResponse {
	pub fn new(name: &PackageName, file: &IndexFile) -> Self {
		let mut versions = BTreeMap::<Version, PackageVersionsResponseVersion>::new();

		for (v_id, entry) in file.entries.iter() {
			let versions_resp = versions.entry(v_id.version().clone()).or_default();

			versions_resp.description = entry.description.clone().unwrap_or_default();
			versions_resp.targets.insert(
				entry.target.kind(),
				PackageVersionsResponseVersionInner {
					target: TargetInfoInner::new(&entry.target),
					yanked: entry.yanked,
					inner: PackageResponseInner::new(entry),
				},
			);
		}

		PackageVersionsResponse {
			name: name.to_string(),
			deprecated: file.meta.deprecated.clone(),
			versions,
		}
	}
}

pub async fn read_package(
	app_state: &AppState,
	package: &PackageName,
	source: &PesdePackageSource,
) -> Result<Option<IndexFile>, crate::error::RegistryError> {
	let path = source.path(&app_state.project);
	let package = package.clone();
	spawn_blocking(move || {
		let (scope, name) = package.as_str();
		let repo = gix::open(path)?;
		let tree = root_tree(&repo)?;

		let Some(versions) = read_file(&tree, [scope, name])? else {
			return Ok(None);
		};

		toml::de::from_str(&versions).map_err(Into::into)
	})
	.await
	.unwrap()
}

pub async fn read_scope_info(
	app_state: &AppState,
	scope: &str,
	source: &PesdePackageSource,
) -> Result<Option<ScopeInfo>, crate::error::RegistryError> {
	let path = source.path(&app_state.project);
	let scope = scope.to_string();
	spawn_blocking(move || {
		let repo = gix::open(path)?;
		let tree = root_tree(&repo)?;

		let Some(versions) = read_file(&tree, [&*scope, SCOPE_INFO_FILE])? else {
			return Ok(None);
		};

		toml::de::from_str(&versions).map_err(Into::into)
	})
	.await
	.unwrap()
}
