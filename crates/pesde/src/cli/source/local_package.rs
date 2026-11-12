#![expect(deprecated)]
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use pesde::__private::parse_manifest;
use pesde::Importer;
use pesde::MANIFEST_FILE_NAME;
use pesde::Manifest;
use pesde::link::PackageStructure;
use pesde::source::InspectResult;
use pesde::source::PackageId;
use serde::Deserialize as _;

use crate::cli::source::index_legacy_pesde::LegacyPesdeManifest;
use crate::cli::source::index_wally::WallyManifest;
use crate::cli::vfs::find_project;

pub struct LocalPackage {
	pub workspace_dir: PathBuf,
	pub importer: Importer,
}

impl LocalPackage {
	pub async fn find(root: &Path, subpath: &Path) -> anyhow::Result<Self> {
		let (workspace_root, importer) = find_project(root, subpath).await?;
		let workspace_dir = root.join(workspace_root);

		Ok(Self {
			workspace_dir,
			importer,
		})
	}

	pub async fn inspect(&self, package_id: &PackageId) -> anyhow::Result<InspectResult> {
		match self.detect_manifest(package_id).await? {
			DetectedManifest::Pesde(manifest) => Ok(InspectResult {
				structure: PackageStructure::Pesde(
					manifest
						.exports
						.context("can't depend on exportless package")?,
					manifest.config,
				),
				dependencies: manifest.dependencies.into_inner(),
			}),
			DetectedManifest::LegacyPesde(manifest) => Ok(InspectResult {
				structure: PackageStructure::LegacyPesde(
					manifest
						.target
						.exports()
						.map_err(anyhow::Error::from_boxed)
						.with_context(|| format!("package `{package_id}`"))?,
					manifest.target.kind(),
				),
				dependencies: manifest.all_dependencies()?,
			}),
			DetectedManifest::Wally(manifest) => {
				let (_, inspect) = manifest.into_inspect_parts().with_context(|| {
					format!("failed to collect wally dependencies of package `{package_id}`")
				})?;

				Ok(inspect)
			}
		}
	}

	async fn detect_manifest(&self, package_id: &PackageId) -> anyhow::Result<DetectedManifest> {
		let package_dir = self.workspace_dir.join(&**self.importer);

		if let Some(manifest) = read_optional(&package_dir.join(MANIFEST_FILE_NAME)).await? {
			let table: toml::map::Map<String, toml::Value> =
				toml::from_str(&manifest).context("failed to parse pesde manifest")?;

			if !table.contains_key("package") {
				return Ok(DetectedManifest::LegacyPesde(
					LegacyPesdeManifest::deserialize(table).with_context(|| {
						format!("failed to parse legacy pesde manifest of package `{package_id}`")
					})?,
				));
			}

			let root = if self.importer.is_root() {
				None
			} else {
				let root_path = self.workspace_dir.join(MANIFEST_FILE_NAME);
				let root = read_optional(&root_path).await?.with_context(|| {
					format!(
						"workspace root manifest not found at {}",
						root_path.display()
					)
				})?;

				Some(parse_manifest(&root, None).with_context(|| {
					format!("failed to parse workspace root manifest of package `{package_id}`")
				})?)
			};

			return Ok(DetectedManifest::Pesde(
				parse_manifest(&manifest, root.as_ref()).with_context(|| {
					format!("failed to parse manifest of package `{package_id}`")
				})?,
			));
		}

		let manifest = read_optional(&package_dir.join("wally.toml"))
			.await?
			.with_context(|| format!("no manifest found in package `{package_id}`"))?;

		let manifest: WallyManifest = toml::from_str(&manifest)
			.with_context(|| format!("failed to parse wally manifest of package `{package_id}`"))?;

		Ok(DetectedManifest::Wally(manifest))
	}
}

#[expect(clippy::large_enum_variant)]
enum DetectedManifest {
	Pesde(Manifest),
	LegacyPesde(LegacyPesdeManifest),
	Wally(WallyManifest),
}

async fn read_optional(path: &Path) -> anyhow::Result<Option<String>> {
	match tokio::fs::read_to_string(path).await {
		Ok(s) => Ok(Some(s)),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(e).with_context(|| format!("error reading {}", path.display())),
	}
}
