use crate::source::specifiers::DependencySpecifiers;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

/// An override key
#[derive(
    Debug, DeserializeFromStr, SerializeDisplay, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub struct OverrideKey(pub Vec<Vec<String>>);

impl FromStr for OverrideKey {
    type Err = errors::OverrideKeyFromStr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let overrides = s
            .split(',')
            .map(|overrides| overrides.split('>').map(|s| s.to_string()).collect())
            .collect::<Vec<Vec<String>>>();

        if overrides.is_empty() {
            return Err(errors::OverrideKeyFromStr::Empty);
        }

        Ok(Self(overrides))
    }
}

impl Display for OverrideKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.0
                .iter()
                .map(|overrides| {
                    overrides
                        .iter()
                        .map(|o| o.as_str())
                        .collect::<Vec<_>>()
                        .join(">")
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

/// A specifier for an override
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum OverrideSpecifier {
    /// A specifier for a dependency
    Specifier(DependencySpecifiers),
    /// An alias for a dependency the current project depends on
    Alias(String),
}

/// Errors that can occur when interacting with override keys
pub mod errors {
    use thiserror::Error;

    /// Errors that can occur when parsing an override key
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum OverrideKeyFromStr {
        /// The override key is empty
        #[error("empty override key")]
        Empty,
    }
}
