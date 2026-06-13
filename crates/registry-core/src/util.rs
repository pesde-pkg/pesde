use semver::Version;

#[must_use]
pub fn semver_ord(version: &Version) -> Vec<u8> {
	// Algorithm taken from crates.io
	// https://github.com/rust-lang/crates.io/blob/6c50d4111e49211e0e4e3bd955be0594dfbf5c18/migrations/2026-05-26-120000-0000_semver_ord_v2/up.sql

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
