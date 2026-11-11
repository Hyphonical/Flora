//! Command-scoped settings and the flag and environment parser.

use super::{ByteSize, ConflictPolicy, bytes_size, duration};
use anyhow::{Context, Result, bail};
use std::{collections::HashMap, path::PathBuf};

pub(crate) struct OptionSpec {
	pub name: &'static str,
	pub alias: Option<char>,
	pub value: bool,
	pub default: Option<&'static str>,
	pub description: &'static str,
	pub commands: &'static str,
}

impl OptionSpec {
	pub(crate) fn applies_to(&self, command: &str) -> bool {
		self.commands == "all" || self.commands.split_whitespace().any(|item| item == command)
	}
}

macro_rules! options {
	($(($name:literal, $alias:expr, $value:literal, $default:expr, $description:literal, $commands:literal)),* $(,)?) => {
		pub(crate) const OPTIONS: &[OptionSpec] = &[$(OptionSpec {
			name: $name,
			alias: $alias,
			value: $value,
			default: $default,
			description: $description,
			commands: $commands,
		}),*];
	};
}

options! {
	("data", None, true, Some("./data"), "Database and cache directory", "serve"),
	("buffer", None, true, Some("./buffer"), "Upload buffer directory", "serve"),
	("storage", None, true, Some("./storage"), "Local storage directory", "serve"),
	("buffer-size", None, true, Some("3GB"), "Upload buffer capacity and maximum file size", "serve"),
	("disk-limit", None, true, None, "Maximum used disk space", "serve"),
	("buffer-retention", None, true, Some("7d"), "Abandoned upload retention", "serve"),
	("trash-retention", None, true, Some("7d"), "Default trash retention", "serve"),
	("chunk-size", None, true, Some("8MB"), "Maximum upload chunk size", "serve"),
	("upload-concurrency", None, true, Some("16"), "Server-wide concurrent chunk operations", "serve"),
	("behind-proxy", None, false, Some("false"), "Trust visitor IP from a trusted proxy", "serve"),
	("trusted-proxies", None, true, None, "Trusted proxy addresses or CIDRs", "serve"),
	("listen", None, true, Some("0.0.0.0:8080"), "Server listen address", "serve"),
	("public-url", None, true, None, "Public server URL", "serve"),
	("rate-limit", None, true, Some("600"), "Requests per minute per client", "serve"),
	("login-rate-limit", None, true, Some("10"), "Login attempts per minute per client", "serve"),
	("log", None, true, Some("info"), "Log level", "all"),
	("server", None, true, None, "Server URL", "upload download ls mv rm cat"),
	("token", None, true, None, "Account authentication token", "login upload download ls mv rm cat"),
	("parallel", None, true, Some("4"), "Concurrent transfer requests", "upload download"),
	("on-conflict", None, true, Some(ConflictPolicy::DEFAULT_NAME), "Conflict policy: reject, rename, replace", "upload"),
	("name", None, true, None, "Name for a file or stdin", "upload"),
	("output", Some('o'), true, None, "Download destination, or - for stdout", "download"),
	("quiet", Some('q'), false, Some("false"), "Only print errors", "all"),
	("no-color", None, false, Some("false"), "Disable color", "all"),
	("help", Some('h'), false, Some("false"), "Show help", "all"),
	("version", Some('V'), false, Some("false"), "Show version", "all"),
}

#[derive(Clone, Copy)]
pub(crate) struct OutputFlags {
	pub quiet: bool,
	pub no_color: bool,
}

impl OutputFlags {
	pub(crate) fn from_process() -> Self {
		let mut quiet = parse_bool_lenient(std::env::var("FLORA_QUIET").ok().as_deref());
		let mut no_color = parse_bool_lenient(std::env::var("FLORA_NO_COLOR").ok().as_deref());
		let mut positional_only = false;
		for arg in std::env::args().skip(1) {
			if !positional_only && arg == "--" {
				positional_only = true;
				continue;
			}
			if positional_only {
				continue;
			}
			if arg == "-q" || arg == "--quiet" {
				quiet = true;
			} else if let Some(val) = arg.strip_prefix("--quiet=") {
				quiet = parse_bool_lenient(Some(val));
			} else if arg == "--no-color" {
				no_color = true;
			} else if let Some(val) = arg.strip_prefix("--no-color=") {
				no_color = parse_bool_lenient(Some(val));
			}
		}
		Self { quiet, no_color }
	}
}

pub(crate) struct GlobalOptions {
	pub log: String,
	pub server: Option<String>,
	pub token: Option<String>,
	pub quiet: bool,
	pub no_color: bool,
}

pub(crate) struct ServeConfig {
	pub data: PathBuf,
	pub buffer: PathBuf,
	pub storage: PathBuf,
	pub buffer_size: ByteSize,
	pub disk_limit: Option<ByteSize>,
	pub buffer_retention: Option<u64>,
	pub trash_retention: String,
	pub chunk_size: u64,
	pub upload_concurrency: usize,
	pub behind_proxy: bool,
	pub trusted_proxies: Vec<ipnet::IpNet>,
	pub listen: String,
	pub public_url: Option<String>,
	pub rate_limit: u32,
	pub login_rate_limit: u32,
}

pub(crate) struct UploadConfig {
	pub label: &'static str,
	pub source: String,
	pub destination: String,
	pub name: Option<String>,
	pub on_conflict: ConflictPolicy,
	pub parallel: usize,
}

pub(crate) struct DownloadConfig {
	pub label: &'static str,
	pub target: String,
	pub output: Option<String>,
	pub parallel: usize,
}

pub(crate) enum Command {
	Help(Option<String>),
	Version,
	Serve(Box<ServeConfig>),
	Login { url: String },
	Upload(UploadConfig),
	Download(DownloadConfig),
	Cat { target: String },
	Ls { path: String },
	Mv { from: String, to: String },
	Rm { path: String },
}

impl Command {
	pub(crate) const fn name(&self) -> &'static str {
		match self {
			Self::Help(_) => "help",
			Self::Version => "version",
			Self::Serve(_) => "serve",
			Self::Login { .. } => "login",
			Self::Upload(c) => c.label,
			Self::Download(c) => c.label,
			Self::Cat { .. } => "cat",
			Self::Ls { .. } => "ls",
			Self::Mv { .. } => "mv",
			Self::Rm { .. } => "rm",
		}
	}
}

pub(crate) fn canonical_command(cmd: &str) -> Option<&'static str> {
	match cmd {
		"serve" => Some("serve"),
		"login" => Some("login"),
		"upload" | "up" => Some("upload"),
		"download" | "down" => Some("download"),
		"cat" => Some("cat"),
		"ls" => Some("ls"),
		"mv" => Some("mv"),
		"rm" => Some("rm"),
		_ => None,
	}
}

pub(crate) struct Config {
	pub global: GlobalOptions,
	pub action: Command,
}

impl Config {
	pub(crate) const fn output_flags(&self) -> OutputFlags {
		OutputFlags { quiet: self.global.quiet, no_color: self.global.no_color }
	}

	pub(crate) fn parse() -> Result<Self> {
		Self::parse_from(std::env::args().skip(1), |env| std::env::var(env))
	}

	fn parse_from(
		mut args: impl Iterator<Item = String>,
		env_lookup: impl Fn(&str) -> std::result::Result<String, std::env::VarError>,
	) -> Result<Self> {
		let mut cli_flags: HashMap<&'static str, String> = HashMap::new();
		let mut command: Option<String> = None;
		let mut positionals = Vec::new();
		let mut positional_only = false;

		while let Some(arg) = args.next() {
			if !positional_only && arg == "--" {
				positional_only = true;
				continue;
			}
			if !positional_only && arg.starts_with('-') && arg != "-" {
				let (spec, inline) = if let Some(long) = arg.strip_prefix("--") {
					let (key, inline) = match long.split_once('=') {
						Some((name, value)) => (name, Some(value)),
						None => (long, None),
					};
					let spec = OPTIONS
						.iter()
						.find(|s| s.name == key)
						.with_context(|| format!("unknown option --{key}"))?;
					(spec, inline)
				} else {
					let rest = &arg[1..];
					let mut chars = rest.chars();
					let ch = chars.next().context("empty option")?;
					let spec = OPTIONS
						.iter()
						.find(|s| s.alias == Some(ch))
						.with_context(|| format!("unknown option -{ch}"))?;
					let tail = &rest[ch.len_utf8()..];
					let inline = if tail.is_empty() {
						None
					} else if spec.value {
						Some(tail.strip_prefix('=').unwrap_or(tail))
					} else {
						bail!("unknown option {arg}");
					};
					(spec, inline)
				};
				let value = if spec.value {
					inline
						.map(str::to_owned)
						.or_else(|| args.next())
						.with_context(|| format!("--{} requires a value", spec.name))?
				} else {
					match inline {
						None => "true".to_owned(),
						Some(v) => {
							parse_bool(spec.name, v)?;
							v.to_owned()
						}
					}
				};
				cli_flags.insert(spec.name, value);
			} else if command.is_none() {
				command = Some(arg);
			} else {
				positionals.push(arg);
			}
		}

		let lookup = |name: &'static str| -> Result<Option<String>> {
			if let Some(value) = cli_flags.get(name) {
				return Ok(Some(value.clone()));
			}
			let env = format!("FLORA_{}", name.replace('-', "_").to_ascii_uppercase());
			match env_lookup(&env) {
				Ok(value) => Ok(Some(value)),
				Err(std::env::VarError::NotPresent) => {
					let default =
						OPTIONS.iter().find(|s| s.name == name).and_then(|s| s.default);
					Ok(default.map(str::to_owned))
				}
				Err(error) => Err(error).with_context(|| format!("reading {env}")),
			}
		};
		let required = |name: &'static str| -> Result<String> {
			lookup(name)?.with_context(|| format!("missing --{name}"))
		};
		let boolean = |name: &'static str| -> Result<bool> { parse_bool(name, &required(name)?) };

		let help = boolean("help")?;
		let version = boolean("version")?;
		let quiet = boolean("quiet")?;
		let no_color = boolean("no-color")?;
		let log = required("log")?;

		if version {
			return Ok(Self {
				global: GlobalOptions { log, server: None, token: None, quiet, no_color },
				action: Command::Version,
			});
		}

		let Some(cmd) = command else {
			if !cli_flags.keys().all(|k| matches!(*k, "help" | "quiet" | "no-color" | "log")) {
				let bad = cli_flags
					.keys()
					.find(|k| !matches!(**k, "help" | "quiet" | "no-color" | "log"))
					.copied()
					.unwrap_or("");
				bail!("option --{bad} requires a command");
			}
			return Ok(Self {
				global: GlobalOptions { log, server: None, token: None, quiet, no_color },
				action: Command::Help(None),
			});
		};

		let Some(canonical) = canonical_command(&cmd) else {
			bail!("unknown command {cmd:?}");
		};

		if help {
			return Ok(Self {
				global: GlobalOptions { log, server: None, token: None, quiet, no_color },
				action: Command::Help(Some(cmd)),
			});
		}

		for &flag in cli_flags.keys() {
			let spec = OPTIONS.iter().find(|s| s.name == flag).context("missing option spec")?;
			if !spec.applies_to(canonical) {
				bail!("option --{flag} is not valid for 'flora {cmd}'");
			}
		}

		let server = if canonical == "serve" || canonical == "login" { None } else { lookup("server")? };
		let token = if canonical == "serve" { None } else { lookup("token")? };
		let global = GlobalOptions { log, server, token, quiet, no_color };

		let action = match canonical {
			"serve" => {
				ensure_max_positionals(&positionals, 0, "flora serve [options]")?;
				Command::Serve(Box::new(parse_serve(&lookup, &required, &boolean)?))
			}
			"login" => {
				ensure_max_positionals(&positionals, 1, "flora login <url>")?;
				let url = positional(&positionals, 0, "flora login <url>")?;
				Command::Login { url }
			}
			"upload" => {
				let usage = format!("flora {cmd} <file|dir|-> [dest/]");
				ensure_max_positionals(&positionals, 2, &usage)?;
				let source = positional(&positionals, 0, &usage)?;
				let destination = positionals.get(1).cloned().unwrap_or_default();
				let parallel = parse_parallel(&required("parallel")?)?;
				let on_conflict = required("on-conflict")?
					.parse::<ConflictPolicy>()
					.context("invalid --on-conflict")?;
				let label = if cmd == "up" { "up" } else { "upload" };
				Command::Upload(UploadConfig {
					label,
					source,
					destination,
					name: lookup("name")?,
					on_conflict,
					parallel,
				})
			}
			"download" => {
				let usage = format!("flora {cmd} <path|url> [-o file|-]");
				ensure_max_positionals(&positionals, 1, &usage)?;
				let target = positional(&positionals, 0, &usage)?;
				let parallel = parse_parallel(&required("parallel")?)?;
				let label = if cmd == "down" { "down" } else { "download" };
				Command::Download(DownloadConfig { label, target, output: lookup("output")?, parallel })
			}
			"cat" => {
				ensure_max_positionals(&positionals, 1, "flora cat <path|url>")?;
				let target = positional(&positionals, 0, "flora cat <path|url>")?;
				Command::Cat { target }
			}
			"ls" => {
				ensure_max_positionals(&positionals, 1, "flora ls [path]")?;
				let path = positionals.first().cloned().unwrap_or_default();
				Command::Ls { path }
			}
			"mv" => {
				ensure_max_positionals(&positionals, 2, "flora mv <from> <to>")?;
				let from = positional(&positionals, 0, "flora mv <from> <to>")?;
				let to = positional(&positionals, 1, "flora mv <from> <to>")?;
				Command::Mv { from, to }
			}
			"rm" => {
				ensure_max_positionals(&positionals, 1, "flora rm <path>")?;
				let path = positional(&positionals, 0, "flora rm <path>")?;
				Command::Rm { path }
			}
			_ => bail!("unknown command {cmd:?}"),
		};

		Ok(Self { global, action })
	}
}

fn parse_serve(
	lookup: &impl Fn(&'static str) -> Result<Option<String>>,
	required: &impl Fn(&'static str) -> Result<String>,
	boolean: &impl Fn(&'static str) -> Result<bool>,
) -> Result<ServeConfig> {
	let positive = |name: &'static str| -> Result<u32> {
		let value: u32 = required(name)?.parse().with_context(|| format!("invalid --{name}"))?;
		if value == 0 {
			bail!("--{name} must be positive");
		}
		Ok(value)
	};
	let upload_concurrency: usize =
		required("upload-concurrency")?.parse().context("invalid --upload-concurrency")?;
	if upload_concurrency == 0 || upload_concurrency > tokio::sync::Semaphore::MAX_PERMITS {
		bail!("--upload-concurrency must be positive and within the semaphore limit");
	}
	let buffer_size: ByteSize = required("buffer-size")?.parse().context("invalid --buffer-size")?;
	let disk_limit = lookup("disk-limit")?
		.map(|limit| limit.parse::<ByteSize>().context("invalid --disk-limit"))
		.transpose()?;
	let trash_retention = required("trash-retention")?;
	duration(&trash_retention)?;
	let trusted_proxies = lookup("trusted-proxies")?
		.unwrap_or_default()
		.split(',')
		.map(str::trim)
		.filter(|s| !s.is_empty())
		.map(|s| {
			s.parse::<ipnet::IpNet>()
				.or_else(|_| s.parse::<std::net::IpAddr>().map(ipnet::IpNet::from))
				.with_context(|| format!("invalid trusted proxy {s:?}"))
		})
		.collect::<Result<Vec<_>>>()?;
	let public_url = lookup("public-url")?
		.map(|value| -> Result<String> {
			let url = reqwest::Url::parse(&value)?;
			if !matches!(url.scheme(), "http" | "https")
				|| url.host_str().is_none()
				|| !url.username().is_empty()
				|| url.password().is_some()
				|| url.query().is_some()
				|| url.fragment().is_some()
			{
				bail!("public-url must be an HTTP(S) base URL without credentials");
			}
			Ok(value.trim_end_matches('/').to_owned())
		})
		.transpose()?;
	Ok(ServeConfig {
		data: required("data")?.into(),
		buffer: required("buffer")?.into(),
		storage: required("storage")?.into(),
		buffer_size,
		disk_limit,
		buffer_retention: duration(&required("buffer-retention")?)?,
		trash_retention,
		chunk_size: bytes_size(&required("chunk-size")?).context("invalid --chunk-size")?,
		upload_concurrency,
		behind_proxy: boolean("behind-proxy")?,
		trusted_proxies,
		listen: required("listen")?,
		public_url,
		rate_limit: positive("rate-limit")?,
		login_rate_limit: positive("login-rate-limit")?,
	})
}

fn parse_parallel(raw: &str) -> Result<usize> {
	let parallel: usize = raw.parse().context("invalid --parallel")?;
	if !(1..=64).contains(&parallel) {
		bail!("--parallel must be between 1 and 64");
	}
	Ok(parallel)
}

fn parse_bool(name: &str, raw: &str) -> Result<bool> {
	match raw.to_ascii_lowercase().as_str() {
		"1" | "true" | "yes" | "on" => Ok(true),
		"0" | "false" | "no" | "off" => Ok(false),
		_ => bail!("--{name} must be true or false"),
	}
}

fn parse_bool_lenient(raw: Option<&str>) -> bool {
	raw.is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

fn positional(positionals: &[String], index: usize, usage: &str) -> Result<String> {
	positionals.get(index).cloned().with_context(|| format!("usage: {usage}"))
}

fn ensure_max_positionals(positionals: &[String], max: usize, usage: &str) -> Result<()> {
	if positionals.len() > max {
		bail!("unexpected argument {:?}; usage: {usage}", positionals[max]);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn parse(args: &[&str]) -> Result<Config> {
		Config::parse_from(args.iter().map(|s| (*s).to_owned()), |_| {
			Err(std::env::VarError::NotPresent)
		})
	}

	#[test]
	fn upload_and_up_parse_identically() {
		let full = parse(&[
			"upload",
			"report.pdf",
			"docs/",
			"--on-conflict",
			"rename",
			"--name",
			"final.pdf",
			"--parallel",
			"8",
		])
		.expect("parse upload");
		let alias = parse(&[
			"up",
			"report.pdf",
			"docs/",
			"--on-conflict",
			"rename",
			"--name",
			"final.pdf",
			"--parallel",
			"8",
		])
		.expect("parse up");

		let (Command::Upload(a), Command::Upload(b)) = (&full.action, &alias.action) else {
			panic!("expected upload commands");
		};
		assert_eq!(a.label, "upload");
		assert_eq!(b.label, "up");
		assert_eq!(a.source, b.source);
		assert_eq!(a.destination, b.destination);
		assert_eq!(a.name, b.name);
		assert_eq!(a.on_conflict, b.on_conflict);
		assert_eq!(a.parallel, b.parallel);
	}

	#[test]
	fn download_and_down_parse_identically() {
		let full = parse(&["download", "docs/report.pdf", "-o", "out.pdf", "--parallel", "6"])
			.expect("parse download");
		let alias = parse(&["down", "docs/report.pdf", "-o", "out.pdf", "--parallel", "6"])
			.expect("parse down");

		let (Command::Download(a), Command::Download(b)) = (&full.action, &alias.action) else {
			panic!("expected download commands");
		};
		assert_eq!(a.label, "download");
		assert_eq!(b.label, "down");
		assert_eq!(a.target, b.target);
		assert_eq!(a.output, b.output);
		assert_eq!(a.parallel, b.parallel);
	}

	#[test]
	fn command_scoped_flags_reject_wrong_commands_on_both_canonical_and_alias() {
		let err_up = parse(&["up", "a.txt", "-o", "b.txt"]).err().expect("reject -o on up");
		assert!(err_up.to_string().contains("not valid for 'flora up'"));

		let err_down =
			parse(&["download", "a.txt", "--on-conflict", "replace"]).err().expect("reject flag");
		assert!(err_down.to_string().contains("not valid for 'flora download'"));
	}
}
