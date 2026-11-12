//! Serves embedded `web/dist` assets and SPA fallback routes.

use axum::{
	body::Body,
	extract::Request,
	http::{Method, StatusCode, header},
	response::{IntoResponse, Response},
};

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

const FALLBACK_INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Flora</title>
<style>
body { background:#0b0d0c; color:#e6ebe8; font-family:ui-monospace,monospace; display:grid; place-items:center; min-height:100vh; margin:0; }
main { max-width:34rem; padding:2rem; border:1px solid #1f2623; border-radius:8px; }
h1 { margin:0 0 0.5rem; font-size:1.25rem; }
p { margin:0; color:#8d9993; line-height:1.5; font-size:0.9rem; }
</style>
</head>
<body><main><h1>Flora Server</h1><p>API is running. Build <code>web/</code> before compiling to embed the web interface.</p></main></body>
</html>"#;

fn find_asset(path: &str) -> Option<(&'static str, &'static [u8])> {
	WEB_ASSETS.iter().find_map(|&(route, mime, bytes)| (route == path).then_some((mime, bytes)))
}

pub(super) fn serve_index() -> Response {
	if let Some((mime, bytes)) = find_asset("/index.html") {
		Response::builder()
			.status(StatusCode::OK)
			.header(header::CONTENT_TYPE, mime)
			.header(header::CACHE_CONTROL, "no-cache")
			.body(Body::from(bytes))
			.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
	} else {
		Response::builder()
			.status(StatusCode::OK)
			.header(header::CONTENT_TYPE, "text/html; charset=utf-8")
			.header(header::CACHE_CONTROL, "no-cache")
			.body(Body::from(FALLBACK_INDEX_HTML))
			.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
	}
}

pub(super) async fn spa_or_asset(request: Request) -> Response {
	if *request.method() != Method::GET && *request.method() != Method::HEAD {
		return StatusCode::NOT_FOUND.into_response();
	}
	let path = request.uri().path();
	if path.starts_with("/api/") {
		return (
			StatusCode::NOT_FOUND,
			[(header::CONTENT_TYPE, "application/json")],
			r#"{"error":"not found"}"#,
		)
			.into_response();
	}
	if let Some((mime, bytes)) = find_asset(path) {
		let immutable = path.starts_with("/assets/") || path.starts_with("/fonts/");
		let cache =
			if immutable { "public, max-age=31536000, immutable" } else { "public, max-age=3600" };
		let body =
			if *request.method() == Method::HEAD { Body::empty() } else { Body::from(bytes) };
		return Response::builder()
			.status(StatusCode::OK)
			.header(header::CONTENT_TYPE, mime)
			.header(header::CONTENT_LENGTH, bytes.len().to_string())
			.header(header::CACHE_CONTROL, cache)
			.body(body)
			.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
	}
	if path == "/" || !path.rsplit('/').next().unwrap_or("").contains('.') {
		return serve_index();
	}
	StatusCode::NOT_FOUND.into_response()
}
