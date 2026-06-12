use fs_err::tokio as fs;
use semver::Version;
use std::env::VarError;
use std::fmt::Display;
use std::str::FromStr;

pub struct Env {
	name: &'static str,
}

impl Env {
	pub fn new(name: &'static str) -> Self {
		Self { name }
	}

	pub async fn try_get(&self) -> Option<String> {
		match std::env::var(self.name) {
			Ok(result) => return Some(result),
			Err(VarError::NotPresent) => {}
			Err(e) => panic!("error reading `{}`: {e}", self.name),
		}

		let file_path = match std::env::var(format!("{}_FILE", self.name)) {
			Ok(result) => result,
			Err(VarError::NotPresent) => return None,
			Err(e) => panic!("error reading `{}_FILE`: {e}", self.name),
		};

		match fs::read_to_string(file_path).await {
			Ok(result) => Some(result.trim().to_string()),
			Err(e) => panic!("error reading `{}_FILE`: {e}", self.name),
		}
	}

	pub async fn try_parse<T>(&self) -> Option<T>
	where
		T: FromStr,
		<T as FromStr>::Err: Display,
	{
		match self.try_get().await.map(|result| result.parse()) {
			Some(Ok(result)) => Some(result),
			Some(Err(e)) => panic!("error parsing `{}`: {e}", self.name),
			None => None,
		}
	}

	pub async fn get(&self) -> String {
		match self.try_get().await {
			Some(result) => result,
			None => panic!(
				"{name} or {name}_FILE is required, but is not set",
				name = self.name
			),
		}
	}

	pub async fn parse<T>(&self) -> T
	where
		T: FromStr,
		<T as FromStr>::Err: Display,
	{
		match self.get().await.parse() {
			Ok(result) => result,
			Err(e) => panic!("error parsing `{}`: {e}", self.name),
		}
	}
}

// Algorithm taken from crates.io
// https://github.com/rust-lang/crates.io/blob/6c50d4111e49211e0e4e3bd955be0594dfbf5c18/migrations/2026-05-26-120000-0000_semver_ord_v2/up.sql
pub fn semver_ord(version: &Version) -> Vec<u8> {
	let mut result = vec![];

	fn ord_num(result: &mut Vec<u8>, num: &str) {
		result.push(num.len() as u8);
		result.extend_from_slice(num.as_bytes());
	}

	ord_num(&mut result, &version.major.to_string());
	ord_num(&mut result, &version.minor.to_string());
	ord_num(&mut result, &version.patch.to_string());

	if version.pre.is_empty() {
		result.push(0x03);
	} else {
		for part in version.pre.split('.') {
			if part.chars().all(|c| c.is_ascii_digit()) {
				result.push(0x01);
				ord_num(&mut result, part);
			} else {
				result.push(0x02);
				result.extend_from_slice(part.as_bytes());
			}
		}
		result.push(0x00);
	}

	result
}
