use pesde::Url;

pub mod config;
pub mod dirs;
pub mod install;
pub mod reporters;
pub mod scripts;
pub mod source;
pub mod ui;
pub mod vfs;

pub(crate) fn rootless_url(url: &Url) -> &str {
	url.as_str().strip_suffix('/').unwrap_or(url.as_str())
}

pub(crate) fn url_component_encode(input: &str) -> percent_encoding::PercentEncode<'_> {
	percent_encoding::utf8_percent_encode(input, percent_encoding::NON_ALPHANUMERIC)
}
