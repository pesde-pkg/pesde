use console::{Style, StyledObject};
use paste::paste;
use std::{fmt::Display, sync::LazyLock};

#[derive(Debug)]
pub struct LazyStyle<T>(LazyLock<T>);

impl LazyStyle<Style> {
	pub fn apply_to<D>(&self, text: D) -> StyledObject<D> {
		LazyLock::force(&self.0).apply_to(text)
	}
}

impl<T: Display> Display for LazyStyle<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", LazyLock::force(&self.0))
	}
}

macro_rules! make_style {
	($name:ident, $color:ident) => {
		make_style!($name, $color());
	};
	($name:ident, $($color:tt)+) => {
		paste! {
			pub static [<$name _STYLE>]: LazyStyle<Style> = LazyStyle(LazyLock::new(||
				Style::new().$($color)+.bold()
			));
		}
	};
}

macro_rules! make_prefix {
    ($name:ident) => {
		paste! {
			pub static [<$name:upper _PREFIX>]: LazyStyle<StyledObject<&'static str>> = LazyStyle(LazyLock::new(||
				[<$name:upper _STYLE>].apply_to(stringify!($name))
			));
		}
	};
}

pub const CLI_COLOR_256: u8 = 214;

make_style!(INFO, cyan);
make_style!(WARN, yellow);
make_prefix!(warn);
make_style!(ERROR, red);
make_prefix!(error);
make_style!(SUCCESS, green);
make_style!(CLI, color256(CLI_COLOR_256));
make_style!(ADDED, green);
make_style!(REMOVED, red);
make_style!(URL, blue().underlined());
