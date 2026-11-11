//! Palette and terminal control sequences for the CLI.

use std::fmt;

#[derive(Clone, Copy)]
pub(super) enum Color {
	Heading,
	Text,
	Dim,
	Muted,
	Accent,
	Warning,
	Error,
}

pub(super) struct Styled<'a> {
	text: &'a str,
	color: Color,
	enabled: bool,
}

pub(super) const fn paint(text: &str, color: Color, enabled: bool) -> Styled<'_> {
	Styled { text, color, enabled }
}

impl fmt::Display for Styled<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if !self.enabled {
			return f.write_str(self.text);
		}
		let (r, g, b) = match self.color {
			Color::Heading => (255, 255, 212),
			Color::Text => (228, 228, 196),
			Color::Dim => (119, 131, 133),
			Color::Muted => (79, 84, 86),
			Color::Accent => (175, 185, 122),
			Color::Warning => (194, 168, 108),
			Color::Error => (192, 109, 68),
		};
		write!(f, "\x1b[38;2;{r};{g};{b}m{}\x1b[0m", self.text)
	}
}

pub(super) const fn clear_line() -> &'static str {
	"\r\x1b[2K"
}
