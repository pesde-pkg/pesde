use std::marker::PhantomData;

use super::super::*;

/// Consent data without any validity guarantees
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnvalidatedConsent {
	/// A unique value to make this one-use
	pub nonce: [u8; 16],
	/// The person consenting
	pub consenter: PublicKey,
}

/// Consent that is known to be signed by its consenter for the scope
#[derive(Debug, Clone)]
pub struct Consent<T: Tagged>(UnvalidatedSigned<UnvalidatedConsent>, PhantomData<T>);

impl<T: Tagged> Consent<T> {
	#[must_use]
	pub fn signing_bytes(scope_id: &ScopeId, unvalidated: &UnvalidatedConsent) -> Vec<u8> {
		let UnvalidatedConsent { nonce, consenter } = unvalidated;
		cbor_core::map! {
			"tag" => T::TAG,
			"scope_id" => scope_id.hash().0,
			"nonce" => nonce,
			"consenter" => consenter.to_string(),
		}
		.encode()
	}

	/// Returns Ok if the [ScopeId] and [UnvalidatedConsent] matched what is expected
	pub fn new(
		scope_id: &ScopeId,
		unvalidated: UnvalidatedSigned<UnvalidatedConsent>,
	) -> Result<Self, BadSignature> {
		let signed_bytes = Self::signing_bytes(scope_id, &unvalidated.body);

		if !unvalidated
			.sig
			.verify(&unvalidated.body.consenter, &signed_bytes)
		{
			return Err(BadSignature);
		}

		Ok(Self(unvalidated, PhantomData))
	}

	/// Returns the underlying [UnvalidatedSigned]
	#[must_use]
	pub fn inner(&self) -> &UnvalidatedSigned<UnvalidatedConsent> {
		&self.0
	}

	/// Returns the underlying [UnvalidatedSigned]
	#[must_use]
	pub fn into_inner(self) -> UnvalidatedSigned<UnvalidatedConsent> {
		self.0
	}

	/// Returns the consenter
	#[must_use]
	pub fn consenter(&self) -> &PublicKey {
		&self.inner().body.consenter
	}

	pub(crate) fn as_cbor(&self) -> cbor_core::Value<'_> {
		cbor_core::map! {
			"nonce" => self.inner().body.nonce,
			"consenter" => self.inner().body.consenter.to_string(),
			"sig" => self.inner().sig.to_string(),
		}
	}
}

impl<T: Tagged> Serialize for Consent<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.inner().serialize(serializer)
	}
}
