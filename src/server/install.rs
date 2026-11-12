//! Serves embedded installer scripts and redirects client binary requests to GitHub Releases.

use super::{Result, missing};
use axum::{
	extract::Path,
	http::{StatusCode, header},
	response::{IntoResponse, Response},
};

const RELEASE_BASE: &str = "https://github.com/Hyphonical/Flora/releases/latest/download";

pub(super) async fn shell() -> impl IntoResponse {
	(
		[(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
		include_str!("../../assets/install.sh"),
	)
}

pub(super) async fn powershell() -> impl IntoResponse {
	(
		[(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
		include_str!("../../assets/install.ps1"),
	)
}

pub(super) async fn binary(Path(target): Path<String>) -> Result<Response> {
	const TARGETS: &[&str] = &[
		"linux-x86_64",
		"linux-aarch64",
		"macos-x86_64",
		"macos-aarch64",
		"windows-x86_64",
		"windows-aarch64",
	];
	if !TARGETS.contains(&target.as_str()) {
		return Err(missing());
	}
	let ext = if target.starts_with("windows-") { ".exe" } else { "" };
	let location = format!("{RELEASE_BASE}/flora-{target}{ext}");
	Ok(Response::builder()
		.status(StatusCode::FOUND)
		.header(header::LOCATION, location)
		.header(header::CACHE_CONTROL, "no-cache")
		.body(axum::body::Body::empty())?)
}
