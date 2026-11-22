//
//
// WARNING! do not use the streaming API of async_zip: https://github.com/Majored/rs-async-zip/pull/203
//
//

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use async_zip::ZipEntryBuilder;
use clap::Args;
use futures::TryStreamExt as _;
use pesde::MANIFEST_FILE_NAME;
use pesde::bounded::BoundedBTreeMap;
use pesde::bounded::BoundedUrl;
use pesde::bounded::BoundedVersionReq;
use pesde::hash::Blake3Hash;
use pesde::hash::Hash;
use pesde::hash::HashAlgorithm;
use pesde::registry::manifest::DependencyType;
use pesde::registry::manifest::ResolvedDependencySpecifier;
use pesde::registry::manifest::ResolvedManifest;
use pesde::registry::*;
use pesde::source::DependencySpecifier;
use pesde::source::PackageId;
use pesde::source::WorkspaceVersionSpecifier;
use pesde::vfs::Vfs;
use semver::VersionReq;
use tokio::io::AsyncReadExt as _;

use crate::cli::install::get_lockfile_locked;
use crate::cli::registry::TreeUpdate;
use crate::cli::ui::command::registry::RegistryCommandContext;
use crate::cli::ui::print_immutable_protocol_warning;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Publishes a package
#[derive(Debug, Args)]
pub struct PublishCommand {
	/// Whether to save the package to a file instead of publishing it to the registry
	#[arg(short, long)]
	dry_run: bool,

	/// Agree to all prompts
	#[arg(short, long)]
	yes: bool,

	/// The public key to use for signing the package. If not specified, the associated key of the registry will be used
	#[arg(short, long)]
	public_key: Option<UserInputPublicKey>,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl PublishCommand {
	pub async fn run(self, ctx: RegistryCommandContext) -> anyhow::Result<()> {
		let member = ctx.member().await?;
		let vfs = ctx.vfs().await?;
		let project = ctx.project().await?;
		let manifest = member.deser_manifest().await?;

		if !manifest.publish {
			anyhow::bail!("package is marked as not publishable in manifest");
		}

		let Some(exports) = &manifest.exports else {
			anyhow::bail!("package has no exports defined in manifest");
		};

		let resolution = get_lockfile_locked(project).await?;

		let resolved_manifest = ResolvedManifest {
			package: manifest.package.clone(),
			exports: exports.clone(),
			config_luau: manifest.config.clone(),
			dependencies: {
				let mut resolved = BTreeMap::new();

				for (alias, (spec, ty)) in manifest.dependencies.iter() {
					let spec = match spec {
						DependencySpecifier::Pesde(spec) => ResolvedDependencySpecifier::Pesde {
							name: spec.name.clone(),
							req: BoundedVersionReq::new(spec.req.clone()).with_context(|| {
								format!("failed to create bounded version req for {alias}")
							})?,
							registry: (spec.registry != *ctx.client().url())
								.then(|| spec.registry.clone())
								.map(BoundedUrl::new)
								.transpose()
								.with_context(|| {
									format!("failed to create bounded registry url for {alias}")
								})?,
						},
						DependencySpecifier::Wally(spec) => ResolvedDependencySpecifier::Wally {
							name: spec.name.clone(),
							req: BoundedVersionReq::new(spec.req.clone()).with_context(|| {
								format!("failed to create bounded version req for {alias}")
							})?,
							index: BoundedUrl::new(spec.index.clone()).with_context(|| {
								format!("failed to create bounded index url for {alias}")
							})?,
						},
						DependencySpecifier::Workspace(spec) => {
							let req = match &spec.version_specifier {
								WorkspaceVersionSpecifier::Concrete(req) => req.clone(),
								WorkspaceVersionSpecifier::Wildcard => VersionReq::STAR,
								spec => {
									let id = &resolution.importers[member.importer()].dependencies
										[alias]
										.0;
									let PackageId::Workspace(importer) = id else {
										unreachable!();
									};

									let version = member
										.workspace()
										.clone()
										.member(importer.clone())
										.deser_manifest()
										.await
										.with_context(|| {
											format!(
												"failed to deserialize manifest for workspace member {importer}"
											)
										})?
										.package
										.version
										.clone();

									format!("{spec}{version}").parse().with_context(|| {
										format!(
											"failed to parse constructed version specifier for {alias}"
										)
									})?
								}
							};

							ResolvedDependencySpecifier::Pesde {
								name: spec.name.clone(),
								req: BoundedVersionReq::new(req).with_context(|| {
									format!("failed to create bounded version req for {alias}")
								})?,
								registry: None,
							}
						}
						DependencySpecifier::Git(_) | DependencySpecifier::LegacyPesde(_) => {
							if *ty != DependencyType::Dev {
								anyhow::bail!(
									"illegal dependency type for {alias}: git and legacy pesde dependencies can only be dev dependencies"
								);
							}

							continue;
						}
					};

					resolved.insert(alias.clone(), (spec, *ty));
				}

				BoundedBTreeMap::new(resolved).unwrap()
			},
		};

		let mut entry_buf = b"# This is a @generated manifest\n".to_vec();
		entry_buf.extend(
			toml::to_string(&resolved_manifest)
				.context("failed to serialize resolved manifest")?
				.into_bytes(),
		);

		let manifest_hash = Hash::digest(HashAlgorithm::default(), &entry_buf);

		let mut zip = async_zip::base::write::ZipFileWriter::new(Vec::new()).force_no_zip64();

		zip.write_entry_whole(
			ZipEntryBuilder::new(MANIFEST_FILE_NAME.into(), async_zip::Compression::Zstd),
			&entry_buf,
		)
		.await?;

		let mut lib_export = exports.lib();
		let mut bin_exports = exports.bin();

		let included = manifest
			.includes
			.matches(vfs.clone() as Arc<dyn Vfs>, member.importer());
		let mut included = std::pin::pin!(included);

		while let Some(path) = included.try_next().await? {
			if &*path == MANIFEST_FILE_NAME {
				continue;
			}

			let relative_path = path
				.strip_prefix::<&str>(member.importer())
				.with_context(|| format!("failed to strip prefix from path {path}"))?;

			lib_export.take_if(|x| x.as_bytes() == relative_path.as_bytes());
			bin_exports.take_if(|x| x.as_bytes() == relative_path.as_bytes());

			let fs_path = vfs.project_dir.join::<&str>(member.importer()).join(&path);

			let metadata = tokio::fs::symlink_metadata(&fs_path)
				.await
				.with_context(|| format!("failed to get metadata for path {path}"))?;

			let file_type = metadata.file_type();

			#[expect(clippy::filetype_is_file)]
			if file_type.is_symlink() {
				return Err(anyhow::anyhow!(
					"symlinks are not allowed in package contents: {path}"
				));
			} else if file_type.is_dir() {
				zip.write_entry_whole(
					ZipEntryBuilder::new(
						format!("{relative_path}/").into(),
						async_zip::Compression::Zstd,
					),
					&[],
				)
				.await?;
			} else if file_type.is_file() {
				let mut file = tokio::fs::File::open(&fs_path)
					.await
					.with_context(|| format!("failed to open file {path}"))?;
				entry_buf.clear();
				file.read_to_end(&mut entry_buf)
					.await
					.with_context(|| format!("failed to read file {path}"))?;

				zip.write_entry_whole(
					ZipEntryBuilder::new(relative_path.into(), async_zip::Compression::Zstd),
					&entry_buf,
				)
				.await?;
			}
		}

		if let Some(lib_export) = lib_export {
			anyhow::bail!("lib export `{lib_export}` was not included in package contents");
		}

		if let Some(bin_export) = bin_exports {
			anyhow::bail!("bin export `{bin_export}` was not included in package contents");
		}

		let zip = zip.close().await.context("failed to finalise zip file")?;

		let archive_hash = Hash::digest(HashAlgorithm::default(), &zip);

		eprintln!("package: {}", resolved_manifest.package.name);
		eprintln!("version: {}", resolved_manifest.package.version);
		if !manifest.package.description.is_empty() {
			eprintln!("description: {}", resolved_manifest.package.description);
		}
		if !manifest.package.license.is_empty() {
			eprintln!("license: {}", resolved_manifest.package.license);
		}
		if let Some(repository) = &manifest.package.repository {
			eprintln!("repository: {repository}");
		}

		eprintln!("manifest hash: {manifest_hash}");
		eprintln!("archive hash: {archive_hash}");

		eprintln!();

		if self.dry_run {
			let out_path = vfs
				.project_dir
				.join::<&str>(member.importer())
				.join("package.peAR");

			tokio::fs::write(&out_path, zip)
				.await
				.with_context(|| format!("failed to write package to {}", out_path.display()))?;

			println!("package written to {}", out_path.display());

			return Ok(());
		}

		eprintln!(
			"WARNING: you are about to push this package to the registry, which will make it publicly available"
		);

		eprintln!();

		print_immutable_protocol_warning();

		eprintln!();

		if !self.yes {
			inquire::Confirm::new("do you want to continue?")
				.with_default(false)
				.with_help_message("this action cannot be undone")
				.prompt()
				.context("failed to read user input")?
				.ok_or_else(|| anyhow::anyhow!("aborted by user"))?;
		}

		let public_key = if let Some(public_key) = &self.public_key {
			&public_key.key
		} else {
			ctx.cli_config()
				.await?
				.identities
				.get(ctx.client().url())
				.with_context(|| {
					format!(
						"registry {} not associated with any identity. use --public-key to specify one",
						ctx.client().url().as_str()
					)
				})?
		};

		let client = ctx.client();

		let scope_id = ScopeId::from(manifest.package.name.scope());

		let scope_head = client.scope_head(&scope_id).await?;

		let (mut scope_state, prev_hash) = if let Some(scope_head) = scope_head {
			let tip_entry = scope_head.tip.entry;

			(
				tip_entry.payload.header().post_state.clone(),
				Some(tip_entry.hash()),
			)
		} else {
			(
				ScopeState {
					owner: public_key.clone(),
					scope_members_root: Default::default(),
					package_versions_root: Default::default(),
					package_deprecations_root: Default::default(),
				},
				None,
			)
		};

		let package_version = PackageVersion {
			pkg: LocalNameId::from(manifest.package.name.local_name()),
			version: manifest.package.version.clone(),
		};

		scope_state.package_versions_root = client
			.tree_transition(
				&scope_state.package_versions_root,
				[TreeUpdate::Insert {
					key: package_version.clone(),
					value: PackageVersionValue {
						archive_hash,
						manifest_hash,
						yank_state: None,
					},
				}],
			)
			.await?;

		let publish_version = PublishVersionUserScopeOpBody {
			header: UserScopeOpHeader {
				signer: public_key.clone(),
				common: ScopeOpHeader {
					scope_id,
					prev_hash: prev_hash.unwrap_or(Blake3Hash([0; _])),
					post_state: scope_state,
				},
			},
			package_version,
			archive_hash,
			manifest_hash,
		};

		let publish_version = self
			.source
			.signed(&ctx, publish_version)
			.await
			.context("failed to sign publish version operation")?;

		let body = PublishPackageBody {
			scope_genesis: if prev_hash.is_none() {
				let scope_genesis = ScopeGenesisPayload {
					scope_id,
					owner: public_key.clone(),
					first_entry_hash: CurrentHash::digest(publish_version.canonical_bytes()),
				};
				let scope_genesis = self
					.source
					.signed(&ctx, scope_genesis)
					.await
					.context("failed to sign scope genesis")?;

				Some(scope_genesis)
			} else {
				None
			},
			publish_version,
		};
		let mut body =
			serde_json::to_vec(&body).context("failed to serialize publish package body")?;
		body.push(b'\n');
		body.extend(zip);

		let response = client
			.publish_package(body)
			.await
			.context("failed to publish package")?;

		println!(
			"successfully published package. server operation timestamp: {}",
			response.published_at
		);

		Ok(())
	}
}
