//! Human-facing CLI output, formatting helpers, and scoped progress bars.

use super::style::{self, Color};
use crate::core::config::OutputFlags;
use std::{
	io::{IsTerminal, Write},
	time::Duration,
};

const RULE_WIDTH: usize = 80;
const BAR_WIDTH: usize = 28;

pub(super) fn interactive() -> bool {
	std::io::stderr().is_terminal() && std::env::var_os("TERM").is_none_or(|t| t != "dumb")
}

fn colored(flags: OutputFlags) -> bool {
	interactive() && !flags.no_color
}

pub(super) fn clean(text: &str) -> String {
	text.chars().map(|ch| if ch.is_control() { '\u{FFFD}' } else { ch }).collect()
}

pub(super) fn line(flags: OutputFlags, text: &str, color: Color) {
	if flags.quiet {
		return;
	}
	eprintln!("  {}", style::paint(&clean(text), color, colored(flags)));
}

pub(super) fn blank(flags: OutputFlags) {
	if !flags.quiet && interactive() {
		eprintln!();
	}
}

pub(super) fn event(flags: OutputFlags, command: &str, message: &str) {
	if flags.quiet {
		return;
	}
	if interactive() {
		line(flags, message, Color::Text);
	} else {
		eprintln!(
			"{}  {}  {}",
			chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
			command,
			clean(message),
		);
	}
}

pub(super) fn row(flags: OutputFlags, command: &str, label: &str, value: &str, color: Color) {
	if flags.quiet {
		return;
	}
	if !interactive() {
		event(flags, command, &format!("{label} {value}"));
		return;
	}
	let label = column(label, 10);
	let value = clean(value);
	eprintln!(
		"  {}{}",
		style::paint(&label, Color::Dim, colored(flags)),
		style::paint(&value, color, colored(flags)),
	);
}

pub(super) fn done(flags: OutputFlags, command: &str, label: &str, message: &str) {
	if interactive() {
		row(flags, command, label, &format!("✓  {message}"), Color::Accent);
	} else {
		event(flags, command, &format!("{label} {message}"));
	}
}

pub(super) fn failure(flags: OutputFlags, command: &str, message: &str) {
	let text = clean(message);
	if interactive() {
		eprintln!("  {}", style::paint(&format!("✕  {text}"), Color::Error, colored(flags)),);
	} else if command.is_empty() {
		eprintln!("{}  error  {text}", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"));
	} else {
		eprintln!("{}  {command}  error  {text}", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),);
	}
}

pub(crate) fn error(flags: OutputFlags, command: Option<&str>, message: &str) {
	failure(flags, command.unwrap_or(""), message);
}

pub(super) fn rule(flags: OutputFlags, label: &str) {
	if flags.quiet || !interactive() {
		return;
	}
	let label = clean(label);
	let rest = "━".repeat(RULE_WIDTH.saturating_sub(label.chars().count() + 8));
	eprintln!(
		"  {}  {}  {}",
		style::paint("━━", Color::Muted, colored(flags)),
		style::paint(&label, Color::Heading, colored(flags)),
		style::paint(&rest, Color::Muted, colored(flags)),
	);
}

pub(super) fn column(text: &str, width: usize) -> String {
	if width == 0 {
		return String::new();
	}
	let text = clean(text);
	let count = text.chars().count();
	if count <= width {
		return format!("{text}{}", " ".repeat(width - count));
	}
	let keep = width.saturating_sub(1);
	let mut truncated: String = text.chars().take(keep).collect();
	truncated.push('…');
	truncated
}

pub(super) fn bytes(value: u64) -> String {
	const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
	let mut n = value as f64;
	let mut unit = 0;
	while n >= 1000.0 && unit < UNITS.len() - 1 {
		n /= 1000.0;
		unit += 1;
	}
	if unit == 0 {
		format!("{value} B")
	} else if n >= 10.0 {
		format!("{n:.0} {}", UNITS[unit])
	} else {
		format!("{n:.1} {}", UNITS[unit])
	}
}

pub(super) fn elapsed(duration: Duration) -> String {
	format!("{:.1}s", duration.as_secs_f64())
}

#[derive(Clone, Copy)]
pub(super) enum ProgressUnit {
	Count,
	Bytes,
}

impl ProgressUnit {
	fn format_pair(self, done: u64, total: u64) -> String {
		match self {
			Self::Count => format!("{done}/{total}"),
			Self::Bytes => format!("{} / {}", bytes(done), bytes(total)),
		}
	}

	fn format_single(self, done: u64) -> String {
		match self {
			Self::Count => done.to_string(),
			Self::Bytes => bytes(done),
		}
	}
}

pub(super) struct ProgressBar<'a> {
	flags: OutputFlags,
	command: &'a str,
	label: &'a str,
	unit: ProgressUnit,
	total: Option<u64>,
	plain_lines: bool,
	active: bool,
}

impl<'a> ProgressBar<'a> {
	pub(super) fn new(
		flags: OutputFlags,
		command: &'a str,
		label: &'a str,
		total: Option<u64>,
		unit: ProgressUnit,
		verbose_log: bool,
	) -> Self {
		Self {
			flags,
			command,
			label,
			unit,
			total: total.filter(|n| *n > 0),
			plain_lines: !interactive() || verbose_log,
			active: false,
		}
	}

	pub(super) fn update(&mut self, done: u64) {
		if self.flags.quiet {
			return;
		}
		let Some(total) = self.total else {
			event(
				self.flags,
				self.command,
				&format!("{} {}", self.label, self.unit.format_single(done)),
			);
			return;
		};
		let pair = self.unit.format_pair(done, total);
		if self.plain_lines {
			event(self.flags, self.command, &format!("{} {pair}", self.label));
			return;
		}
		let fill = (done.saturating_mul(BAR_WIDTH as u64) / total).min(BAR_WIDTH as u64) as usize;
		let percent = (done.saturating_mul(100) / total).min(100);
		let label = column(self.label, 10);
		let summary = format!(
			"  {label}{}{}  {percent:>3}%  {pair}",
			"━".repeat(fill),
			"─".repeat(BAR_WIDTH - fill),
		);
		eprint!(
			"{}{}",
			style::clear_line(),
			style::paint(&summary, Color::Accent, colored(self.flags))
		);
		if done >= total {
			eprintln!();
			self.active = false;
		} else {
			self.active = true;
			let _ = std::io::stderr().flush();
		}
	}

	pub(super) fn clear(&mut self) {
		if self.active {
			eprint!("{}", style::clear_line());
			let _ = std::io::stderr().flush();
			self.active = false;
		}
	}
}

impl Drop for ProgressBar<'_> {
	fn drop(&mut self) {
		self.clear();
	}
}
