//! Read-only Smart-HTTP Git proxy to GitHub so `cargo install --git https://flora.hypho.dev` works.

use super::{App, Error, Result, bad, forbidden};
use axum::{
	body::{Body, Bytes},
	extract::{Query, State},
	http::{HeaderMap, StatusCode, header},
	response::Response,
};
use futures_util::TryStreamExt;
use serde::Deserialize;

const UPSTREAM_GIT_REPO: &str = "https://github.com/Hyphonical/Flora.git";

#[derive(Deserialize)]
pub(super) struct RefsQuery {
	service: Option<String>,
}

pub(super) async fn info_refs(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<RefsQuery>,
) -> Result<Response> {
	match query.service.as_deref() {
		Some("git-upload-pack") => {}
		Some("git-receive-pack") => return Err(forbidden()),
		_ => return Err(bad("unsupported git service")),
	}
	let url = format!("{UPSTREAM_GIT_REPO}/info/refs?service=git-upload-pack");
	let mut req = app
		.http
		.get(url)
		.header(header::USER_AGENT, "git/2.47.0")
		.header(header::ACCEPT, "application/x-git-upload-pack-advertisement, */*");
	if let Some(proto) = headers.get("git-protocol").and_then(|v| v.to_str().ok()) {
		req = req.header("Git-Protocol", proto);
	}
	proxy_upstream(req).await
}

pub(super) async fn upload_pack(
	State(app): State<App>,
	headers: HeaderMap,
	body: Bytes,
) -> Result<Response> {
	let content_type = headers
		.get(header::CONTENT_TYPE)
		.and_then(|v| v.to_str().ok())
		.unwrap_or("application/x-git-upload-pack-request");
	if content_type != "application/x-git-upload-pack-request" {
		return Err(bad("expected application/x-git-upload-pack-request"));
	}
	let url = format!("{UPSTREAM_GIT_REPO}/git-upload-pack");
	let mut req = app
		.http
		.post(url)
		.header(header::USER_AGENT, "git/2.47.0")
		.header(header::CONTENT_TYPE, "application/x-git-upload-pack-request")
		.header(header::ACCEPT, "application/x-git-upload-pack-result")
		.body(body);
	if let Some(proto) = headers.get("git-protocol").and_then(|v| v.to_str().ok()) {
		req = req.header("Git-Protocol", proto);
	}
	proxy_upstream(req).await
}

async fn proxy_upstream(request: reqwest::RequestBuilder) -> Result<Response> {
	let upstream = request.send().await.map_err(|e| {
		Error(StatusCode::BAD_GATEWAY, anyhow::anyhow!("upstream git request failed: {e}"))
	})?;
	let status =
		StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
	let mut builder = Response::builder()
		.status(status)
		.header(header::CACHE_CONTROL, "no-cache, max-age=0, must-revalidate");
	if let Some(ct) = upstream.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
		builder = builder.header(header::CONTENT_TYPE, ct);
	}
	let stream = upstream.bytes_stream().map_err(std::io::Error::other);
	Ok(builder.body(Body::from_stream(stream))?)
}
