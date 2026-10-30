use thiserror::Error;

mod seq;
pub use seq::*;
mod display;
pub use display::*;

/// A value exceeds its limit
#[derive(Debug, Error)]
#[error("value exceeds the maximum length of {limit} (got {actual})")]
pub struct BoundedSizeError {
	/// The maximum allowed length
	pub limit: usize,
	/// The actual length
	pub actual: usize,
}
