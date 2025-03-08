use crate::AuthConfig;
use fs_err::tokio as fs;
use gix::bstr::BStr;
use semver::Version;
use serde::{
	de::{MapAccess, Visitor},
	Deserialize, Deserializer, Serializer,
};
use sha2::{Digest as _, Sha256};
use std::{
	collections::{BTreeMap, HashSet},
	fmt::{Display, Formatter},
	path::Path,
};

pub fn authenticate_conn(
	conn: &mut gix::remote::Connection<
		'_,
		'_,
		Box<dyn gix::protocol::transport::client::Transport + Send>,
	>,
	auth_config: &AuthConfig,
) {
	if let Some(iden) = auth_config.git_credentials().cloned() {
		conn.set_credentials(move |action| match action {
			gix::credentials::helper::Action::Get(ctx) => {
				Ok(Some(gix::credentials::protocol::Outcome {
					identity: iden.clone(),
					next: gix::credentials::helper::NextAction::from(ctx),
				}))
			}
			gix::credentials::helper::Action::Store(_)
			| gix::credentials::helper::Action::Erase(_) => Ok(None),
		});
	}
}

pub fn serialize_gix_url<S: Serializer>(url: &gix::Url, serializer: S) -> Result<S::Ok, S::Error> {
	serializer.serialize_str(&url.to_bstring().to_string())
}

pub fn deserialize_gix_url<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<gix::Url, D::Error> {
	let s = String::deserialize(deserializer)?;
	gix::Url::from_bytes(BStr::new(&s)).map_err(serde::de::Error::custom)
}

pub fn deserialize_gix_url_map<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<BTreeMap<String, gix::Url>, D::Error> {
	BTreeMap::<String, String>::deserialize(deserializer)?
		.into_iter()
		.map(|(k, v)| {
			gix::Url::from_bytes(BStr::new(&v))
				.map(|v| (k, v))
				.map_err(serde::de::Error::custom)
		})
		.collect()
}

#[allow(dead_code)]
pub fn deserialize_gix_url_vec<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<Vec<gix::Url>, D::Error> {
	Vec::<String>::deserialize(deserializer)?
		.into_iter()
		.map(|v| gix::Url::from_bytes(BStr::new(&v)).map_err(serde::de::Error::custom))
		.collect()
}

pub fn deserialize_gix_url_hashset<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<HashSet<gix::Url>, D::Error> {
	HashSet::<String>::deserialize(deserializer)?
		.into_iter()
		.map(|v| gix::Url::from_bytes(BStr::new(&v)).map_err(serde::de::Error::custom))
		.collect()
}

pub fn deserialize_git_like_url<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<gix::Url, D::Error> {
	let s = String::deserialize(deserializer)?;
	if s.contains(':') {
		gix::Url::from_bytes(BStr::new(&s)).map_err(serde::de::Error::custom)
	} else {
		gix::Url::from_bytes(BStr::new(format!("https://github.com/{s}").as_bytes()))
			.map_err(serde::de::Error::custom)
	}
}

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
