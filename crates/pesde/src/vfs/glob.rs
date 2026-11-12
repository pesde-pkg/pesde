use std::collections::HashSet;
use std::fmt::Debug;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_stream::try_stream;
use futures::Stream;
use futures::StreamExt as _;
use futures::stream::BoxStream;
use ouroboros::self_referencing;
use serde::Deserialize;
use serde::Serialize;
use wax::Glob;
use wax::Program as _;

use crate::Error;
use crate::Result;

use super::DirEntry;
use super::Vfs;
use super::VfsPath;

struct Walk<'t> {
	prefix: VfsPath,
	glob: Option<Glob<'t>>,
	max_depth: Option<usize>,
}

struct Compiled<'t> {
	negative: wax::Any<'t>,
	prune: wax::Any<'t>,
	walks: Vec<Walk<'t>>,
}

pub struct VfsGlobset(VfsGlobsetInner);

#[self_referencing]
struct VfsGlobsetInner {
	globs: Box<[Box<str>]>,
	#[borrows(globs)]
	#[covariant]
	compiled: Compiled<'this>,
}

impl Debug for VfsGlobset {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("VfsGlobset")
			.field("globs", &self.0.borrow_globs())
			.finish()
	}
}

impl Default for VfsGlobset {
	fn default() -> Self {
		Self(VfsGlobsetInner::new([].into(), |_| Compiled {
			negative: wax::any::<[&str; 0]>([]).unwrap(),
			prune: wax::any::<[&str; 0]>([]).unwrap(),
			walks: vec![],
		}))
	}
}

impl VfsGlobset {
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.0.borrow_globs().is_empty()
	}

	pub fn matches<'a>(
		&'a self,
		vfs: Arc<dyn Vfs>,
		root: &'a VfsPath,
	) -> impl Stream<Item = Result<VfsPath>> + Send + 'a {
		struct Frame<'a> {
			entries: BoxStream<'a, std::io::Result<DirEntry>>,
			dir: VfsPath,
			rel: PathBuf,
		}

		try_stream!({
			let Compiled {
				negative,
				prune,
				walks,
			} = self.0.borrow_compiled();

			let mut seen = HashSet::new();

			for Walk {
				prefix,
				glob,
				max_depth,
			} in walks
			{
				let Some(glob) = glob else {
					let (parent, name) = match prefix.pop() {
						Some((parent, name)) => (
							root.join(
								&parent
									.parse::<VfsPath>()
									.map_err(|e| Error::vfs(e, prefix.clone()))?,
							),
							name,
						),
						None => (root.clone(), prefix.clone()),
					};

					let mut entries = std::pin::pin!(vfs.clone().read_dir(parent));
					let mut found = false;
					while let Some(entry) = entries.next().await {
						match entry {
							Ok(e) if e.name == name => {
								found = true;
								break;
							}
							Ok(_) => {}
							Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
							Err(e) => Err(Error::vfs(e, prefix.clone()))?,
						}
					}

					if found {
						let full = root.join(prefix);
						if !negative.is_match(Path::new(&**prefix)) && seen.insert(full.clone()) {
							yield full;
						}
					}
					continue;
				};

				if *max_depth == Some(0) {
					continue;
				}

				let start = root.join(prefix);

				let mut stack = vec![Frame {
					entries: Box::pin(vfs.clone().read_dir(start.clone())),
					dir: start,
					rel: PathBuf::new(),
				}];

				while let Some(frame) = stack.last_mut() {
					let entry = match frame.entries.next().await {
						Some(Ok(entry)) => entry,
						Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
							stack.pop();
							continue;
						}
						Some(Err(e)) => Err(Error::vfs(e, frame.dir.clone()))?,
						None => {
							stack.pop();
							continue;
						}
					};

					let rel = frame.rel.join(&*entry.name);
					let child = frame.dir.join(&entry.name);

					let root_rel = if prefix.is_empty() {
						rel.clone()
					} else {
						Path::new(&**prefix).join(&rel)
					};

					if glob.is_match(rel.as_path())
						&& !negative.is_match(root_rel.as_path())
						&& seen.insert(child.clone())
					{
						yield child.clone();
					}

					let depth = rel.components().count();
					let can_go_deeper = max_depth.is_none_or(|max| depth < max);

					if entry.is_dir && can_go_deeper && !prune.is_match(root_rel.as_path()) {
						stack.push(Frame {
							entries: Box::pin(vfs.clone().read_dir(child.clone())),
							dir: child,
							rel,
						});
					}
				}
			}
		})
	}
}

impl Serialize for VfsGlobset {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.0.borrow_globs().serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for VfsGlobset {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct Visitor;
		impl<'de> serde::de::Visitor<'de> for Visitor {
			type Value = VfsGlobset;

			fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(formatter, "a list of globs")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: serde::de::SeqAccess<'de>,
			{
				let globs = Box::<[Box<str>]>::deserialize(
					serde::de::value::SeqAccessDeserializer::new(&mut seq),
				)?;

				VfsGlobsetInner::try_new(globs, |globs| compile::<A::Error>(globs)).map(VfsGlobset)
			}
		}
		deserializer.deserialize_seq(Visitor)
	}
}

fn compile<E: serde::de::Error>(globs: &[Box<str>]) -> Result<Compiled<'_>, E> {
	let mut negatives = Vec::new();
	let mut exhaustive = Vec::new();
	let mut walks = Vec::new();

	for g in globs {
		if let Some(negated) = g.strip_prefix('!') {
			let glob = Glob::new(negated).map_err(E::custom)?;
			if glob.is_exhaustive().is_always() {
				exhaustive.push(glob.clone());
			}
			negatives.push(glob);
		} else {
			let glob = Glob::new(g).map_err(E::custom)?;
			walks.extend(build_walk::<E>(glob)?);
		}
	}

	Ok(Compiled {
		negative: wax::any(negatives).map_err(E::custom)?,
		prune: wax::any(exhaustive).map_err(E::custom)?,
		walks,
	})
}

fn build_walk<E: serde::de::Error>(glob: Glob<'_>) -> Result<Option<Walk<'_>>, E> {
	let (prefix, glob) = glob.partition();

	let path = VfsPath::from_path(&prefix).map_err(E::custom)?;
	if path.is_empty() && glob.is_none() {
		return Ok(None);
	}

	let max_depth = glob.as_ref().and_then(max_depth);

	Ok(Some(Walk {
		prefix: path,
		glob,
		max_depth,
	}))
}

fn max_depth(glob: &Glob<'_>) -> Option<usize> {
	match glob.depth() {
		wax::query::Variance::Invariant(depth) => Some(depth),
		wax::query::Variance::Variant(_) => None,
	}
}
