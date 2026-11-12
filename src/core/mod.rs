//! Shared wire types, size and duration parsers, and path validation.

pub(crate) mod config;
pub(crate) mod hex;
pub(crate) mod logging;
pub(crate) mod rename;
pub(crate) mod walk;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
	collections::HashSet,
	fmt,
	path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct User {
	pub id: String,
	pub name: String,
	pub password_hash: String,
	pub admin: bool,
	pub quota: Option<u64>,
	pub retention: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TokenKind {
	#[default]
	Account,
	File,
	Folder,
}

impl TokenKind {
	pub(crate) const fn as_str(self) -> &'static str {
		match self {
			Self::Account => "account",
			Self::File => "file",
			Self::Folder => "folder",
		}
	}
}

impl fmt::Display for TokenKind {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TransferSource {
	#[default]
	Web,
	Cli,
	Share,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Token {
	pub hash: String,
	pub owner: String,
	pub scope: String,
	#[serde(default)]
	pub kind: TokenKind,
	pub session: bool,
	#[serde(default)]
	pub created: u64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub label: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub secret: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ConflictPolicy {
	#[default]
	Reject,
	Rename,
	Replace,
}

impl ConflictPolicy {
	pub(crate) const DEFAULT_NAME: &'static str = "reject";

	pub(crate) const fn as_str(self) -> &'static str {
		match self {
			Self::Reject => Self::DEFAULT_NAME,
			Self::Rename => "rename",
			Self::Replace => "replace",
		}
	}
}

impl fmt::Display for ConflictPolicy {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

impl std::str::FromStr for ConflictPolicy {
	type Err = anyhow::Error;
	fn from_str(value: &str) -> Result<Self> {
		match value {
			Self::DEFAULT_NAME => Ok(Self::Reject),
			"rename" => Ok(Self::Rename),
			"replace" => Ok(Self::Replace),
			_ => bail!("conflict policy must be reject, rename, or replace"),
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ByteSize {
	Bytes(u64),
	Percent(u8),
}

impl ByteSize {
	pub(crate) fn resolve(self, capacity: u64) -> Result<u64> {
		match self {
			Self::Bytes(bytes) => Ok(bytes),
			Self::Percent(percent) => capacity
				.checked_mul(u64::from(percent))
				.context("size too large")
				.map(|value| value / 100),
		}
	}
}

impl std::str::FromStr for ByteSize {
	type Err = anyhow::Error;

	fn from_str(input: &str) -> Result<Self> {
		let text = input.trim();
		if let Some(percent) = text.strip_suffix('%') {
			let n: u8 = percent.parse().context("invalid percentage")?;
			if n > 100 {
				bail!("percentage exceeds 100%");
			}
			return Ok(Self::Percent(n));
		}
		Ok(Self::Bytes(bytes_size(text)?))
	}
}

pub(crate) fn bytes_size(input: &str) -> Result<u64> {
	let text = input.trim();
	let split = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
	let n: u64 = text[..split].parse().context("invalid size")?;
	let multiplier: u64 = match text[split..].trim().to_ascii_uppercase().as_str() {
		"" | "B" => 1,
		"KB" => 1000,
		"MB" => 1000u64.pow(2),
		"GB" => 1000u64.pow(3),
		"TB" => 1000u64.pow(4),
		"KIB" => 1024,
		"MIB" => 1024u64.pow(2),
		"GIB" => 1024u64.pow(3),
		"TIB" => 1024u64.pow(4),
		unit => bail!("unknown size unit {unit:?}"),
	};
	n.checked_mul(multiplier).context("size too large")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FileRecord {
	pub owner: String,
	pub path: String,
	pub size: u64,
	pub hash: String,
	pub modified: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Upload {
	pub id: String,
	pub owner: String,
	pub path: String,
	#[serde(default)]
	pub requested_path: String,
	#[serde(default)]
	pub source: TransferSource,
	pub size: Option<u64>,
	pub reserved: u64,
	pub chunk_size: u64,
	pub chunks: Vec<Option<Chunk>>,
	pub conflict: ConflictPolicy,
	#[serde(default)]
	pub created: u64,
	pub updated: u64,
	pub copying: bool,
	#[serde(default)]
	pub final_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Chunk {
	pub size: u64,
	pub hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadRequest {
	pub path: String,
	#[serde(default)]
	pub owner: Option<String>,
	pub size: Option<u64>,
	#[serde(default)]
	pub conflict: ConflictPolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompleteRequest {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub hash: Option<String>,
	pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Entry {
	pub name: String,
	pub path: String,
	pub directory: bool,
	pub size: u64,
	pub modified: u64,
	pub hash: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub item_count: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub shared_token: Option<String>,
}

pub(crate) fn now() -> u64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub(crate) fn duration(input: &str) -> Result<Option<u64>> {
	if input == "never" {
		return Ok(None);
	}
	let mut chars = input.chars().peekable();
	let mut seen = HashSet::new();
	let mut total = 0u64;
	let mut any = false;
	while chars.peek().is_some() {
		let mut number = String::new();
		while chars.peek().is_some_and(char::is_ascii_digit) {
			if let Some(c) = chars.next() {
				number.push(c);
			}
		}
		if number.is_empty() {
			bail!("expected a whole number in duration {input:?}");
		}
		let n: u64 = number.parse()?;
		let unit = chars.next().context("duration is missing a unit")?;
		if !seen.insert(unit) {
			bail!("unit {unit:?} appears twice in {input:?}");
		}
		let multiplier = match unit {
			'h' => 3600,
			'd' => 86400,
			'w' => 604800,
			_ => bail!("unknown duration unit {unit:?}; use h, d, w, or never"),
		};
		total = total
			.checked_add(n.checked_mul(multiplier).context("duration too large")?)
			.context("duration too large")?;
		any = true;
	}
	if !any || total == 0 {
		bail!("duration must be positive or never");
	}
	Ok(Some(total))
}

pub(crate) fn clean_path(input: &str, allow_root: bool) -> Result<String> {
	if input.contains('\\') || input.chars().any(char::is_control) {
		bail!("invalid path");
	}
	let input = input.trim_start_matches('/');
	let mut result = Vec::new();
	for component in input.split('/') {
		if component.is_empty() || component == "." {
			continue;
		}
		if component == ".."
			|| component.len() > 255
			|| component.eq_ignore_ascii_case(".flora")
			|| component.eq_ignore_ascii_case(".flora.tmp")
			|| component.eq_ignore_ascii_case(".trash")
		{
			bail!("reserved or invalid path component {component:?}");
		}
		if component.ends_with('.')
			|| component.ends_with(' ')
			|| component.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
		{
			bail!("invalid filename {component:?}; names must work on Windows, Linux, and macOS");
		}
		let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ');
		let upper = stem.to_ascii_uppercase();
		let device = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
			|| upper.strip_prefix("COM").or_else(|| upper.strip_prefix("LPT")).is_some_and(
				|suffix| {
					matches!(
						suffix,
						"1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
					)
				},
			);
		if device {
			bail!("reserved filename {component:?}");
		}
		result.push(component);
	}
	if result.is_empty() && !allow_root {
		bail!("path must name a file");
	}
	Ok(result.join("/"))
}

pub(crate) fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
	let mut result = root.to_path_buf();
	for component in Path::new(relative).components() {
		match component {
			Component::Normal(value) => result.push(value),
			_ => bail!("unsafe path"),
		}
		match std::fs::symlink_metadata(&result) {
			Ok(metadata) if metadata.file_type().is_symlink() => {
				bail!("symbolic links are not allowed in storage paths")
			}
			Ok(_) => {}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(error) => return Err(error.into()),
		}
	}
	Ok(result)
}

pub(crate) async fn safe_join_async(root: &Path, relative: &str) -> Result<PathBuf> {
	let mut result = root.to_path_buf();
	for component in Path::new(relative).components() {
		match component {
			Component::Normal(value) => result.push(value),
			_ => bail!("unsafe path"),
		}
		match tokio::fs::symlink_metadata(&result).await {
			Ok(metadata) if metadata.file_type().is_symlink() => {
				bail!("symbolic links are not allowed in storage paths")
			}
			Ok(_) => {}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(error) => return Err(error.into()),
		}
	}
	Ok(result)
}

pub(crate) fn scoped(scope: &str, path: &str) -> bool {
	scope.is_empty()
		|| path == scope
		|| path.strip_prefix(scope).is_some_and(|rest| rest.starts_with('/'))
}

pub(crate) fn query_flag<'de, D>(deserializer: D) -> std::result::Result<bool, D::Error>
where
	D: serde::Deserializer<'de>,
{
	struct Visitor;
	impl serde::de::Visitor<'_> for Visitor {
		type Value = bool;
		fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
			f.write_str("a boolean or 1/0/true/false")
		}
		fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<bool, E> {
			Ok(v)
		}
		fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<bool, E> {
			match v {
				0 => Ok(false),
				1 => Ok(true),
				_ => Err(E::custom("expected 0 or 1")),
			}
		}
		fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<bool, E> {
			match v.trim().to_ascii_lowercase().as_str() {
				"1" | "true" | "yes" => Ok(true),
				"" | "0" | "false" | "no" => Ok(false),
				_ => Err(E::custom("expected 1, 0, true, or false")),
			}
		}
	}
	deserializer.deserialize_any(Visitor)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn duration_accepts_w_d_h_and_rejects_months_or_duplicates() {
		assert_eq!(duration("never").unwrap(), None);
		assert_eq!(duration("2w1d12h").unwrap(), Some(2 * 604_800 + 86_400 + 12 * 3600));
		assert!(duration("30m").is_err(), "ambiguous 'm' unit must be rejected");
		assert!(duration("1d2d").is_err(), "duplicate unit must be rejected");
		assert!(duration("0h").is_err(), "zero duration must be rejected");
	}

	#[test]
	fn clean_path_enforces_portable_cross_platform_rules() {
		assert_eq!(clean_path("/docs//2026/report.pdf", false).unwrap(), "docs/2026/report.pdf");
		assert_eq!(clean_path("/", true).unwrap(), "");
		assert!(clean_path("/", false).is_err());
		for bad_path in [
			"../secret",
			"docs\\report.pdf",
			".flora",
			".FLORA.TMP",
			"sub/.trash/file",
			"CON.txt",
			"lpt1",
			"trailing.",
			"trailing ",
			"bad:name.txt",
			"bad|name.txt",
		] {
			assert!(clean_path(bad_path, false).is_err(), "expected {bad_path:?} to be rejected");
		}
	}

	#[test]
	fn byte_size_parses_units_and_percentages() {
		assert_eq!("500MB".parse::<ByteSize>().unwrap().resolve(0).unwrap(), 500_000_000);
		assert_eq!("2GiB".parse::<ByteSize>().unwrap().resolve(0).unwrap(), 2 * 1024 * 1024 * 1024);
		assert_eq!("25%".parse::<ByteSize>().unwrap().resolve(4_000).unwrap(), 1_000);
		assert!("101%".parse::<ByteSize>().is_err());
	}
}
