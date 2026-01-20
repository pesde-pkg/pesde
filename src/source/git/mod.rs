use crate::{
	deser_manifest,
	manifest::{target::Target, Alias, DependencyType, Manifest},
	names::PackageNames,
	reporters::DownloadProgressReporter,
	source::{
		fs::{store_in_cas, FsEntry, PackageFs},
		git::{pkg_ref::GitPackageRef, specifier::GitDependencySpecifier},
		git_index::{read_file, GitBasedSource},
		specifiers::DependencySpecifiers,
		traits::{
			DownloadOptions, GetTargetOptions, PackageRef as _, RefreshOptions, ResolveOptions,
		},
		PackageSource, ResolveResult, VersionId, ADDITIONAL_FORBIDDEN_FILES, IGNORED_DIRS,
		IGNORED_FILES,
	},
	util::hash,
	Project, LOCKFILE_FILE_NAME, MANIFEST_FILE_NAME,
};
use fs_err::tokio as fs;
use gix::{bstr::BStr, traverse::tree::Recorder, ObjectId, Url};
use relative_path::RelativePathBuf;
use semver::{BuildMetadata, Version};
use std::{
	collections::{BTreeMap, BTreeSet},
	fmt::Debug,
	hash::Hash,
	path::PathBuf,
};
use tokio::task::{spawn_blocking, JoinSet};
use tracing::instrument;

/// The Git package reference
pub mod pkg_ref;
/// The Git dependency specifier
pub mod specifier;

/// The Git package source
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct GitPackageSource {
	repo_url: Url,
}

impl GitBasedSource for GitPackageSource {
	fn path(&self, project: &Project) -> PathBuf {
		project
			.data_dir()
			.join("git_repos")
			.join(hash(self.as_bytes()))
	}

	fn repo_url(&self) -> &Url {
		&self.repo_url
	}
}

impl GitPackageSource {
	/// Creates a new Git package source
	#[must_use]
	pub fn new(repo_url: Url) -> Self {
		Self { repo_url }
	}

	fn as_bytes(&self) -> Vec<u8> {
		self.repo_url.to_bstring().to_vec()
	}
}

fn transform_pesde_dependencies(
	manifest: &Manifest,
	repo_url: &Url,
	rev: &str,
	root_tree: &gix::Tree,
) -> Result<BTreeMap<Alias, (DependencySpecifiers, DependencyType)>, errors::ResolveError> {
	let dependencies = manifest
		.all_dependencies()
		.map_err(|e| errors::ResolveError::CollectDependencies(Box::new(repo_url.clone()), e))?;

	dependencies
		.into_iter()
		.map(|(alias, (mut spec, ty))| {
			match &mut spec {
				DependencySpecifiers::Pesde(specifier) => {
					specifier.index = manifest
						.indices
						.get(&specifier.index)
						.ok_or_else(|| {
							errors::ResolveError::PesdeIndexNotFound(
								specifier.index.clone(),
								Box::new(repo_url.clone()),
							)
						})?
						.to_string();
				}
				#[cfg(feature = "wally-compat")]
				DependencySpecifiers::Wally(specifier) => {
					specifier.index = manifest
						.wally_indices
						.get(&specifier.index)
						.ok_or_else(|| {
							errors::ResolveError::WallyIndexNotFound(
								specifier.index.clone(),
								Box::new(repo_url.clone()),
							)
						})?
						.to_string();
				}
				DependencySpecifiers::Git(_) => {}
				DependencySpecifiers::Workspace(specifier) => {
					let lockfile = read_file(root_tree, [LOCKFILE_FILE_NAME]).map_err(|e| {
						errors::ResolveError::ReadLockfile(Box::new(repo_url.clone()), e)
					})?;

					let lockfile = match lockfile {
						Some(l) => match crate::lockfile::parse_lockfile(&l) {
							Ok(l) => l,
							Err(e) => {
								return Err(errors::ResolveError::ParseLockfile(
									Box::new(repo_url.clone()),
									e,
								))
							}
						},
						None => {
							return Err(errors::ResolveError::NoLockfile(Box::new(
								repo_url.clone(),
							)))
						}
					};

					let target = specifier.target.unwrap_or(manifest.target.kind());

					let path = lockfile
						.workspace
						.get(&specifier.name)
						.and_then(|targets| targets.get(&target))
						.ok_or_else(|| {
							errors::ResolveError::NoPathForWorkspaceMember(
								specifier.name.to_string(),
								target,
								Box::new(repo_url.clone()),
							)
						})?
						.clone();

					spec = DependencySpecifiers::Git(GitDependencySpecifier {
						repo: repo_url.clone(),
						rev: rev.to_string(),
						path: Some(path),
					});
				}
				DependencySpecifiers::Path(_) => {
					return Err(errors::ResolveError::Path(Box::new(repo_url.clone())));
				}
			}

			Ok((alias, (spec, ty)))
		})
		.collect()
}

impl PackageSource for GitPackageSource {
	type Specifier = GitDependencySpecifier;
	type Ref = GitPackageRef;
	type RefreshError = crate::source::git_index::errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetTargetError = errors::GetTargetError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, options: &RefreshOptions) -> Result<(), Self::RefreshError> {
		GitBasedSource::refresh(self, options).await
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		specifier: &Self::Specifier,
		options: &ResolveOptions,
	) -> Result<ResolveResult<Self::Ref>, Self::ResolveError> {
		let ResolveOptions { project, .. } = options;

		let path = self.path(project);
		let repo_url = self.repo_url.clone();
		let specifier = specifier.clone();

		let (name, version_id, dependencies, tree_id) = spawn_blocking(move || {
			let repo = gix::open(path).map_err(|e| {
				errors::ResolveError::OpenRepo(Box::new(repo_url.clone()), Box::new(e))
			})?;
			let rev = repo
				.rev_parse_single(BStr::new(&specifier.rev))
				.map_err(|e| {
					errors::ResolveError::ParseRev(
						specifier.rev.clone(),
						Box::new(repo_url.clone()),
						Box::new(e),
					)
				})?;

			// TODO: possibly use the search algorithm from src/main.rs to find the workspace root

			let root_tree = rev
				.object()
				.map_err(|e| errors::ResolveError::ParseRevToObject(Box::new(repo_url.clone()), e))?
				.peel_to_tree()
				.map_err(|e| {
					errors::ResolveError::ParseObjectToTree(Box::new(repo_url.clone()), e)
				})?;

			let tree = if let Some(path) = &specifier.path {
				root_tree
					.lookup_entry_by_path(path.as_str())
					.map_err(|e| {
						errors::ResolveError::ReadTreeEntry(
							Box::new(repo_url.clone()),
							path.clone(),
							e,
						)
					})?
					.ok_or_else(|| {
						errors::ResolveError::NoEntryAtPath(
							Box::new(repo_url.clone()),
							path.clone(),
						)
					})?
					.object()
					.map_err(|e| {
						errors::ResolveError::ParseEntryToObject(Box::new(repo_url.clone()), e)
					})?
					.peel_to_tree()
					.map_err(|e| {
						errors::ResolveError::ParseObjectToTree(Box::new(repo_url.clone()), e)
					})?
			} else {
				root_tree.clone()
			};
			let version = Version {
				major: 0,
				minor: 0,
				patch: 0,
				build: BuildMetadata::EMPTY,
				pre: tree.id.to_string().parse().unwrap(),
			};

			let manifest = match read_file(&tree, [MANIFEST_FILE_NAME])
				.map_err(|e| errors::ResolveError::ReadManifest(Box::new(repo_url.clone()), e))?
			{
				Some(m) => match toml::from_str::<Manifest>(&m) {
					Ok(m) => Some(m),
					Err(e) => {
						return Err(errors::ResolveError::DeserManifest(
							Box::new(repo_url.clone()),
							e,
						))
					}
				},
				None => None,
			};

			#[cfg(feature = "wally-compat")]
			let Some(manifest) = manifest
			else {
				use crate::{
					manifest::target::TargetKind,
					source::wally::{
						compat_util::WALLY_MANIFEST_FILE_NAME,
						manifest::{Realm, WallyManifest},
					},
				};

				let manifest = read_file(&tree, [WALLY_MANIFEST_FILE_NAME]).map_err(|e| {
					errors::ResolveError::ReadManifest(Box::new(repo_url.clone()), e)
				})?;

				let Some(manifest) = manifest else {
					return Err(errors::ResolveError::NoManifest(Box::new(repo_url.clone())));
				};

				let manifest = match toml::from_str::<WallyManifest>(&manifest) {
					Ok(manifest) => manifest,
					Err(e) => {
						return Err(errors::ResolveError::DeserManifest(
							Box::new(repo_url.clone()),
							e,
						))
					}
				};
				let dependencies = manifest.all_dependencies().map_err(|e| {
					errors::ResolveError::CollectDependencies(Box::new(repo_url.clone()), e)
				})?;

				return Ok((
					PackageNames::Wally(manifest.package.name),
					VersionId(
						version,
						match manifest.package.realm {
							Realm::Shared => TargetKind::Roblox,
							Realm::Server => TargetKind::RobloxServer,
						},
					),
					dependencies,
					tree.id.to_string(),
				));
			};
			#[cfg(not(feature = "wally-compat"))]
			let Some(manifest) = manifest
			else {
				return Err(errors::ResolveError::NoManifest(Box::new(repo_url.clone())));
			};

			let dependencies =
				transform_pesde_dependencies(&manifest, &repo_url, &specifier.rev, &root_tree)?;

			Ok((
				PackageNames::Pesde(manifest.name),
				VersionId(version, manifest.target.kind()),
				dependencies,
				tree.id.to_string(),
			))
		})
		.await
		.unwrap()?;

		let new_structure = matches!(name, PackageNames::Pesde(_));

		Ok((
			name,
			BTreeMap::from([(
				version_id,
				GitPackageRef {
					repo: self.repo_url.clone(),
					tree_id,
					new_structure,
					dependencies,
				},
			)]),
			BTreeSet::new(),
		))
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		pkg_ref: &Self::Ref,
		options: &DownloadOptions<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let DownloadOptions {
			project, reporter, ..
		} = options;

		let index_file = project
			.cas_dir()
			.join("git_index")
			.join(hash(self.as_bytes()))
			.join(&pkg_ref.tree_id);

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}#{}",
					pkg_ref.repo,
					pkg_ref.tree_id
				);
				reporter.report_done();
				return toml::from_str::<PackageFs>(&s).map_err(|e| {
					errors::DownloadError::DeserializeFile(Box::new(self.repo_url.clone()), e)
				});
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadError::Io(e)),
		}

		let path = self.path(project);
		let repo_url = self.repo_url.clone();
		let tree_id = match pkg_ref.tree_id.parse::<ObjectId>() {
			Ok(oid) => oid,
			Err(e) => return Err(errors::DownloadError::ParseTreeId(Box::new(repo_url), e)),
		};

		let records = spawn_blocking(move || {
			let repo = gix::open(path)
				.map_err(|e| errors::DownloadError::OpenRepo(Box::new(repo_url.clone()), e))?;

			let mut recorder = Recorder::default();

			let object = match repo.find_object(tree_id) {
				Ok(object) => object,
				Err(e) => {
					return Err(errors::DownloadError::OidToTree(
						tree_id,
						Box::new(repo_url),
						e,
					))
				}
			};

			let tree = match object.peel_to_tree() {
				Ok(tree) => tree,
				Err(e) => {
					return Err(errors::DownloadError::ParseObjectToTree(
						Box::new(repo_url),
						e,
					))
				}
			};

			if let Err(e) = tree.traverse().breadthfirst(&mut recorder) {
				return Err(errors::DownloadError::TraverseTree(Box::new(repo_url), e));
			}

			recorder
				.records
				.into_iter()
				.filter(|entry| {
					// we do not support submodules, so we filter them out so
					// find_object does not error
					entry.mode.kind() != gix::object::tree::EntryKind::Commit
				})
				.map(|entry| {
					let object = repo.find_object(entry.oid).map_err(|e| {
						errors::DownloadError::ParseOidToObject(
							entry.oid,
							Box::new(repo_url.clone()),
							e,
						)
					})?;

					Ok::<_, errors::DownloadError>((
						RelativePathBuf::from(entry.filepath.to_string()),
						if matches!(object.kind, gix::object::Kind::Tree) {
							None
						} else {
							Some(object.data.clone())
						},
					))
				})
				.collect::<Result<Vec<_>, _>>()
		})
		.await
		.unwrap()?;

		let mut tasks = records
			.into_iter()
			.filter(|(path, contents)| {
				let name = path.file_name().unwrap_or("");
				if contents.is_none() {
					return !IGNORED_DIRS.contains(&name);
				}

				if IGNORED_FILES.contains(&name) {
					return false;
				}

				if pkg_ref.use_new_structure() && ADDITIONAL_FORBIDDEN_FILES.contains(&name) {
					tracing::debug!(
						"removing {name} from {}#{} at {path} - using new structure",
						pkg_ref.repo,
						pkg_ref.tree_id
					);
					return false;
				}

				true
			})
			.map(|(path, contents)| {
				let project = project.clone();

				async move {
					let Some(contents) = contents else {
						return Ok::<_, errors::DownloadError>((path, FsEntry::Directory));
					};

					let hash = store_in_cas(project.cas_dir(), contents.as_slice()).await?;

					Ok((path, FsEntry::File(hash)))
				}
			})
			.collect::<JoinSet<_>>();

		let mut entries = BTreeMap::new();

		while let Some(res) = tasks.join_next().await {
			let (path, entry) = res.unwrap()?;
			entries.insert(path, entry);
		}

		let fs = PackageFs::Cas(entries);

		if let Some(parent) = index_file.parent() {
			fs::create_dir_all(parent).await?;
		}

		fs::write(
			&index_file,
			toml::to_string(&fs).map_err(|e| {
				errors::DownloadError::SerializeIndex(Box::new(self.repo_url.clone()), e)
			})?,
		)
		.await
		.map_err(errors::DownloadError::Io)?;

		reporter.report_done();

		Ok(fs)
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_target(
		&self,
		pkg_ref: &Self::Ref,
		options: &GetTargetOptions,
	) -> Result<Target, Self::GetTargetError> {
		if !pkg_ref.new_structure {
			#[cfg(feature = "wally-compat")]
			return crate::source::wally::compat_util::get_target(options)
				.await
				.map_err(Into::into);
			#[cfg(not(feature = "wally-compat"))]
			panic!("wally-compat feature is not enabled, and package is a wally package");
		}

		deser_manifest(&options.path)
			.await
			.map(|m| m.target)
			.map_err(Into::into)
	}
}

/// Errors that can occur when interacting with the Git package source
pub mod errors {
	use crate::manifest::target::TargetKind;
	use gix::ObjectId;
	use relative_path::RelativePathBuf;
	use thiserror::Error;

	/// Errors that can occur when resolving a package from a Git package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ResolveError {
		/// An error occurred opening the Git repository
		#[error("error opening Git repository for url {0}")]
		OpenRepo(Box<gix::Url>, #[source] Box<gix::open::Error>),

		/// An error occurred parsing rev
		#[error("error parsing rev {0} for repository {1}")]
		ParseRev(
			String,
			Box<gix::Url>,
			#[source] Box<gix::revision::spec::parse::single::Error>,
		),

		/// An error occurred parsing rev to object
		#[error("error parsing rev to object for repository {0}")]
		ParseRevToObject(Box<gix::Url>, #[source] gix::object::find::existing::Error),

		/// An error occurred parsing object to tree
		#[error("error parsing object to tree for repository {0}")]
		ParseObjectToTree(Box<gix::Url>, #[source] gix::object::peel::to_kind::Error),

		/// An error occurred reading the manifest
		#[error("error reading manifest of repository {0}")]
		ReadManifest(
			Box<gix::Url>,
			#[source] crate::source::git_index::errors::ReadFile,
		),

		/// An error occurred collecting all manifest dependencies
		#[error("error collecting all manifest dependencies for repository {0}")]
		CollectDependencies(
			Box<gix::Url>,
			#[source] crate::manifest::errors::AllDependenciesError,
		),

		/// An error occurred deserializing a manifest
		#[error("error deserializing manifest for repository {0}")]
		DeserManifest(Box<gix::Url>, #[source] toml::de::Error),

		/// No manifest was found
		#[error("no manifest found in repository {0}")]
		NoManifest(Box<gix::Url>),

		/// A pesde index was not found in the manifest
		#[error("pesde index {0} not found in manifest for repository {1}")]
		PesdeIndexNotFound(String, Box<gix::Url>),

		/// A Wally index was not found in the manifest
		#[error("wally index {0} not found in manifest for repository {1}")]
		WallyIndexNotFound(String, Box<gix::Url>),

		/// An error occurred reading a tree entry
		#[error("error reading tree entry for repository {0} at {1}")]
		ReadTreeEntry(
			Box<gix::Url>,
			RelativePathBuf,
			#[source] gix::object::find::existing::Error,
		),

		/// No entry was found at the specified path
		#[error("no entry found at path {1} in repository {0}")]
		NoEntryAtPath(Box<gix::Url>, RelativePathBuf),

		/// An error occurred parsing an entry to object
		#[error("error parsing an entry to object for repository {0}")]
		ParseEntryToObject(Box<gix::Url>, #[source] gix::object::find::existing::Error),

		/// An error occurred reading the lockfile
		#[error("error reading lockfile for repository {0}")]
		ReadLockfile(
			Box<gix::Url>,
			#[source] crate::source::git_index::errors::ReadFile,
		),

		/// An error occurred while parsing the lockfile
		#[error("error parsing lockfile for repository {0}")]
		ParseLockfile(
			Box<gix::Url>,
			#[source] crate::lockfile::errors::ParseLockfileError,
		),

		/// The repository is missing a lockfile
		#[error("no lockfile found in repository {0}")]
		NoLockfile(Box<gix::Url>),

		/// No path for a workspace member was found in the lockfile
		#[error("no path found for workspace member {0} {1} in lockfile for repository {2}")]
		NoPathForWorkspaceMember(String, TargetKind, Box<gix::Url>),

		/// The package depends on a path package
		#[error("the package {0} depends on a path package")]
		Path(Box<gix::Url>),
	}

	/// Errors that can occur when downloading a package from a Git package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DownloadError {
		/// An error occurred deserializing a file
		#[error("error deserializing file in repository {0}")]
		DeserializeFile(Box<gix::Url>, #[source] toml::de::Error),

		/// An error occurred interacting with the file system
		#[error("error interacting with the file system")]
		Io(#[from] std::io::Error),

		/// An error occurred while creating a Wally target
		#[cfg(feature = "wally-compat")]
		#[error("error creating Wally target")]
		GetTarget(#[from] crate::source::wally::compat_util::errors::GetTargetError),

		/// An error occurred opening the Git repository
		#[error("error opening Git repository for url {0}")]
		OpenRepo(Box<gix::Url>, #[source] gix::open::Error),

		/// An error occurred while traversing the tree
		#[error("error traversing tree for repository {0}")]
		TraverseTree(
			Box<gix::Url>,
			#[source] gix::traverse::tree::breadthfirst::Error,
		),

		/// Getting the tree by object id failed
		#[error("error getting tree from object id {0} for repository {1}")]
		OidToTree(
			ObjectId,
			Box<gix::Url>,
			#[source] gix::object::find::existing::Error,
		),

		/// An error occurred parsing an object id to object
		#[error("error parsing object id {0} to object for repository {1}")]
		ParseOidToObject(
			ObjectId,
			Box<gix::Url>,
			#[source] gix::object::find::existing::Error,
		),

		/// An error occurred parsing object to tree
		#[error("error parsing object to tree for repository {0}")]
		ParseObjectToTree(Box<gix::Url>, #[source] gix::object::peel::to_kind::Error),

		/// An error occurred while serializing the index file
		#[error("error serializing the index file for repository {0}")]
		SerializeIndex(Box<gix::Url>, #[source] toml::ser::Error),

		/// An error occurred while parsing tree_id to ObjectId
		#[error("error parsing tree_id to ObjectId for repository {0}")]
		ParseTreeId(Box<gix::Url>, #[source] gix::hash::decode::Error),
	}

	/// Errors that can occur when getting a target from a Git package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum GetTargetError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred while creating a Wally target
		#[cfg(feature = "wally-compat")]
		#[error("error creating Wally target")]
		GetTarget(#[from] crate::source::wally::compat_util::errors::GetTargetError),
	}
}
