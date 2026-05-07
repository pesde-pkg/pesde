//! pesde package source backend abstraction
#![allow(async_fn_in_trait)]

use crate::Project;
use crate::hash::Hash;
use crate::names::Name;
use crate::names::PackageName;
use crate::names::Scope;
use crate::reporters::DownloadProgressReporter;
use crate::signature::PublicKey;
use crate::signature::Signature;
use futures::Stream;
use relative_path::RelativePathBuf;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;
use std::sync::Arc;

/// A source of  pesde packages
pub trait PesdePackageSourceBackend: Debug + Display + Send + Sync {
	/// The error type for refreshing this backend
	type RefreshError: std::error::Error + Send + Sync + 'static;
	/// The error type for downloading entries
	type DownloadError: std::error::Error + Send + Sync + 'static;

	/// Refreshes the backend
	fn refresh(
		&self,
		project: &Project,
	) -> impl Future<Output = Result<(), Self::RefreshError>> + Send;

	/// Downloads entries for a package version
	fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &PackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> impl Future<
		Output = Result<
			impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
			Self::DownloadError,
		>,
	> + Send;
}

/// All available pesde package backends
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PesdePackageBackends {}

impl Display for PesdePackageBackends {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		todo!()
	}
}

impl FromStr for PesdePackageBackends {
	type Err = errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Err(errors::ParseBackendErrorKind::NoMatch(s.to_string(), "".into()).into())
	}
}

crate::ser_display_deser_fromstr!(PesdePackageBackends);

impl PesdePackageSourceBackend for PesdePackageBackends {
	type RefreshError = errors::RefreshError;
	type DownloadError = errors::DownloadError;

	async fn refresh(&self, _project: &Project) -> Result<(), Self::RefreshError> {
		todo!()
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &PackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		todo!();
		Ok(futures::stream::empty())
	}
}

/// A trait for types that can be serialised in a canonical form
pub trait Canonical: Serialize {
	/// Returns a canonical serialisation of the given body for signing or signature verification
	#[must_use]
	fn signing_bytes(&self) -> Vec<u8> {
		cbor_core::Value::serialized(self)
			.expect("failed to serialise body for signing")
			.encode()
	}
}

/// An entry with an associated signature
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedEntry<T: Canonical> {
	/// The body being signed
	pub body: T,
	/// The signature over the canonical serialisation of body
	pub sig: Signature,
}

impl<T: Canonical> SignedEntry<T> {
	/// Verifies the signature of this entry against the given public key
	#[must_use]
	pub fn verify(&self, public_key: &PublicKey) -> bool {
		self.sig.verify(public_key, &self.body.signing_bytes())
	}
}

/// The sequence number of a registry entry
/// This number is a globally increasing number in the log
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntrySeq(pub u64);

/// The sequence number of a scope entry
/// This number is a per-scope increasing number, unlike [EntrySeq] which is globally increasing
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeSeq(pub u64);

/// The stable identifier for an identity
/// It is the hash of the initial public key and never changes, even after key rotation
/// A given public key can only ever be registered once - once rotated away from, it cannot
/// be re-registered as a new identity
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IdentityId(pub Hash);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
/// Permissions for a scope member
pub enum ScopePermission {
	/// Permission to publish new packages and versions
	Publish,
	/// Permission to manage the retirement status of packages (deprecations) and versions (yanks)
	Retire,
}

/// An entry for a member in the scope manifest
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberEntry {
	/// The stable identity ID of the member
	pub identity_id: IdentityId,
	/// The permissions granted to this member
	pub permissions: BTreeSet<ScopePermission>,
}

/// The manifest for a scope, describing its owner, members, and their permissions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeManifest {
	/// The sole identity permitted to manage this scope's manifest
	pub owner: IdentityId,
	/// Members with restricted permissions
	pub members: Vec<MemberEntry>,
}

/// The body of a Publish entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishBody {
	/// The scope of the package being published
	pub scope: Scope,
	/// The name of the package being published
	pub name: Name,
	/// The version of the package being published
	pub version: Version,
	/// The hash of the archive containing the package contents
	pub archive_hash: Hash,
	/* TODO: other fields, e.g. dependencies */
}

/// The body of a Yank entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YankBody {
	/// The scope of the package being yanked
	pub scope: Scope,
	/// The name of the package being yanked
	pub name: Name,
	/// The version of the package being yanked
	pub version: Version,
}

/// The body of a Deprecate entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeprecateBody {
	/// The scope of the package being deprecated
	pub scope: Scope,
	/// The name of the package being deprecated
	pub name: Name,
	/// The reason for deprecation
	pub reason: String,
}

/// The body of a ManifestUpdate entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeManifestUpdateBody {
	/// The scope whose manifest is being updated
	pub scope: Scope,
	/// The complete new manifest, replacing the previous one entirely
	pub manifest: ScopeManifest,
}

/// The payload of a scope entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopePayload {
	/// Publish a new package version
	Publish(PublishBody),
	/// Yank an existing package version
	Yank(YankBody),
	/// Deprecate an existing package
	Deprecate(DeprecateBody),
	/// Replace the scope manifest entirely
	ManifestUpdate(ScopeManifestUpdateBody),
}

/// The body of a scope entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredScopeEntryBody {
	/// Hash of the previous [AuthoredScopeEntry] in this scope's sub-chain
	/// `None` if this is the first entry in the scope, in which case the registry
	/// implicitly creates the scope with the author as sole owner with full permissions
	#[serde(skip_serializing_if = "Option::is_none", default)]
	pub prev_scope: Option<Hash>,
	/// The sequence number of this entry within the scope
	pub scope_seq: ScopeSeq,
	/// The [EntrySeq] of the most recent [IdentityRotation] for the author, or `None` if the author has never rotated their key
	#[serde(skip_serializing_if = "Option::is_none", default)]
	pub prev_identity_seq: Option<EntrySeq>,
	/// The identity of the author
	pub author_identity: IdentityId,
	/// The payload of this entry
	pub payload: ScopePayload,
}
impl Canonical for AuthoredScopeEntryBody {}

/// The body of a RegisterIdentity entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterIdentityBody {
	/// The identity ID, which must equal Hash(public_key)
	/// Stored separately to ensure new hash algorithms can be adopted in the future without creating conflicts
	pub identity_id: IdentityId,
	/// The initial public key for this identity
	pub public_key: PublicKey,
}
impl Canonical for RegisterIdentityBody {}

/// The body of an IdentityRotation entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRotationBody {
	/// The identity being rotated
	pub identity_id: IdentityId,
	/// The [EntrySeq] of the previous [IdentityRotation] for this identity, or `None` if this is the first rotation after registration
	#[serde(skip_serializing_if = "Option::is_none", default)]
	pub prev_identity: Option<EntrySeq>,
	/// The new public key to associate with this identity after rotation
	pub new_pubkey: PublicKey,
}
impl Canonical for IdentityRotationBody {}

/// A forced scope ownership transfer done by the registry administrator, without the consent of the previous owner
/// Intended solely for administrative interventions including e.g. squatting or legal disputes
/// This entry should be brought up to the user interactively if encountered during installation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminScopeTransfer {
	/// The scope being transferred
	pub scope: Scope,
	/// The new manifest to install, including the new owner
	pub manifest: ScopeManifest,
}

/// All possible entry kinds in the registry log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
	/// A scope-chained entry (publish, yank, or manifest update)
	Scoped(SignedEntry<AuthoredScopeEntryBody>),
	/// Registration of a new identity, anchoring its initial public key
	RegisterIdentity(SignedEntry<RegisterIdentityBody>),
	/// Rotation of the public key for an existing identity
	IdentityRotation(SignedEntry<IdentityRotationBody>),
	/// A forced scope ownership transfer initiated by the registry operator
	AdminScopeTransfer(AdminScopeTransfer),
}

/// An entry in the registry log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
	/// The globally unique sequence number of this entry
	pub seq: EntrySeq,
	/// The content of this entry
	pub kind: EntryKind,
}

/// Errors that can occur when interacting with pesde package source backends
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a pesde package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ParseBackendError))]
	pub enum ParseBackendErrorKind {
		/// No backend type matched the input
		#[error("no backend type matched for {0}")]
		NoMatch(
			String,
			#[source] /* TODO */ Box<dyn std::error::Error + Send + Sync>,
		),
	}

	/// Errors that can occur when refreshing a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RefreshError))]
	#[non_exhaustive]
	pub enum RefreshErrorKind {
		// /// An error occurred from the Git backend
		// #[error("error from git backend")]
		// Git(#[from] crate::source::git_index::errors::RefreshError),
	}

	/// Errors that can occur when reading the config file for a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ConfigError))]
	#[non_exhaustive]
	pub enum ConfigErrorKind {
		// /// An error occurred from the Git backend
		// #[error("error from git backend")]
		// Git(#[from] GitConfigError),
	}

	/// Errors that can occur when reading an index file for a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ReadIndexFileError))]
	#[non_exhaustive]
	pub enum ReadIndexFileErrorKind {
		// /// An error occurred from the Git backend
		// #[error("error from git backend")]
		// Git(#[from] GitReadIndexFileError),
	}

	/// Errors that can occur when downloading a package from a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		// /// An error occurred from the Git backend
		// #[error("error from git backend")]
		// Git(#[from] GitDownloadError),
	}
}
