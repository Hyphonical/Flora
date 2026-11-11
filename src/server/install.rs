//! Serves embedded installer scripts and platform-specific Flora binaries.

use super::{App, Result, bad, missing};
use axum::{
	body::Body,
	extract::{Path, State},
	http::{StatusCode, header},
	response::{IntoResponse, Response},
};
use tokio_util::io::ReaderStream;

pub(super) async fn shell(State(app): State<App>) -> Result<impl IntoResponse> {
	let base = app
		.public_url
		.as_deref()
		.ok_or_else(|| bad("set FLORA_PUBLIC_URL to enable the installer"))?
		.replace('\'', "'\\''");
	let script = include_str!("../../assets/install.sh").replace("__FLORA_PUBLIC_URL__", &base);
	Ok(([(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")], script))
}

pub(super) async fn powershell(State(app): State<App>) -> Result<impl IntoResponse> {
	let base = app
		.public_url
		.as_deref()
		.ok_or_else(|| bad("set FLORA_PUBLIC_URL to enable the installer"))?
		.replace('\'', "''");
	let script = include_str!("../../assets/install.ps1").replace("__FLORA_PUBLIC_URL__", &base);
	Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], script))
}

pub(super) async fn binary(State(app): State<App>, Path(target): Path<String>) -> Result<Response> {
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
	let supplied = app.data.join("assets").join(format!("flora-{target}"));
	let native = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
	let file = if tokio::fs::try_exists(&supplied).await? {
		supplied
	} else if target == native {
		std::env::current_exe()?
	} else {
		return Err(missing());
	};
	let file = tokio::fs::File::open(file).await?;
	let size = file.metadata().await?.len();
	Ok(Response::builder()
		.status(StatusCode::OK)
		.header(header::CONTENT_TYPE, "application/octet-stream")
		.header(header::CONTENT_LENGTH, size.to_string())
		.body(Body::from_stream(ReaderStream::new(file)))?)
}
