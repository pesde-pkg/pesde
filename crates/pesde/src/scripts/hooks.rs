use crate::__private::ser_display_deser_fromstr;
use crate::source::PackageId;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::marker::PhantomData;
use std::str::FromStr;

/// Hooks are scripts, except they are not meant to be manually executed.
/// Because of this, adding new hooks is not a breaking change; they are a fixed set,
/// using them as general purpose scripts is not allowed.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
#[serde(default)]
pub struct Hooks {
	pub link: Option<Hook<LinkHook>>,
}

#[derive(Debug, Clone)]
pub struct Hook<R>(Box<str>, PhantomData<fn() -> R>);
ser_display_deser_fromstr!(Hook<R>);

impl<R> Hook<R> {
	/// Returns the hook's script.
	#[must_use]
	pub fn script(&self) -> &str {
		&self.0
	}
}

impl<R> Display for Hook<R> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl<R> FromStr for Hook<R> {
	type Err = std::convert::Infallible;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(Hook(s.into(), PhantomData))
	}
}

/// A hook which lets the user specify "context" of the linking process.
/// Inputs:
/// - array of package ids
///
/// Outputs:
/// - partial JSON map of package id to linking contexts:
///   - for Wally packages, a string of code injected between the require call and module return,
///     the module can be referenced by the identifier `module`.
///   - for pesde packages, this value is ignored and should not be specified, its meaning may change in the future.
#[derive(Debug, Clone)]
pub struct LinkHook(pub HashMap<PackageId, Option<Box<str>>>);
