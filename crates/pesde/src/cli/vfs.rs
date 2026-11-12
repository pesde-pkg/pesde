use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr as _;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use anyhow::Context as _;
use async_stream::try_stream;
use async_trait::async_trait;
use futures::AsyncRead;
use futures::FutureExt as _;
use futures::Stream;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use itertools::EitherOrBoth;
use itertools::Itertools as _;
use pesde::__private::str_as_fs_safe;
use pesde::Importer;
use pesde::MANIFEST_FILE_NAME;
use pesde::hash::Hash;
use pesde::hash::HashAlgorithm;
use pesde::hash::RuntimeHasher;
use pesde::link::StructureKind;
use pesde::source::PackageId;
use pesde::vfs::DirEntry;
use pesde::vfs::PackageStorer;
use pesde::vfs::Vfs;
use pesde::vfs::VfsPath;
use pesde::vfs::glob::VfsGlobset;
use rand::RngExt as _;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::task::JoinSet;
use tokio_util::compat::Compat;
use tokio_util::compat::TokioAsyncReadCompatExt as _;
use tokio_util::compat::TokioAsyncWriteCompatExt as _;
use tracing::instrument;

#[derive(Debug)]
pub struct FsVfs {
	pub(crate) cas_dir: Box<Path>,
	pub(crate) project_dir: Box<Path>,
}

impl FsVfs {
	pub async fn new(for_dir: impl Into<Box<Path>>) -> anyhow::Result<Self> {
		let for_dir = for_dir.into();

		let cas_dir = get_linkable_dir(&for_dir)
			.await
			.with_context(|| {
				format!(
					"failed to find a linkable directory for the project directory {}",
					for_dir.display()
				)
			})?
			.join(".pesde")
			.join("cas")
			.join("v1");

		let vfs = Self {
			cas_dir: cas_dir.into(),
			project_dir: for_dir,
		};

		// save on having to do this everywhere
		let tmp_dir = vfs.tmp_dir_path();
		tokio::fs::create_dir_all(&tmp_dir).await.with_context(|| {
			format!(
				"failed to create parent directory for CAS directory {}",
				tmp_dir.display()
			)
		})?;

		Ok(vfs)
	}

	pub async fn in_project(cwd: PathBuf) -> anyhow::Result<(Self, Importer)> {
		let mut cts = cwd.components();
		let root = cts
			.by_ref()
			.take_while_inclusive(|c| !matches!(c, Component::RootDir))
			.collect::<PathBuf>();
		let cwd = cts.collect::<PathBuf>();

		let (workspace_root, importer) = find_project(&root, &cwd).await?;

		Ok((Self::new(root.join(workspace_root)).await?, importer))
	}

	pub fn from_parts(cas_dir: impl Into<Box<Path>>, project_dir: impl Into<Box<Path>>) -> Self {
		Self {
			cas_dir: cas_dir.into(),
			project_dir: project_dir.into(),
		}
	}
}

pub(crate) async fn find_project(root: &Path, cwd: &Path) -> anyhow::Result<(PathBuf, Importer)> {
	// hacky but this allows us to use the glob matching code
	// cas_dir won't be used as package handles won't
	let vfs = Arc::new(FsVfs {
		cas_dir: Path::new("!!!DUMMY!!!").into(),
		project_dir: root.into(),
	});

	let mut current_path = Some(cwd.to_path_buf());
	let mut project_root = None::<PathBuf>;
	let mut workspace_dir = None::<PathBuf>;

	let mut manifest_buf = Vec::new();

	async fn get_workspace_member(
		buf: &mut Vec<u8>,
		vfs: &Arc<FsVfs>,
		manifest_file: &mut tokio::fs::File,
		at_path: &Path,
		member_path: &Path,
	) -> anyhow::Result<bool> {
		#[derive(Deserialize, Default)]
		#[serde(default)]
		struct Workspace {
			members: VfsGlobset,
		}

		#[derive(Deserialize)]
		struct WorkspaceOnlyManifest {
			#[serde(default)]
			workspace: Workspace,
		}

		let len = manifest_file.read_to_end(buf).await?;
		let manifest: WorkspaceOnlyManifest = toml::from_slice(&buf[..len])?;

		if manifest.workspace.members.is_empty() {
			return Ok(false);
		}

		let at_path = VfsPath::from_path(at_path).context("failed to convert path to VfsPath")?;

		manifest
			.workspace
			.members
			.matches(vfs.clone(), &at_path)
			.try_any(async |p| {
				// comparing std paths by equality is fragile
				p.split('/')
					.zip_longest(member_path.components())
					.all(|elem| {
						let EitherOrBoth::Both(a, b) = elem else {
							return false;
						};

						if let Component::Normal(b) = b {
							a == b
						} else {
							false
						}
					})
			})
			.await
			.map_err(Into::into)
	}

	while let Some(path) = current_path {
		current_path = path.parent().map(Path::to_path_buf);

		if workspace_dir.is_some() && project_root.is_some() {
			break;
		}

		let manifest_path = root.join(&path).join(MANIFEST_FILE_NAME);
		let mut manifest = match tokio::fs::File::open(&manifest_path).await {
			Ok(manifest) => manifest,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
			Err(e) => {
				return Err(e).with_context(|| {
					format!("failed to open manifest file {}", manifest_path.display())
				});
			}
		};

		match (project_root.as_ref(), workspace_dir.as_ref()) {
			(Some(project_root), None) => {
				if get_workspace_member(&mut manifest_buf, &vfs, &mut manifest, &path, project_root)
					.await?
				{
					workspace_dir = Some(path);
				}
			}

			(None, None) => {
				// initializing a new member of a workspace
				if get_workspace_member(&mut manifest_buf, &vfs, &mut manifest, &path, cwd).await? {
					workspace_dir = Some(path);
					break;
				}

				project_root = Some(path);
			}

			(_, _) => unreachable!(),
		}
	}

	// we mustn't expect the project root to be found, as that would
	// disable the ability to run pesde in a non-project directory (for example to init it)
	let project_root = project_root.unwrap_or_else(|| cwd.to_path_buf());
	let workspace_root = workspace_dir.unwrap_or_else(|| project_root.clone());

	let importer = Importer::from_path(VfsPath::from_path(
		project_root.strip_prefix(&workspace_root)?,
	)?);

	Ok((workspace_root, importer))
}

#[instrument(level = "trace")]
async fn get_linkable_dir(path: &Path) -> anyhow::Result<PathBuf> {
	let mut curr_path = PathBuf::with_capacity(path.as_os_str().len());

	let (file_in_current, temp_file_name) = {
		let mut rand = rand::rng();
		(
			path.join(format!(".pesde-linkable-dir-test-{}", rand.random::<u64>())),
			format!(".pesde-linkable-dir-test-{}", rand.random::<u64>()),
		)
	};

	tokio::fs::File::create(&file_in_current)
		.await
		.with_context(|| format!("failed to create temporary file in `{}`", path.display()))?;

	// C: and \ are different components on Windows
	#[cfg(windows)]
	let components = path.components().map(|c| {
		let mut path = c.as_os_str().to_os_string();
		if let std::path::Component::Prefix(_) = c {
			path.push(std::path::MAIN_SEPARATOR_STR);
		}

		path
	});
	#[cfg(not(windows))]
	let components = path.components().map(|c| c.as_os_str().to_os_string());

	let result = 'result: {
		for component in components {
			curr_path.push(component);

			let try_path = curr_path.join(&temp_file_name);

			if tokio::fs::hard_link(&file_in_current, &try_path)
				.await
				.is_ok()
			{
				if let Err(err) = tokio::fs::remove_file(&try_path).await {
					tracing::warn!(
						"failed to remove temporary file at `{}`: {err}",
						try_path.display()
					);
				}

				break 'result Ok(curr_path);
			}
		}

		Err(anyhow::format_err!(
			"couldn't find a linkable directory for any point in `{}`",
			curr_path.display()
		))
	};

	tokio::fs::remove_file(&file_in_current)
		.await
		.with_context(|| format!("failed to remove temporary file in `{}`", path.display()))?;

	result
}

#[async_trait]
impl Vfs for FsVfs {
	async fn read_file(
		self: Arc<Self>,
		path: &VfsPath,
	) -> std::io::Result<Box<dyn AsyncRead + Unpin + Send>> {
		let path = self.project_dir.join(path);
		let file = tokio::fs::File::open(&path)
			.await
			.map_err(|e| contextualize_io_error(e, &path))?;
		Ok(Box::new(file.compat()))
	}

	async fn write_file(self: Arc<Self>, path: &VfsPath, contents: &[u8]) -> std::io::Result<()> {
		let temp_path = self.unique_tmp_path();
		tokio::fs::write(&temp_path, contents)
			.await
			.map_err(|e| contextualize_io_error(e, &temp_path))?;

		let path = self.project_dir.join(path);
		if let Some(parent) = path.parent() {
			tokio::fs::create_dir_all(parent)
				.await
				.map_err(|e| contextualize_io_error(e, parent))?;
		}
		tokio::fs::rename(&temp_path, &path)
			.await
			.map_err(|e| contextualize_io_error(e, &path))?;

		Ok(())
	}

	async fn exists(self: Arc<Self>, path: &VfsPath) -> std::io::Result<bool> {
		let path = self.project_dir.join(path);
		tokio::fs::try_exists(&path)
			.await
			.map_err(|e| contextualize_io_error(e, &path))
	}

	fn read_dir(self: Arc<Self>, path: VfsPath) -> BoxStream<'static, std::io::Result<DirEntry>> {
		let path = self.project_dir.join(path);
		try_stream!({
			let mut read_dir = match tokio::fs::read_dir(&path).await {
				Ok(dir) => dir,
				Err(e) => {
					Err(contextualize_io_error(e, &path))?;
					return;
				}
			};

			while let Some(entry) = read_dir.next_entry().await.transpose() {
				let entry = match entry {
					Ok(entry) => entry,
					Err(e) => {
						Err(contextualize_io_error(e, &path))?;
						return;
					}
				};

				yield DirEntry {
					name: entry
						.file_name()
						.into_string()
						.map_err(|name| {
							std::io::Error::new(
								std::io::ErrorKind::InvalidData,
								format!("invalid UTF-8 in file name: `{}`", name.to_string_lossy()),
							)
						})?
						.parse()
						.map_err(std::io::Error::other)?,
					is_dir: match entry.metadata().await {
						Ok(metadata) => metadata.is_dir(),
						Err(e) => {
							Err(contextualize_io_error(e, &path))?;
							return;
						}
					},
				};
			}
		})
		.boxed()
	}

	async fn remove_dir_all(self: Arc<Self>, path: &VfsPath) -> std::io::Result<()> {
		let path = self.project_dir.join(path);
		tokio::fs::remove_dir_all(&path)
			.await
			.map_err(|e| contextualize_io_error(e, &path))
	}

	async fn copy_package_to(
		self: Arc<Self>,
		immutable: bool,
		package: &PackageId,
		structure: StructureKind,
		destination: &VfsPath,
	) -> std::io::Result<()> {
		let manifest_path = match self.package_manifest(package) {
			CasOrLink::Cas(path) => path,
			CasOrLink::Link(path) => {
				self.link_path_package_to(&path, structure, destination)
					.await?;
				return Ok(());
			}
		};

		let manifest_file = tokio::fs::read(&manifest_path)
			.await
			.map_err(|e| contextualize_io_error(e, &manifest_path))?;

		let manifest: CasPackageLayoutManifest =
			serde_json::from_slice(&manifest_file).map_err(std::io::Error::other)?;

		self.copy_existing_package_to(manifest, immutable, destination, self.unique_tmp_path())
			.await
	}

	async fn copy_and_maybe_store_package(
		self: Arc<Self>,
		immutable: bool,
		package: &PackageId,
		structure: StructureKind,
		destination: &VfsPath,
	) -> std::io::Result<Option<Box<dyn PackageStorer>>> {
		let manifest_path = match self.package_manifest(package) {
			CasOrLink::Cas(path) => path,
			CasOrLink::Link(path) => {
				self.link_path_package_to(&path, structure, destination)
					.await?;
				return Ok(None);
			}
		};

		match tokio::fs::read(&manifest_path).await {
			Ok(manifest_file) => {
				let manifest: CasPackageLayoutManifest =
					serde_json::from_slice(&manifest_file).map_err(std::io::Error::other)?;

				self.copy_existing_package_to(
					manifest,
					immutable,
					destination,
					self.unique_tmp_path(),
				)
				.await?;

				return Ok(None);
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => {
				return Err(contextualize_io_error(e, &manifest_path));
			}
		}

		let storer = CasPackageStorer {
			manifest_path,
			tmp_file_path: self.unique_tmp_path(),
			vfs: self,
			immutable,
			destination: destination.clone(),
			manifest: CasPackageLayoutManifest {
				hash_algorithm: HashAlgorithm::default(),
				entries: BTreeMap::new(),
			},
			current_file: None,
		};

		Ok(Some(Box::new(storer)))
	}
}

enum CasOrLink {
	Cas(Box<Path>),
	Link(PathBuf),
}

impl FsVfs {
	fn tmp_dir_path(&self) -> Box<Path> {
		self.cas_dir.join(".tmp").into()
	}

	pub(crate) fn unique_tmp_path(&self) -> Box<Path> {
		self.tmp_dir_path()
			.join(format!("tmp-{}", rand::random::<u64>()))
			.into()
	}

	async fn link_path_package_to(
		&self,
		path: &Path,
		structure: StructureKind,
		destination: &VfsPath,
	) -> std::io::Result<()> {
		let tmp_dir = self.unique_tmp_path();
		tokio::fs::create_dir_all(&tmp_dir)
			.await
			.map_err(|e| contextualize_io_error(e, &tmp_dir))?;

		let mut stack = vec![PathBuf::new()];

		while let Some(relative) = stack.pop() {
			let absolute = path.join(&relative);
			let mut read_dir = tokio::fs::read_dir(&absolute)
				.await
				.map_err(|e| contextualize_io_error(e, &absolute))?;

			while let Some(entry) = read_dir.next_entry().await.transpose() {
				let entry = match entry {
					Ok(entry) => entry,
					Err(e) => {
						return Err(contextualize_io_error(e, &absolute));
					}
				};

				let name = entry.file_name().into_string().map_err(|name| {
					std::io::Error::new(
						std::io::ErrorKind::InvalidData,
						format!("invalid UTF-8 in file name: `{}`", name.to_string_lossy()),
					)
				})?;

				let destination = tmp_dir.join(&relative).join(&name);

				let is_dir = entry
					.metadata()
					.await
					.map_err(|e| contextualize_io_error(e, &absolute.join(&name)))?
					.is_dir();
				if is_dir {
					if structure
						.ignored_directories()
						.iter()
						.any(|n| **n == relative)
					{
						continue;
					}

					tokio::fs::create_dir_all(&destination)
						.await
						.map_err(|e| contextualize_io_error(e, &destination))?;

					stack.push(relative.join(&name));
					continue;
				}

				if structure.ignored_files().iter().any(|n| **n == relative) {
					continue;
				}

				// TODO: use reflinking if possible
				tokio::fs::hard_link(&entry.path(), &destination)
					.await
					.map_err(|e| contextualize_io_error(e, &destination))?;
			}
		}

		let destination = self.project_dir.join(destination);
		if let Some(parent) = destination.parent() {
			tokio::fs::create_dir_all(parent)
				.await
				.map_err(|e| contextualize_io_error(e, parent))?;
		}

		tokio::fs::rename(&tmp_dir, &destination)
			.await
			.map_err(|e| contextualize_io_error(e, &destination))?;

		Ok(())
	}

	async fn copy_existing_package_to(
		&self,
		manifest: CasPackageLayoutManifest,
		immutable: bool,
		destination: &VfsPath,
		temp_path: Box<Path>,
	) -> std::io::Result<()> {
		let destination = self.project_dir.join(destination);
		match tokio::fs::remove_dir_all(&destination).await {
			Ok(_) => {}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(contextualize_io_error(e, &destination)),
		}

		tokio::fs::create_dir_all(&temp_path)
			.await
			.map_err(|e| contextualize_io_error(e, &temp_path))?;

		for (path, entry) in manifest.entries {
			let dest_path = temp_path.join(&path);

			let dir_path = match &entry {
				None => Some(&*dest_path),
				Some(_) => dest_path.parent(),
			};

			if let Some(dir_path) = dir_path {
				tokio::fs::create_dir_all(dir_path)
					.await
					.map_err(|e| contextualize_io_error(e, dir_path))?;
			}

			if let Some(hash) = entry {
				let src_path = self.file_path(manifest.hash_algorithm, &hash);

				// TODO: utilise reflinking
				if immutable {
					tokio::fs::hard_link(&src_path, &dest_path)
						.await
						.map_err(|e| contextualize_io_error(e, &dest_path))?;
				} else {
					tokio::fs::copy(&src_path, &dest_path)
						.await
						.map_err(|e| contextualize_io_error(e, &dest_path))?;
				}
			}
		}

		if let Some(parent) = destination.parent() {
			tokio::fs::create_dir_all(parent)
				.await
				.map_err(|e| contextualize_io_error(e, parent))?;
		}
		tokio::fs::rename(&temp_path, &destination)
			.await
			.map_err(|e| contextualize_io_error(e, &destination))?;

		Ok(())
	}

	fn cas_indices_dir(&self) -> PathBuf {
		self.cas_dir.join("indices")
	}

	fn package_manifest(&self, package: &PackageId) -> CasOrLink {
		let mut path = self.cas_indices_dir();

		match package {
			PackageId::Pesde(registry, name, version) => {
				path.push("pesde");
				path.push(str_as_fs_safe(&registry.to_string()));
				path.push(str_as_fs_safe(name.scope()));
				path.push(str_as_fs_safe(name.local_name()));
				path.push(str_as_fs_safe(&version.to_string()));
			}
			PackageId::Wally(index, name, version) => {
				path.push("wally");
				path.push(str_as_fs_safe(&index.to_string()));
				path.push(str_as_fs_safe(name.scope()));
				path.push(str_as_fs_safe(name.local_name()));
				path.push(str_as_fs_safe(&version.to_string()));
			}
			PackageId::LegacyPesde(index, name, version, target) => {
				path.push("legacy_pesde");
				path.push(str_as_fs_safe(&index.to_string()));
				path.push(str_as_fs_safe(name.scope()));
				path.push(str_as_fs_safe(name.local_name()));
				path.push(str_as_fs_safe(&version.to_string()));
				path.push(str_as_fs_safe(&target.to_string()));
			}
			PackageId::Git(url, revision, subpath) => {
				path.push("git");
				path.push(str_as_fs_safe(&url.to_string()));
				path.push(str_as_fs_safe(revision));
				// use : because it is an invalid character in a path, so we will not collide with any other paths
				path.push(str_as_fs_safe(if subpath.is_empty() {
					":"
				} else {
					subpath
				}));
			}
			PackageId::Workspace(importer) => {
				return CasOrLink::Link(self.project_dir.join(&**importer));
			}
		}

		CasOrLink::Cas(path.into())
	}

	fn cas_parts_dir(&self) -> PathBuf {
		self.cas_dir.join("by_algorithm")
	}

	fn file_path(&self, algo: HashAlgorithm, hash: &str) -> Box<Path> {
		let (prefix, rest) = hash.split_at(2);
		self.cas_parts_dir()
			.join(algo.to_string())
			.join(prefix)
			.join(rest)
			.into()
	}
}

struct CasPackageStorer {
	vfs: Arc<FsVfs>,
	immutable: bool,
	destination: VfsPath,
	manifest_path: Box<Path>,
	tmp_file_path: Box<Path>,
	manifest: CasPackageLayoutManifest,
	current_file: Option<CasFileWriter>,
}

#[async_trait]
impl PackageStorer for CasPackageStorer {
	async fn directory_at(&mut self, path: VfsPath) -> std::io::Result<()> {
		self.manifest.entries.insert(path, None);

		Ok(())
	}

	async fn file_at<'a>(
		&'a mut self,
		path: VfsPath,
		size_hint: u64,
	) -> std::io::Result<&'a mut (dyn futures::io::AsyncWrite + Unpin + Send + 'a)> {
		self.flush_current().await?;

		let file = tokio::fs::File::create_new(&self.tmp_file_path)
			.await
			.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;

		file.set_len(size_hint)
			.await
			.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;

		let file = self.current_file.insert(CasFileWriter {
			hasher: self.manifest.hash_algorithm.hasher(),
			handle: file.compat_write(),
			written: 0,
			path,
		});

		Ok(file)
	}

	async fn commit(mut self: Box<Self>) -> std::io::Result<()> {
		self.flush_current().await?;

		let manifest_json = serde_json::to_vec(&self.manifest).map_err(std::io::Error::other)?;

		let mut file = tokio::fs::File::create_new(&self.tmp_file_path)
			.await
			.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;
		file.write_all(&manifest_json)
			.await
			.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;

		file.shutdown()
			.await
			.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;

		file.shutdown()
			.await
			.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;
		drop(file);

		if let Some(parent) = self.manifest_path.parent() {
			tokio::fs::create_dir_all(parent)
				.await
				.map_err(|e| contextualize_io_error(e, parent))?;
		}

		tokio::fs::rename(&self.tmp_file_path, &self.manifest_path)
			.await
			.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;

		// TODO: do this continuously as files are written, rather than at the end
		self.vfs
			.copy_existing_package_to(
				self.manifest,
				self.immutable,
				&self.destination,
				self.tmp_file_path,
			)
			.await?;

		Ok(())
	}
}

impl CasPackageStorer {
	async fn flush_current(&mut self) -> std::io::Result<()> {
		if let Some(writer) = self.current_file.take() {
			let hash = writer.hasher.finalize();
			let file_path = self.vfs.file_path(hash.algorithm(), &hash.encoded());

			self.manifest
				.entries
				.insert(writer.path, Some(hash.encoded().into()));

			let mut handle = writer.handle.into_inner();

			handle
				.set_len(writer.written)
				.await
				.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;

			handle
				.shutdown()
				.await
				.map_err(|e| contextualize_io_error(e, &self.tmp_file_path))?;

			drop(handle);

			if let Some(parent) = file_path.parent() {
				tokio::fs::create_dir_all(parent)
					.await
					.map_err(|e| contextualize_io_error(e, parent))?;
			}

			match tokio::fs::rename(&self.tmp_file_path, file_path).await {
				Ok(_) => {}
				Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
				Err(e) => return Err(contextualize_io_error(e, &self.tmp_file_path)),
			}
		}

		Ok(())
	}
}

struct CasFileWriter {
	hasher: RuntimeHasher,
	handle: Compat<tokio::fs::File>,
	written: u64,
	path: VfsPath,
}

impl futures::io::AsyncWrite for CasFileWriter {
	fn poll_write(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<std::io::Result<usize>> {
		let handle = Pin::new(&mut self.handle);
		let written = futures::ready!(handle.poll_write(cx, buf))?;
		self.hasher.update(&buf[..written]);
		self.written += written as u64;
		Poll::Ready(Ok(written))
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
		let handle = Pin::new(&mut self.handle);
		handle.poll_flush(cx)
	}

	fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
		let handle = Pin::new(&mut self.handle);
		handle.poll_close(cx)
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CasPackageLayoutManifest {
	#[serde(rename = "a")]
	pub hash_algorithm: HashAlgorithm,
	#[serde(rename = "e")]
	pub entries: BTreeMap<VfsPath, Option<Box<str>>>,
}

#[derive(Debug, Error)]
#[error("{1}")]
struct ContextualizeIoError<T>(#[source] std::io::Error, T);

fn contextualize_io_error(e: std::io::Error, path: &Path) -> std::io::Error {
	std::io::Error::new(
		e.kind(),
		ContextualizeIoError(
			e,
			anyhow::format_err!("I/O error while accessing `{}`", path.display()),
		),
	)
}

impl FsVfs {
	pub fn list_cas_packages(
		&self,
	) -> impl Stream<Item = anyhow::Result<(PathBuf, CasPackageLayoutManifest)>> + use<> {
		async fn read_manifest(path: &Path) -> anyhow::Result<Option<CasPackageLayoutManifest>> {
			let contents = match tokio::fs::read(path).await {
				Ok(c) => c,
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
				Err(e) => {
					return Err(e).with_context(|| {
						format!("failed to read cas package file at `{}`", path.display())
					});
				}
			};
			let manifest = serde_json::from_slice(&contents).with_context(|| {
				format!(
					"failed to parse cas package manifest at `{}`",
					path.display()
				)
			})?;
			Ok(Some(manifest))
		}

		let root = self.cas_indices_dir();
		try_stream! {
			let mut stack = vec![root.clone()];
			while let Some(dir) = stack.pop() {
				let mut read_dir = match tokio::fs::read_dir(&dir).await {
					Ok(d) => d,
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
					Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => {
						if let Some(m) = read_manifest(&dir).await? {
							yield (dir, m);
						}
						continue;
					}
					Err(e) => {
						Err(e).with_context(|| {
							format!("failed to read cas package directory at `{}`", dir.display())
						})?;

						unreachable!()
					}
				};

				loop {
					let entry = match read_dir.next_entry().await {
						Ok(Some(entry)) => entry,
						Ok(None) => break,
						Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
						Err(e) => {
							Err(e).with_context(|| {
								format!("failed to read entry in `{}`", dir.display())
							})?;
							unreachable!()
						}
					};

					let path = entry.path();
					match entry.file_type().await {
						Ok(ty) if ty.is_dir() => stack.push(path),
						// we do not want to follow symlinks, they should not appear here at all
						#[expect(clippy::filetype_is_file)]
						Ok(ty) if ty.is_file() => {
							if let Some(m) = read_manifest(&path).await? {
								yield (path, m);
							}
						}
						Ok(_) => {}
						Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
						Err(e) => {
							Err(e).with_context(|| {
								format!("failed to stat `{}`", path.display())
							})?;
						}
					}
				}
			}
		}
	}

	pub async fn remove_unused_hashes(&self) -> anyhow::Result<HashSet<Hash>> {
		fn traverse(
			dir: PathBuf,
			algorithm: HashAlgorithm,
			accumulated: Arc<str>,
		) -> BoxFuture<'static, anyhow::Result<HashSet<Hash>>> {
			async move {
				let stream = match read_dir_stream(&dir).await {
					Ok(s) => s,
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
						return Ok(Default::default());
					}
					Err(e) => return Err(e).context("failed to read hash directory"),
				};

				let mut tasks = stream
					.map(|entry| {
						let accumulated = accumulated.clone();
						async move {
							let entry = entry.context("failed to read dir entry")?;
							let name = entry.file_name();
							let component = name
								.to_str()
								.context("non-UTF-8 hash component")?
								.to_owned();
							let accumulated: Arc<str> = format!("{accumulated}{component}").into();
							let path = entry.path();

							let file_type =
								entry.file_type().await.context("failed to get file type")?;

							if file_type.is_dir() {
								return traverse(path, algorithm, accumulated).await;
							}

							let nlinks = get_nlinks(&path)
								.await
								.context("failed to count file usage")?;
							if nlinks > 1 {
								return Ok(HashSet::new());
							}

							let hash = Hash::from_encoded(algorithm, &accumulated)
								.context("failed to decode hash from path")?;

							tokio::fs::remove_file(&path).await.with_context(|| {
								format!("failed to remove unused file at `{}`", path.display())
							})?;

							if let Some(parent) = path.parent() {
								remove_empty_dir(parent).await?;
							}

							Ok(HashSet::from([hash]))
						}
					})
					.collect::<JoinSet<anyhow::Result<_>>>()
					.await;

				let mut removed_hashes = HashSet::new();
				while let Some(result) = tasks.join_next().await {
					removed_hashes.extend(result.unwrap()?);
				}

				Ok(removed_hashes)
			}
			.boxed()
		}

		let mut res = HashSet::new();

		let tasks = match read_dir_stream(&self.cas_parts_dir()).await {
			Ok(tasks) => tasks,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(res),
			Err(e) => return Err(e).context("failed to read cas directory"),
		};

		let mut tasks = tasks
			.map(|algorithm_entry| async move {
				let algorithm_entry = algorithm_entry.context("failed to read cas dir entry")?;
				let algorithm = algorithm_entry.file_name();
				let algorithm = algorithm.to_str().context("non-UTF-8 cas algorithm name")?;

				let Ok(algorithm) = HashAlgorithm::from_str(algorithm) else {
					tracing::warn!("skipping unrecognized hash algorithm directory `{algorithm}`");
					return Ok(None);
				};

				let removed_hashes =
					traverse(algorithm_entry.path(), algorithm, Default::default()).await?;

				Ok(Some(removed_hashes))
			})
			.collect::<JoinSet<anyhow::Result<_>>>()
			.await;

		while let Some(removed_hashes) = tasks.join_next().await {
			let Some(removed_hashes) = removed_hashes.unwrap()? else {
				continue;
			};

			res.extend(removed_hashes);
		}

		Ok(res)
	}
}

async fn read_dir_stream(
	dir: &Path,
) -> std::io::Result<impl Stream<Item = std::io::Result<tokio::fs::DirEntry>> + use<>> {
	let mut read_dir = tokio::fs::read_dir(dir).await?;

	Ok(try_stream!({
		while let Some(entry) = read_dir.next_entry().await? {
			yield entry;
		}
	}))
}

async fn remove_empty_dir(path: &Path) -> anyhow::Result<()> {
	match tokio::fs::remove_dir(path).await {
		Ok(_) => {}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
		Err(e) => {
			return Err(e)
				.with_context(|| format!("failed to remove directory `{}`", path.display()));
		}
	}

	Ok(())
}

async fn get_nlinks(path: &Path) -> anyhow::Result<u64> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::MetadataExt as _;
		let metadata = tokio::fs::metadata(path).await?;
		Ok(metadata.nlink())
	}
	// life if rust stabilized the nightly feature from 2019
	#[cfg(windows)]
	{
		use std::os::windows::ffi::OsStrExt as _;
		use windows::Win32::Foundation::CloseHandle;
		use windows::Win32::Storage::FileSystem::CreateFileW;
		use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
		use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
		use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
		use windows::Win32::Storage::FileSystem::GetFileInformationByHandle;
		use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
		use windows::core::PWSTR;

		let path = path.to_path_buf();
		tokio::task::spawn_blocking(move || unsafe {
			let handle = CreateFileW(
				PWSTR(
					path.as_os_str()
						.encode_wide()
						.chain(std::iter::once(0))
						.collect::<Vec<_>>()
						.as_mut_ptr(),
				),
				FILE_GENERIC_READ.0,
				FILE_SHARE_READ,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None,
			)?;

			let mut info =
				windows::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION::default();
			let res = GetFileInformationByHandle(handle, &mut info);
			CloseHandle(handle)?;
			res?;

			Ok(info.nNumberOfLinks as u64)
		})
		.await
		.unwrap()
	}
	#[cfg(not(any(unix, windows)))]
	{
		compile_error!("unsupported platform");
	}
}
