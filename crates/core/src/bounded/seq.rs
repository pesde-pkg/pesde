use std::{collections::BTreeSet, marker::PhantomData, ops::Deref};

use serde::{Deserialize, Serialize};

use crate::bounded::BoundedSizeError;

macro_rules! bounded_seq {
	($name:ident($seq:ty) $(where $($ty:ty: $bound:tt),+)?, $make:expr, $add:ident) => {
		#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
		#[serde(transparent)]
		pub struct $name<T, const N: usize>($seq) $(where $($ty: $bound),+)?;

		impl<T, const N: usize> $name<T, N> $(where $($ty: $bound),+)? {
			pub fn new(seq: impl Into<$seq>) -> Result<Self, BoundedSizeError> {
				let seq = seq.into();
				if seq.len() > N {
					return Err(BoundedSizeError {
						limit: N,
						actual: seq.len(),
					});
				}

				Ok(Self(seq))
			}

			#[must_use]
			pub fn into_inner(self) -> $seq {
				self.0
			}
		}

		impl<T, const N: usize> Deref for $name<T, N> $(where $($ty: $bound),+)? {
			type Target = $seq;

			fn deref(&self) -> &Self::Target {
				&self.0
			}
		}

		impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for $name<T, N> $(where $($ty: $bound),+)? {
			fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
			where
				D: serde::Deserializer<'de>,
			{
				struct Visitor<T> {
					phantom: PhantomData<fn() -> T>,
					limit: usize,
				}

				impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> $(where $($ty: $bound),+)? {
					type Value = $seq;

					fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
						write!(formatter, "a sequence of at most {} elements", self.limit)
					}

					fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
					where
						A: serde::de::SeqAccess<'de>,
					{
						let mut elems =
							$make(seq.size_hint().unwrap_or(self.limit).min(self.limit));
						let mut count = 0;

						while let Some(elem) = seq.next_element()? {
							count += 1;
							if count <= self.limit {
								elems.$add(elem);
							}
						}

						if count > self.limit {
							return Err(serde::de::Error::invalid_length(count, &self));
						}

						Ok(elems.into())
					}
				}

				deserializer
					.deserialize_seq(Visitor {
						phantom: PhantomData,
						limit: N,
					})
					.map(Self)
			}
		}
	};
}
bounded_seq!(BoundedSlice(Box<[T]>), Vec::with_capacity, push);
bounded_seq!(BoundedBTreeSet(BTreeSet<T>) where T: Ord, |_| BTreeSet::new(), insert);
