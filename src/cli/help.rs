//! Help and version presentation built from the parser's option definitions.

use super::{
	render::{clean, column},
	style::{self, Color},
};
use crate::core::config::{OPTIONS, OptionSpec, OutputFlags, canonical_command};
use std::io::IsTerminal;
fn usage(command: &str) -> &'static str {
	match command {
		"serve" => "flora serve [options]",
		"upload" => "flora upload <file|dir|-> [dest/] [options]",
		"up" => "flora up <file|dir|-> [dest/] [options]",
		"download" => "flora download <path|url> [-o file|-] [options]",
		"down" => "flora down <path|url> [-o file|-] [options]",
		"ls" => "flora ls [path] [options]",
		"mv" => "flora mv <from> <to> [options]",
		"rm" => "flora rm <path> [options]",
		"cat" => "flora cat <path|url> [options]",
		"login" => "flora login <url> [options]",
		_ => "flora <command> [options]",
	}
}

fn stdout_interactive() -> bool {
	std::io::stdout().is_terminal() && std::env::var_os("TERM").is_none_or(|t| t != "dumb")
}

fn stdout_colored(flags: OutputFlags) -> bool {
	stdout_interactive() && !flags.no_color
}

fn help_rule(flags: OutputFlags, label: &str) {
	if !stdout_interactive() {
		return;
	}
	let label = clean(label);
	let rest = "━".repeat(80usize.saturating_sub(label.chars().count() + 8));
	let c = stdout_colored(flags);
	println!(
		"  {}  {}  {}",
		style::paint("━━", Color::Muted, c),
		style::paint(&label, Color::Heading, c),
		style::paint(&rest, Color::Muted, c),
	);
}

fn help_line(flags: OutputFlags, text: &str, color: Color) {
	println!("  {}", style::paint(&clean(text), color, stdout_colored(flags)));
}

fn help_blank() {
	println!();
}

fn help_group(flags: OutputFlags, title: &str, description: &str) {
	help_blank();
	help_line(flags, &format!("{title} — {description}"), Color::Heading);
}

fn option(flags: OutputFlags, spec: &OptionSpec) {
	let name = format!("--{}{}", spec.name, if spec.value { " <value>" } else { "" });
	let alias = spec.alias.map(|c| format!("-{c}")).unwrap_or_default();
	let default = spec
		.default
		.filter(|value| *value != "false")
		.map(|value| format!(" (default: {value})"))
		.unwrap_or_default();
	help_line(
		flags,
		&format!("{} {}  {}{default}", column(&name, 24), column(&alias, 3), spec.description),
		Color::Text,
	);
}

pub(super) fn show(flags: OutputFlags, command: Option<&str>) {
	let flags = OutputFlags { quiet: false, ..flags };
	let command = command.unwrap_or("");
	let canonical = canonical_command(command).unwrap_or(command);
	help_rule(flags, if command.is_empty() { "flora" } else { command });
	let description = match canonical {
		"upload" => {
			"Upload a file, a folder, or stdin (alias: up). Interrupted uploads can resume."
		}
		"download" => "Download a file by path or share URL (alias: down).",
		"ls" => "List files and folders.",
		"mv" => "Move or rename a file or folder.",
		"rm" => "Move a file or folder to trash.",
		"cat" => "Print a file to stdout.",
		"login" => "Connect this machine to a server.",
		"serve" => "Run the Flora server.",
		_ => "Permanent file storage you host yourself.",
	};
	help_line(flags, description, Color::Dim);
	help_blank();
	help_line(flags, "USAGE:", Color::Dim);
	help_line(flags, usage(command), Color::Text);
	help_blank();
	help_line(flags, "Flags override FLORA_<FLAG_NAME> environment variables.", Color::Dim);
	if command.is_empty() {
		help_group(flags, "commands", "work with files and run the server");
		for (name, desc) in [
			("upload, up", "Upload a file, a folder, or stdin"),
			("download, down", "Download by path or share URL"),
			("ls", "List files and folders"),
			("mv", "Move or rename"),
			("rm", "Delete (moves to trash)"),
			("cat", "Print a file to stdout"),
			("login", "Connect this machine to a server"),
			("serve", "Run the server"),
		] {
			help_line(flags, &format!("{} {desc}", column(name, 16)), Color::Accent);
		}
	} else {
		help_group(flags, "options", "for this command");
		for spec in OPTIONS.iter().filter(|s| s.commands != "all" && s.applies_to(canonical)) {
			option(flags, spec);
		}
	}
	help_group(flags, "global", "work with every command");
	for spec in OPTIONS.iter().filter(|s| s.commands == "all") {
		option(flags, spec);
	}
	if command.is_empty() {
		help_group(flags, "examples", "common tasks");
		for example in [
			"flora upload report.pdf docs/",
			"flora up photos/ --on-conflict rename",
			"cat db.sql | flora up - --name db.sql",
			"flora download https://files.example.com/t/<token>",
		] {
			help_line(flags, example, Color::Dim);
		}
		help_blank();
		help_line(flags, "Run flora <command> --help for command options.", Color::Muted);
	}
}

pub(super) fn version() {
	println!("flora {}", env!("CARGO_PKG_VERSION"));
}
