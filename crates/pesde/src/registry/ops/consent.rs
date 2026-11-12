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
	/// Returns Ok if the [ScopeId] and [UnvalidatedConsent] matched what is expected
	pub fn new(
		scope_id: &ScopeId,
		unvalidated: UnvalidatedSigned<UnvalidatedConsent>,
	) -> Result<Self, BadSignature> {
		let UnvalidatedConsent { nonce, consenter } = &unvalidated.body;
		let signed_bytes = cbor_core::map! {
			"tag" => T::TAG,
			"scope_id" => scope_id.to_string(),
			"nonce" => nonce,
			"consenter" => consenter.to_string(),
		}
		.encode();

		if !unvalidated.sig.verify(consenter, &signed_bytes) {
			return Err(BadSignature);
		}

		Ok(Self(unvalidated, PhantomData))
	}

	/// Returns the underlying [UnvalidatedSigned]
	#[must_use]
	pub fn inner(&self) -> &UnvalidatedSigned<UnvalidatedConsent> {
		&self.0
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
