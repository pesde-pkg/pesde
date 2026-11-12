use std::fmt::Debug;
use std::sync::Arc;

use crate::link::StructureKind;
use crate::source::PackageId;
use async_trait::async_trait;
use futures::AsyncRead;
use futures::AsyncWrite;
use futures::stream::BoxStream;

pub mod glob;
mod path;
pub use path::*;

/// The virtual file system abstraction allows pesde to be very flexible in terms of runtime environments,
/// as its modular trait design is intended to allow running in various environments, even completely in-memory.
/// It intentionally isn't a pure wrapper over file system operations, but a contextual trait that lets
/// the implementation choose to implement various mechanisms by whatever is best suited.
/// For portability and security, it is also not rooted: pesde assumes that root ("" path)
/// is the root of the project, and that all paths are relative to it. Paths are also more opinioned as
/// they are required to be valid UTF-8, reject characters and file names that aren't cross-platform compatible, and
/// are not allowed to contain directory traversal components (e.g. ".."). This makes working with them
/// far more obvious and less error-prone.
#[async_trait]
pub trait Vfs: Send + Sync + Debug {
	async fn read_file(
		self: Arc<Self>,
		path: &VfsPath,
	) -> std::io::Result<Box<dyn AsyncRead + Unpin + Send>>;

	async fn write_file(self: Arc<Self>, path: &VfsPath, contents: &[u8]) -> std::io::Result<()>;

	async fn exists(self: Arc<Self>, path: &VfsPath) -> std::io::Result<bool>;

	fn read_dir(self: Arc<Self>, path: VfsPath) -> BoxStream<'static, std::io::Result<DirEntry>>;

	async fn remove_dir_all(self: Arc<Self>, path: &VfsPath) -> std::io::Result<()>;

	/// Inserts the package directory into `destination`, removing anything else that may exist there.
	/// The Vfs should treat `immutable` as a hint that the installation should be immutable, allowing it
	/// to choose a more efficient mechanism such as reflinks (preferable, harder to corrupt by accidental editing),
	/// hardlinking (less preferable, but still much more efficient than copying), or copying (least preferable).
	/// If immutable = false, the Vfs should expect the files to be written to by the user, and so should choose
	/// an appropriate mechanism that allows for that (e.g. reflinks or copying, but not hardlinking).
	async fn copy_package_to(
		self: Arc<Self>,
		immutable: bool,
		package: &PackageId,
		structure: StructureKind,
		destination: &VfsPath,
	) -> std::io::Result<()>;

	/// If the package exists, this should follow [Self::copy_package_to]'s behaviour.
	/// If it doesn't, it should return a storer handle that can be used to write the package contents.
	/// Even if the package didn't exist, it should still be linked to `destination` as if it did exist;
	/// whether it happens actively while storing or lazily afterwards is up to the implementation.
	///
	/// Storage should, ideally, be atomic and utilise a locking mechanism to prevent multiple processes from
	/// downloading & storing the same package at the same time. This isn't strictly required and is mostly
	/// a user experience concern. In particular, contexts where multiple processes are impossible to
	/// run at the same time (e.g. a strictly controlled environment) this can pretty safely be ignored.
	async fn copy_and_maybe_store_package(
		self: Arc<Self>,
		immutable: bool,
		package: &PackageId,
		structure: StructureKind,
		destination: &VfsPath,
	) -> std::io::Result<Option<Box<dyn PackageStorer>>>;
}

#[async_trait]
#[must_use]
pub trait PackageStorer: Send + Sync {
	async fn directory_at(&mut self, path: VfsPath) -> std::io::Result<()>;
	async fn file_at<'a>(
		&'a mut self,
		path: VfsPath,
		size_hint: u64,
	) -> std::io::Result<&'a mut (dyn AsyncWrite + Unpin + Send + 'a)>;

	async fn commit(self: Box<Self>) -> std::io::Result<()>;
}

pub struct DirEntry {
	pub name: VfsPath,
	pub is_dir: bool,
}

// #[cfg(test)]
// pub(crate) use tests::TestVfs;

// #[cfg(test)]
// mod tests {
// 	use async_stream::try_stream;
// 	use futures::StreamExt as _;
// 	use futures::TryStreamExt as _;
// 	use opendal_core::Operator;

// 	use super::*;

// 	#[derive(Debug)]
// 	pub struct TestVfs(opendal_core::Operator);

// 	impl TestVfs {
// 		pub fn new() -> Self {
// 			Self(Operator::new(opendal_core::services::Memory::default()).unwrap())
// 		}
// 	}

// 	fn map_err(e: opendal_core::Error) -> std::io::Error {
// 		use opendal_core::ErrorKind;
// 		match e.kind() {
// 			ErrorKind::Unexpected => std::io::Error::other(e),
// 			ErrorKind::Unsupported => std::io::Error::new(std::io::ErrorKind::Unsupported, e),
// 			ErrorKind::ConfigInvalid => std::io::Error::other(e),
// 			ErrorKind::NotFound => std::io::Error::new(std::io::ErrorKind::NotFound, e),
// 			ErrorKind::PermissionDenied => {
// 				std::io::Error::new(std::io::ErrorKind::PermissionDenied, e)
// 			}
// 			ErrorKind::IsADirectory => std::io::Error::new(std::io::ErrorKind::IsADirectory, e),
// 			ErrorKind::NotADirectory => std::io::Error::new(std::io::ErrorKind::NotADirectory, e),
// 			ErrorKind::AlreadyExists => std::io::Error::new(std::io::ErrorKind::AlreadyExists, e),
// 			ErrorKind::RateLimited => std::io::Error::other(e),
// 			ErrorKind::IsSameFile => std::io::Error::new(std::io::ErrorKind::InvalidFilename, e),
// 			ErrorKind::ConditionNotMatch => {
// 				std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
// 			}
// 			ErrorKind::RangeNotSatisfied => {
// 				std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
// 			}
// 			ErrorKind::Conflict => std::io::Error::new(std::io::ErrorKind::AlreadyExists, e),
// 			_ => e.into(),
// 		}
// 	}

// 	// #[async_trait]
// 	// impl Vfs for TestVfs {
// 	// 	async fn read_file(
// 	// 		&self,
// 	// 		path: &VfsPath,
// 	// 	) -> std::io::Result<Box<dyn AsyncRead + Unpin + Send>> {
// 	// 		self.0
// 	// 			.reader(&format!("project/{path}"))
// 	// 			.await
// 	// 			.map_err(map_err)?
// 	// 			.into_futures_async_read(..)
// 	// 			.await
// 	// 			.map(|r| Box::new(r) as Box<dyn AsyncRead + Unpin + Send>)
// 	// 			.map_err(map_err)
// 	// 	}

// 	// 	async fn write_file(&self, path: &VfsPath, contents: &[u8]) -> std::io::Result<()> {
// 	// 		self.0
// 	// 			.write(&format!("project/{path}"), contents.to_vec())
// 	// 			.await
// 	// 			.map(|_| ())
// 	// 			.map_err(map_err)
// 	// 	}

// 	// 	fn read_dir(&self, path: VfsPath) -> BoxStream<'static, std::io::Result<DirEntry>> {
// 	// 		let operator = self.0.clone();
// 	// 		try_stream!({
// 	// 			let mut read_dir = operator
// 	// 				.lister(&format!("project/{path}/"))
// 	// 				.await
// 	// 				.map_err(map_err)?;
// 	// 			while let Some(entry) = read_dir.try_next().await.map_err(map_err)? {
// 	// 				yield DirEntry {
// 	// 					name: entry.name().parse()?,
// 	// 					is_dir: entry.metadata().is_dir(),
// 	// 				};
// 	// 			}
// 	// 		})
// 	// 		.boxed()
// 	// 	}

// 	// 	async fn exists(&self, path: &VfsPath) -> std::io::Result<bool> {
// 	// 		self.0
// 	// 			.exists(&format!("project/{path}"))
// 	// 			.await
// 	// 			.map_err(map_err)
// 	// 	}

// 	// 	async fn remove_dir_all(&self, path: &VfsPath) -> std::io::Result<()> {
// 	// 		self.0
// 	// 			.delete(&format!("project/{path}/"))
// 	// 			.await
// 	// 			.map_err(map_err)
// 	// 	}
// 	// }
// }
