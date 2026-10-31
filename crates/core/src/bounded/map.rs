use std::{collections::BTreeMap, marker::PhantomData, ops::Deref};

use serde::{Deserialize, Serialize};

use crate::bounded::BoundedSizeError;

macro_rules! bounded_map {
	($name:ident($map:ty) $(where $($ty:ty: $bound:tt),+)?, $make:expr, $insert:ident) => {
		#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
		#[serde(transparent)]
		pub struct $name<K, V, const N: usize>($map) $(where $($ty: $bound),+)?;

		impl<K, V, const N: usize> $name<K, V, N> $(where $($ty: $bound),+)? {
			pub fn new(map: impl Into<$map>) -> Result<Self, BoundedSizeError> {
				let map = map.into();
				if map.len() > N {
					return Err(BoundedSizeError {
						limit: N,
						actual: map.len(),
					});
				}

				Ok(Self(map))
			}

			#[must_use]
			pub fn into_inner(self) -> $map {
				self.0
			}

			#[must_use]
			pub fn len(&self) -> usize {
				self.0.len()
			}

			#[must_use]
			pub fn is_empty(&self) -> bool {
				self.0.is_empty()
			}
		}

		impl<K, V, const N: usize> Deref for $name<K, V, N> $(where $($ty: $bound),+)? {
			type Target = $map;

			fn deref(&self) -> &Self::Target {
				&self.0
			}
		}

		impl<'de, K: Deserialize<'de>, V: Deserialize<'de>, const N: usize> Deserialize<'de> for $name<K, V, N> $(where $($ty: $bound),+)? {
			fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
			where
				D: serde::Deserializer<'de>,
			{
				struct Visitor<K, V> {
					phantom: PhantomData<fn() -> (K, V)>,
					limit: usize,
				}

				impl<'de, K: Deserialize<'de>, V: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<K, V> $(where $($ty: $bound),+)? {
					type Value = $map;

					fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
						write!(formatter, "a map of at most {} elements", self.limit)
					}

                    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>,
                    {
                        let mut elems = $make(map.size_hint().unwrap_or(self.limit).min(self.limit));
                        let mut count = 0;

                        while let Some((key, value)) = map.next_entry()? {
                            count += 1;
                            if count <= self.limit {
                                elems.$insert(key, value);
                            }
                        }

                        if count > self.limit {
                            return Err(serde::de::Error::invalid_length(count, &self));
                        }

                        Ok(elems.into())
                    }
				}

				deserializer
					.deserialize_map(Visitor {
						phantom: PhantomData,
						limit: N,
					})
					.map(Self)
			}
		}
	};
}
bounded_map!(BoundedBTreeMap(BTreeMap<K, V>) where K: Ord, |_| BTreeMap::new(), insert);
