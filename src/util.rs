use crate::AuthConfig;
use gix::bstr::BStr;
use serde::{Deserialize, Deserializer, Serializer};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

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
            gix::credentials::helper::Action::Store(_) => Ok(None),
            gix::credentials::helper::Action::Erase(_) => Ok(None),
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
