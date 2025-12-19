use fs_err::tokio as fs;
use semver::Version;
use serde::{
	Deserialize, Deserializer,
	de::{MapAccess, Visitor},
};
use sha2::{Digest as _, Sha256};
use std::{
	collections::BTreeMap,
	fmt::{Display, Formatter},
	path::{Path, PathBuf},
};

pub fn hash<S: AsRef<[u8]>>(struc: S) -> String {
	format!("{:x}", Sha256::digest(struc.as_ref()))
}

pub fn is_default<T: Default + Eq>(t: &T) -> bool {
	t == &T::default()
}

#[must_use]
pub fn no_build_metadata(version: &Version) -> Version {
	let mut version = version.clone();
	version.build = semver::BuildMetadata::EMPTY;
	version
}

pub async fn remove_empty_dir(path: &Path) -> std::io::Result<()> {
	match fs::remove_dir(path).await {
		Ok(()) => Ok(()),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
		// concurrent removal on Windows seems to fail with PermissionDenied
		// TODO: investigate why this happens and whether we can avoid it without ignoring all PermissionDenied errors
		#[cfg(windows)]
		Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
		Err(e) => Err(e),
	}
}

/// Implement `Serialize` and `Deserialize` for a type that implements `Display` and `FromStr`
#[macro_export]
macro_rules! ser_display_deser_fromstr {
	($struct_name:ident) => {
		impl serde::Serialize for $struct_name {
			fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
			where
				S: serde::ser::Serializer,
			{
				serializer.collect_str(self)
			}
		}

		impl<'de> serde::Deserialize<'de> for $struct_name {
			fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
			where
				D: serde::de::Deserializer<'de>,
			{
				let s = String::deserialize(deserializer)?;
				s.parse().map_err(serde::de::Error::custom)
			}
		}
	};
}

pub fn deserialize_no_dup_keys<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
where
	K: Display + Ord + Deserialize<'de>,
	V: Deserialize<'de>,
	D: Deserializer<'de>,
{
	struct NoDupKeysVisitor<K, V> {
		map: BTreeMap<K, V>,
	}

	impl<'de, K, V> Visitor<'de> for NoDupKeysVisitor<K, V>
	where
		K: Display + Ord + Deserialize<'de>,
		V: Deserialize<'de>,
	{
		type Value = BTreeMap<K, V>;

		fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
			formatter.write_str("a map with no duplicate keys")
		}

		fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
		where
			A: MapAccess<'de>,
		{
			let mut res = self.map;

			while let Some((key, value)) = map.next_entry()? {
				if res.contains_key(&key) {
					return Err(serde::de::Error::custom(format!("duplicate key `{key}`")));
				}

				res.insert(key, value);
			}

			Ok(res)
		}
	}

	deserializer.deserialize_map(NoDupKeysVisitor {
		map: BTreeMap::new(),
	})
}

pub async fn symlink_file(src: PathBuf, dst: PathBuf) -> std::io::Result<()> {
	#[cfg(unix)]
	{
		fs::symlink(src, dst).await
	}
	#[cfg(windows)]
	{
		if std::env::var("PESDE_FORCE_SYMLINK").is_ok() {
			return fs::symlink_file(src, dst).await;
		}

		fs::hard_link(src, dst).await
	}
}

pub async fn symlink_dir(src: PathBuf, dst: PathBuf) -> std::io::Result<()> {
	#[cfg(unix)]
	{
		fs::symlink(src, dst).await
	}
	#[cfg(windows)]
	{
		if std::env::var("PESDE_FORCE_SYMLINK").is_ok() {
			return fs::symlink_dir(src, dst).await;
		}

		tokio::task::spawn_blocking(move || {
			junction::create(&src, &dst).map_err(|e| {
				std::io::Error::new(
					e.kind(),
					format!("failed to create junction from `{src:?}` to `{dst:?}`"),
				)
			})
		})
		.await
		.unwrap()
	}
}
