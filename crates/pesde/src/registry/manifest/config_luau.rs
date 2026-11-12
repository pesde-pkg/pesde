use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;

use super::Alias;
use crate::bounded::BoundedBTreeMap;
use crate::bounded::BoundedStr;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LuauConfig {
	luau: LuauField,
	// using an Arc on rest allows the client to trivially clone the config & change the aliases
	#[serde(flatten)]
	rest: Arc<LuauTable>,
}

impl LuauConfig {
	/// Get the aliases from the Luau config.
	#[must_use]
	pub fn aliases(&self) -> &BTreeMap<Alias, Box<str>> {
		&self.luau.aliases
	}

	/// Gets the aliases from the Luau config mutably.
	pub fn aliases_mut(&mut self) -> &mut BTreeMap<Alias, Box<str>> {
		&mut self.luau.aliases
	}
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct LuauField {
	#[serde(deserialize_with = "deserialize_aliases")]
	aliases: BTreeMap<Alias, Box<str>>,
	#[serde(flatten)]
	rest: Arc<LuauTable>,
}

fn deserialize_aliases<'de, D>(deserializer: D) -> Result<BTreeMap<Alias, Box<str>>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	struct Visitor;
	impl<'de> serde::de::Visitor<'de> for Visitor {
		type Value = BTreeMap<Alias, Box<str>>;

		fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
			write!(formatter, "a map of aliases")
		}

		fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
		where
			A: serde::de::MapAccess<'de>,
		{
			let mut aliases = BTreeMap::new();
			let mut count = 0usize;
			while let Some((key, value)) = map.next_entry::<Alias, Box<str>>()? {
				if !(1..2048).contains(&value.len()) {
					return Err(serde::de::Error::custom(
						"alias value length is out of bounds",
					));
				}

				if count >= 4096 {
					return Err(serde::de::Error::custom("too many aliases"));
				}
				count += 1;

				aliases.insert(key, value);
			}
			Ok(aliases)
		}
	}

	deserializer.deserialize_map(Visitor)
}

/// A string for a Luau config. The max length is not fixed
/// and can be changed in future releases, it exists to force sane-ish limits.
pub type LuauString = BoundedStr<65535>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum LuauValue {
	String(LuauString),
	Number(f64),
	Boolean(bool),
	Table(LuauTable),
}
type LuauTable = BoundedBTreeMap<LuauString, LuauValue, 4096>;
