use pesde::jiff;

pub mod auth;
pub mod blob;
pub mod db;
pub mod error;
pub mod log;
pub mod tree;

pub fn microsecond_timestamp() -> jiff::Timestamp {
	// we store timestamps in microsecond precision, anything larger is annoying to work with and overall more trouble than it's worth
	jiff::Timestamp::now()
		.round(jiff::Unit::Microsecond)
		.expect("failed to round timestamp to microsecond precision")
}
