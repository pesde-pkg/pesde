use fs_err::tokio as fs;
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
