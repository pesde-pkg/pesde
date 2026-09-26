//! Data models for the registry

use std::convert::Infallible;
use std::fmt::Display;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::hasher::Hasher;
use merkleberg::Merge;
use semver::Prerelease;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::bounded::Bounded;
use crate::hash::Blake3Hash;
use crate::hash::Hash;
use crate::hash::Hasher as _;
use crate::ser_display_deser_fromstr;
use crate::signature::PublicKey;
use crate::signature::Signature;

mod global;
mod response;
mod scope;
pub use global::*;
pub use response::*;
pub use scope::*;

/// Returns a canonical serialisation of the given struct for cryptographic purposes
#[must_use]
pub fn canonical_bytes(data: &impl Serialize) -> Vec<u8> {
	cbor_core::Value::serialized(data)
		.expect("failed to serialise body for signing")
		.encode()
}

/// A tagged object
pub trait Tagged {
	/// The tag of this value
	const TAG: &'static str;
}

/// A tag of an operation
#[derive(Debug, Clone, Copy)]
pub struct OpTag<T: Tagged>(PhantomData<T>);

impl<T: Tagged> Default for OpTag<T> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<T: Tagged> Serialize for OpTag<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		T::TAG.serialize(serializer)
	}
}

impl<'de, T: Tagged> Deserialize<'de> for OpTag<T> {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct Visitor(&'static str);
		impl serde::de::Visitor<'_> for Visitor {
			type Value = ();

			fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(formatter, "string {:?}", self.0)
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where
				E: serde::de::Error,
			{
				if v == self.0 {
					Ok(())
				} else {
					Err(E::invalid_value(serde::de::Unexpected::Str(v), &self))
				}
			}
		}

		deserializer
			.deserialize_str(Visitor(T::TAG))
			.map(|_| Default::default())
	}
}

/// Things that can be an [Entry]'s payload
pub trait EntryPayload {}

/// An entry in a log, at a known leaf position
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry<T: EntryPayload> {
	/// The leaf position of this entry
	pub pos: u64,
	/// The time of publishing of this entry
	/// This value is server authoritative because of time sync issues a client provided value would pose
	pub published_at: Timestamp,
	/// The payload of this entry
	pub payload: T,
}

/// An object that can be signed & carries its own signing key
pub trait Signable {
	/// The key that should sign this
	fn signer(&self) -> &PublicKey;
}

/// An unvalidated record carrying a signature and a signer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnvalidatedSigned<T> {
	/// The signature
	pub sig: Signature,
	/// The body
	#[serde(flatten)]
	pub body: T,
}

/// The signature didn't match what was expected
#[derive(Debug, Error)]
#[error("invalid signature")]
pub struct BadSignature;

/// A validated wrapper over [UnvalidatedSigned], allowing construction only if it's legal
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct Signed<T: Signable>(UnvalidatedSigned<T>);

impl<T: Signable + Serialize> Signed<T> {
	/// Validates the passed in [UnvalidatedSigned] and returns Some if it's valid
	pub fn new(input: UnvalidatedSigned<T>) -> Result<Self, BadSignature> {
		if !input
			.sig
			.verify(input.body.signer(), &canonical_bytes(&input.body))
		{
			return Err(BadSignature);
		}

		Ok(Self(input))
	}

	/// Returns a reference to the underlying [UnvalidatedSigned]
	pub fn inner(&self) -> &UnvalidatedSigned<T> {
		&self.0
	}

	/// Returns the underlying [UnvalidatedSigned]
	pub fn into_inner(self) -> UnvalidatedSigned<T> {
		self.0
	}
}

impl<'de, T: Signable + Serialize + Deserialize<'de>> Deserialize<'de> for Signed<T> {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Self::new(UnvalidatedSigned::deserialize(deserializer)?).map_err(serde::de::Error::custom)
	}
}

/// The scope id; hash of the scope name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeId(CurrentHash);

impl ScopeId {
	/// Creates a new [Self] from a [Scope](crate::names::Scope)
	#[must_use]
	pub fn from(name: &crate::names::Scope) -> Self {
		let mut hasher = CurrentHash::hasher();
		hasher.update(name.as_str().as_bytes());
		Self(hasher.finalize())
	}
}

impl Display for ScopeId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

/// The local name id; hash of the package local name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct LocalNameId(CurrentHash);

impl LocalNameId {
	/// Creates a new [Self] from a [LocalName](crate::names::LocalName)
	#[must_use]
	pub fn from(name: &crate::names::LocalName) -> Self {
		let mut hasher = CurrentHash::hasher();
		hasher.update(name.as_str().as_bytes());
		Self(hasher.finalize())
	}
}

impl Display for LocalNameId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

/// The current hash used by the registry
pub type CurrentHash = Blake3Hash;

/// The [Merge] and [Hasher] implementation using the [CurrentHash]
#[derive(Debug)]
pub struct CurrentMerkleHasher;

impl Merge for CurrentMerkleHasher {
	type Item = CurrentHash;
	type Error = Infallible;

	fn leaf_hash(data: &[u8]) -> Result<Self::Item, Self::Error> {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x00]);
		hasher.update(data);
		Ok(hasher.finalize())
	}

	fn merge_pos(
		pos: u64,
		left: &Self::Item,
		right: &Self::Item,
	) -> Result<Self::Item, Self::Error> {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x01]);
		hasher.update(&pos.to_be_bytes());
		hasher.update(left.0.as_ref());
		hasher.update(right.0.as_ref());
		Ok(hasher.finalize())
	}
}

impl<K: Serialize, V: Serialize> Hasher<K, V> for CurrentMerkleHasher {
	type Output = CurrentHash;

	fn empty_hash() -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x10]);
		hasher.finalize()
	}

	fn hash_key(key: &K) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x11]);
		cbor_core::Value::serialized(key)
			.unwrap()
			.write_to(&mut hasher)
			.unwrap();
		hasher.finalize()
	}

	fn hash_value(value: &V) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x12]);
		cbor_core::Value::serialized(value)
			.unwrap()
			.write_to(&mut hasher)
			.unwrap();
		hasher.finalize()
	}

	fn hash_slot(key: &K, child: &Self::Output) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x13]);
		cbor_core::Value::serialized(key)
			.unwrap()
			.write_to(&mut hasher)
			.unwrap();
		hasher.update(child.0.as_ref());
		hasher.finalize()
	}

	fn merge_hashes(a: &Self::Output, b: &Self::Output) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(a.0.as_ref());
		hasher.update(b.0.as_ref());
		hasher.finalize()
	}

	fn hash_leaf(entry_count: usize, merkle_root: &Self::Output) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x14]);
		hasher.update(&(entry_count as u64).to_be_bytes());
		hasher.update(merkle_root.0.as_ref());
		hasher.finalize()
	}

	fn hash_internal(child_count: usize, slots_root: &Self::Output) -> Self::Output {
		let mut hasher = CurrentHash::hasher();
		hasher.update(&[0x15]);
		hasher.update(&(child_count as u64).to_be_bytes());
		hasher.update(slots_root.0.as_ref());
		hasher.finalize()
	}
}
