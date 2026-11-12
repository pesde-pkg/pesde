use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use futures::AsyncWriteExt as _;
use futures::StreamExt as _;
use futures::stream;
use gix::remote::Direction;
use pesde::__private::str_as_fs_safe;
use pesde::Url;
use pesde::reporters::DownloadProgressReporter;
use pesde::source::DependencySpecifier;
use pesde::source::GitDependencySpecifier;
use pesde::source::InspectResult;
use pesde::source::PackageId;
use pesde::source::PackageSource;
use pesde::source::ResolveResult;
use pesde::source::SourceError;
use pesde::vfs::PackageStorer;
use pesde::vfs::VfsPath;
use tokio::io::AsyncReadExt as _;
use tokio::task::spawn_blocking;

use crate::cli::dirs::cache_dir;
use crate::cli::source::local_package::LocalPackage;
use crate::cli::vfs::FsVfs;

const REFSPECS: &[&str] = &[
	"+HEAD:refs/pesde/head",
	"+refs/heads/*:refs/heads/*",
	"+refs/tags/*:refs/tags/*",
];

pub struct GixGitPackageSource {
	repo_url: Url,
	path: Box<Path>,
	vfs: Arc<FsVfs>,
}

struct Checkout {
	dir: Box<Path>,
	package: LocalPackage,
}

impl GixGitPackageSource {
	pub async fn new(repo_url: Url) -> anyhow::Result<Self> {
		let cache_dir = cache_dir()?;
		let path = cache_dir
			.join("git_repos")
			.join("git_full")
			.join(str_as_fs_safe(repo_url.as_str()))
			.into_boxed_path();

		let vfs = Arc::new(FsVfs::new(cache_dir).await?);

		let context = path.to_path_buf();

		if tokio::fs::try_exists(&path)
			.await
			.with_context(|| format!("error checking if path exists: {}", path.display()))?
		{
			let path = path.clone().to_path_buf();

			spawn_blocking(move || {
				let repo = gix::open_opts(&path, gix::open::Options::isolated())?;
				let remote = repo
					.find_default_remote(Direction::Fetch)
					.context("no default remote found in repository")??;

				remote
					.connect(Direction::Fetch)
					.context("error connecting to remote repository")?
					.prepare_fetch(gix::progress::Discard, Default::default())
					.context("error preparing fetch from remote repository")?
					.receive(gix::progress::Discard, &false.into())
					.context("error reading from remote repository")?;

				anyhow::Ok(())
			})
			.await
			.unwrap()
			.with_context(|| {
				format!(
					"error refreshing git repository of {} at path: {}",
					repo_url,
					context.display()
				)
			})?;
		} else {
			let url = gix::Url::try_from(repo_url.as_str()).context("error parsing git url")?;

			tokio::fs::create_dir_all(&path)
				.await
				.with_context(|| format!("error creating directory {}", path.display()))?;

			let path = path.clone().to_path_buf();

			spawn_blocking(move || {
				gix::clone::PrepareFetch::new(
					url,
					path,
					gix::create::Kind::Bare,
					gix::create::Options::default(),
					gix::open::Options::isolated(),
				)
				.context("error preparing clone of git repository")?
				.configure_remote(|mut remote| {
					remote.replace_refspecs(REFSPECS, Direction::Fetch)?;
					Ok(remote)
				})
				.fetch_only(gix::progress::Discard, &false.into())
				.context("error fetching git repository")
				.map(|_| ())
			})
			.await
			.unwrap()
			.with_context(|| {
				format!(
					"error cloning git repository of {} at path: {}",
					repo_url,
					context.display()
				)
			})?;
		}

		Ok(Self {
			repo_url,
			path,
			vfs,
		})
	}

	async fn locate(&self, package_id: &PackageId) -> anyhow::Result<Checkout> {
		let PackageId::Git(_, revision, subpath) = package_id else {
			anyhow::bail!("invalid package id for git source: {package_id}");
		};

		let commit = gix::ObjectId::from_hex(revision.as_bytes())
			.with_context(|| format!("invalid commit id `{revision}` in package id"))?;

		let checkout_dir = self.vfs.unique_tmp_path();
		tokio::fs::create_dir_all(&checkout_dir)
			.await
			.with_context(|| format!("error creating directory {}", checkout_dir.display()))?;

		let located = async {
			spawn_blocking({
				let repo_path = self.path.clone();
				let dest = checkout_dir.to_path_buf();

				move || {
					let repo = gix::open_opts(&*repo_path, gix::open::Options::isolated())?;

					let tree_id = repo
						.find_commit(commit)
						.context("commit not found, it may have been removed from the remote")?
						.tree_id()
						.context("error reading commit tree id")?;

					let mut index = repo
						.index_from_tree(&tree_id)
						.context("error creating index from tree")?;

					let mut options = repo
						.checkout_options(
							gix::worktree::stack::state::attributes::Source::IdMapping,
						)
						.context("error getting checkout options")?;
					options.destination_is_initially_empty = true;

					let outcome = gix::worktree::state::checkout(
						&mut index,
						dest,
						repo.objects.clone().into_arc()?,
						&gix::progress::Discard,
						&gix::progress::Discard,
						&false.into(),
						options,
					)
					.context("error checking out worktree")?;

					anyhow::ensure!(
						outcome.errors.is_empty(),
						"{} errors occurred while checking out the worktree",
						outcome.errors.len()
					);

					anyhow::Ok(())
				}
			})
			.await
			.unwrap()
			.with_context(|| format!("error checking out {commit} of {}", self.repo_url))?;

			LocalPackage::find(&checkout_dir, Path::new(&subpath.to_string())).await
		}
		.await;

		match located {
			Ok(package) => Ok(Checkout {
				dir: checkout_dir,
				package,
			}),
			Err(e) => {
				remove_checkout(&checkout_dir).await;
				Err(e)
			}
		}
	}
}

#[async_trait]
impl PackageSource for GixGitPackageSource {
	async fn resolve(&self, specifier: &DependencySpecifier) -> Result<ResolveResult, SourceError> {
		let DependencySpecifier::Git(specifier) = specifier else {
			return Err(anyhow::format_err!(
				"invalid dependency specifier for git source: {specifier}"
			)
			.into());
		};

		let GitDependencySpecifier { url, rev, path } = specifier.clone();

		let commit = spawn_blocking({
			let repo_path = self.path.clone();
			let rev = rev.to_string();

			move || {
				let repo = gix::open_opts(&*repo_path, gix::open::Options::isolated())?;

				let spec = if rev.is_empty() {
					"refs/pesde/head"
				} else {
					rev.as_str()
				};

				anyhow::Ok(
					repo.rev_parse_single(spec)
						.context("error parsing rev")?
						.object()
						.context("error reading rev object")?
						.peel_to_commit()
						.context("error peeling rev to commit")?
						.id,
				)
			}
		})
		.await
		.unwrap()
		.with_context(|| format!("error resolving rev `{rev}` in git repository `{url}`"))?;

		let id = PackageId::Git(url, commit.to_string().into(), path);

		Ok(ResolveResult::Found(
			stream::iter(std::iter::once(Ok::<_, SourceError>((id, None)))).boxed(),
		))
	}

	async fn download(
		&self,
		package_id: &PackageId,
		_reporter: Arc<dyn DownloadProgressReporter + 'static>,
		to: &mut dyn PackageStorer,
	) -> Result<(), SourceError> {
		let checkout = self.locate(package_id).await?;
		let package_dir = checkout
			.package
			.workspace_dir
			.join(&**checkout.package.importer);

		let result = async {
			let mut buf = vec![0; 8 * 1024];
			let mut stack = vec![VfsPath::root()];

			while let Some(relative) = stack.pop() {
				let dir = package_dir.join(&relative);
				let mut read_dir = tokio::fs::read_dir(&dir)
					.await
					.with_context(|| format!("error reading directory {}", dir.display()))?;

				while let Some(entry) = read_dir
					.next_entry()
					.await
					.with_context(|| format!("error reading directory {}", dir.display()))?
				{
					let name = entry.file_name().into_string().map_err(|name| {
						anyhow::format_err!(
							"invalid UTF-8 in file name: `{}`",
							name.to_string_lossy()
						)
					})?;

					if name == ".git" {
						continue;
					}

					let entry_path = entry.path();
					let file_type = entry.file_type().await.with_context(|| {
						format!("error reading file type of {}", entry_path.display())
					})?;

					let component = name
						.parse::<VfsPath>()
						.with_context(|| format!("git package path component `{name}`"))?;
					let path = relative.join(&component);

					#[expect(clippy::filetype_is_file)]
					if file_type.is_dir() {
						to.directory_at(path.clone()).await?;
						stack.push(path);
					} else if file_type.is_file() {
						let mut source = tokio::fs::File::open(&entry_path)
							.await
							.with_context(|| format!("error opening {}", entry_path.display()))?;
						let size_hint = source
							.metadata()
							.await
							.with_context(|| {
								format!("error reading metadata of {}", entry_path.display())
							})?
							.len();

						let file = to.file_at(path, size_hint).await?;
						while let n = source
							.read(&mut buf)
							.await
							.with_context(|| format!("error reading {}", entry_path.display()))?
							&& n > 0
						{
							file.write_all(&buf[..n]).await?;
						}
					} else {
						tracing::warn!("skipping unsupported entry `{path}` in git package");
					}
				}
			}

			anyhow::Ok(())
		}
		.await
		.with_context(|| format!("error storing git package `{package_id}`"));

		remove_checkout(&checkout.dir).await;
		result?;

		Ok(())
	}

	async fn inspect(&self, package_id: &PackageId) -> Result<InspectResult, SourceError> {
		let checkout = self.locate(package_id).await?;

		let result = checkout.package.inspect(package_id).await;

		remove_checkout(&checkout.dir).await;

		Ok(result?)
	}
}

async fn remove_checkout(dir: &Path) {
	if let Err(e) = tokio::fs::remove_dir_all(dir).await {
		tracing::warn!(
			"failed to remove temporary checkout `{}`: {e}",
			dir.display()
		);
	}
}
