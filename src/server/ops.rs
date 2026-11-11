//! Rate limiting, activity, dashboard, WebP thumbnail cache, and purge.

use super::{App, Result, auth, bad, conflict, db, files, forbidden, trash, uploads};
use crate::core::{self, FileRecord, Upload, User};
use axum::{
	Json,
	body::Body,
	extract::{ConnectInfo, Query, Request, State},
	http::{HeaderMap, Method, StatusCode, header},
	middleware::Next,
	response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::{
	collections::{HashMap, HashSet},
	io::Cursor,
	net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

const MAX_ACTIVITY_LIMIT: usize = 1000;
const DEFAULT_ACTIVITY_LIMIT: usize = 200;
const MAX_THUMBNAIL_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const THUMBNAIL_CACHE_CONTROL: &str = "private, max-age=86400";

#[derive(Default)]
pub(crate) struct Limits {
	requests: HashMap<(IpAddr, bool), (u64, u32)>,
	last_swept: u64,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Event {
	time: u64,
	client: String,
	method: String,
	path: String,
	status: u16,
}

fn is_trusted_proxy(trusted: &[ipnet::IpNet], ip: IpAddr) -> bool {
	if trusted.is_empty() { ip.is_loopback() } else { trusted.iter().any(|net| net.contains(&ip)) }
}

fn extract_client_ip(app: &App, peer: Option<IpAddr>, headers: &HeaderMap) -> IpAddr {
	let fallback = peer.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
	if !app.behind_proxy || !peer.is_some_and(|p| is_trusted_proxy(&app.trusted_proxies, p)) {
		return fallback;
	}
	for name in ["CF-Connecting-IP", "X-Real-IP"] {
		if let Some(ip) = headers
			.get(name)
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.trim().parse::<IpAddr>().ok())
		{
			return ip;
		}
	}
	if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
		let chain: Vec<IpAddr> =
			xff.split(',').filter_map(|part| part.trim().parse::<IpAddr>().ok()).collect();
		if let Some(&untrusted) =
			chain.iter().rev().find(|&&ip| !is_trusted_proxy(&app.trusted_proxies, ip))
		{
			return untrusted;
		}
		if let Some(&first) = chain.first() {
			return first;
		}
	}
	fallback
}

fn rate_bucket_ip(ip: IpAddr) -> IpAddr {
	match ip {
		IpAddr::V4(_) => ip,
		IpAddr::V6(v6) => {
			let s = v6.segments();
			IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
		}
	}
}

async fn is_exempt_transfer(app: &App, method: &Method, path: &str, headers: &HeaderMap) -> bool {
	if *method == Method::PUT
		&& let Some(rest) = path.strip_prefix("/api/uploads/")
		&& let Some((id, num)) = rest.split_once('/')
		&& Uuid::parse_str(id).is_ok()
		&& num.parse::<usize>().is_ok()
	{
		return auth::authenticate(app, headers).await.is_ok();
	}
	if *method == Method::GET && headers.contains_key(header::RANGE) {
		if path == "/api/download" {
			return auth::authenticate(app, headers).await.is_ok();
		}
		if let Some(rest) = path.strip_prefix("/t/")
			&& let Some(token) = rest.split('/').next()
			&& !token.is_empty()
		{
			return auth::authenticate_token(app, token).await.is_ok();
		}
	}
	false
}

pub(super) async fn guard(State(app): State<App>, request: Request, next: Next) -> Response {
	let peer = request.extensions().get::<ConnectInfo<std::net::SocketAddr>>().map(|p| p.0.ip());
	let ip = extract_client_ip(&app, peer, request.headers());
	let path = request.uri().path().to_owned();
	let login = path == "/api/login" || path == "/api/setup";
	let now = core::now();
	if login || !is_exempt_transfer(&app, request.method(), &path, request.headers()).await {
		let bucket = rate_bucket_ip(ip);
		let mut limits = app.limits.lock().await;
		if now.saturating_sub(limits.last_swept) >= 60 || limits.requests.len() > 2048 {
			limits.requests.retain(|_, (start, _)| now.saturating_sub(*start) < 120);
			limits.last_swept = now;
		}
		let (start, count) = limits.requests.entry((bucket, login)).or_insert((now, 0));
		if now.saturating_sub(*start) >= 60 {
			*start = now;
			*count = 0;
		}
		let max = if login { app.login_rate_limit } else { app.rate_limit };
		if *count >= max {
			return (
				StatusCode::TOO_MANY_REQUESTS,
				Json(serde_json::json!({"error": "rate limit exceeded"})),
			)
				.into_response();
		}
		*count += 1;
	}
	let method = request.method().to_string();
	let meaningful = match method.as_str() {
		"POST" => {
			matches!(
				path.as_str(),
				"/api/setup" | "/api/users" | "/api/tokens" | "/api/purge" | "/api/files"
			) || path.ends_with("/complete")
				|| path.ends_with("/restore")
		}
		"PATCH" => path == "/api/files" || path.starts_with("/api/users/"),
		"PUT" => path == "/api/upload",
		"DELETE" => path == "/api/files" || path.starts_with("/api/tokens/"),
		_ => false,
	};
	let response = next.run(request).await;
	if meaningful && response.status().is_success() {
		let event = Event {
			time: now,
			client: ip.to_string(),
			method,
			path,
			status: response.status().as_u16(),
		};
		if let Err(error) =
			app.db.put_buffered(&format!("activity/{now:020}/{}", Uuid::new_v4()), &event)
		{
			tracing::error!(%error, "could not record activity");
		}
	}
	response
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivityQuery {
	limit: Option<usize>,
}

pub(super) async fn activity(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<ActivityQuery>,
) -> Result<Json<Vec<Event>>> {
	let principal = auth::authenticate(&app, &headers).await?;
	if !principal.admin() {
		return Err(forbidden());
	}
	let limit = query.limit.unwrap_or(DEFAULT_ACTIVITY_LIMIT).clamp(1, MAX_ACTIVITY_LIMIT);
	let events: Vec<Event> = app.db.list_rev_limit_async("activity/", limit).await?;
	Ok(Json(events))
}

pub(super) async fn dashboard(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
	let p = auth::authenticate(&app, &headers).await?;
	if !p.manage() {
		return Err(forbidden());
	}
	let users: Vec<User> =
		if p.admin() { app.db.list_async("user/").await? } else { vec![p.user.clone()] };
	let uploads: Vec<Upload> = app.db.list_async("upload/").await?;
	let mut rows = Vec::with_capacity(users.len());
	for user in users {
		let used = files::usage(&app, &user.id).await?;
		let buffer_reserved: u64 =
			uploads.iter().filter(|u| u.owner == user.id).map(|u| u.reserved).sum();
		rows.push(serde_json::json!({
			"id": user.id,
			"name": user.name,
			"quota": user.quota,
			"used": used,
			"buffer_reserved": buffer_reserved,
		}));
	}
	Ok(Json(serde_json::json!({"users": rows, "buffer_size": app.buffer_size})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileQuery {
	path: String,
	owner: Option<String>,
}

fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
	headers
		.get(header::IF_NONE_MATCH)
		.and_then(|v| v.to_str().ok())
		.is_some_and(|raw| raw.split(',').map(str::trim).any(|tag| tag == "*" || tag == etag))
}

fn cached_thumb_response(etag: &str, file: tokio::fs::File, len: u64) -> Result<Response> {
	Ok(Response::builder()
		.header(header::CONTENT_TYPE, "image/webp")
		.header(header::CONTENT_LENGTH, len.to_string())
		.header(header::ETAG, etag)
		.header(header::CACHE_CONTROL, THUMBNAIL_CACHE_CONTROL)
		.body(Body::from_stream(ReaderStream::new(file)))?)
}

pub(super) async fn thumbnail(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<FileQuery>,
) -> Result<Response> {
	let p = auth::authenticate(&app, &headers).await?;
	let path = core::clean_path(&query.path, false).map_err(bad)?;
	let owner = files::owner(&app, &p, query.owner.as_deref()).await?;
	if !files::allowed(&p, &owner, &path, false) {
		return Err(forbidden());
	}
	let record: FileRecord =
		app.db.get_async(&db::file(&owner, &path)).await?.ok_or_else(super::missing)?;
	if record.size > MAX_THUMBNAIL_SOURCE_BYTES {
		return Err(bad("image is too large for thumbnail generation"));
	}
	let etag = format!("\"thumb-{}\"", record.hash);
	let dir = app.data.join("thumbnails").join(&owner);
	let cache = dir.join(format!("{}.webp", record.hash));
	if etag_matches(&headers, &etag) && tokio::fs::try_exists(&cache).await? {
		return Ok(Response::builder()
			.status(StatusCode::NOT_MODIFIED)
			.header(header::ETAG, etag)
			.header(header::CACHE_CONTROL, THUMBNAIL_CACHE_CONTROL)
			.body(Body::empty())?);
	}
	tokio::fs::create_dir_all(&dir).await?;
	match tokio::fs::File::open(&cache).await {
		Ok(file) => {
			let len = file.metadata().await?.len();
			return cached_thumb_response(&etag, file, len);
		}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		Err(e) => return Err(e.into()),
	}

	let _permit =
		app.thumb_permits.acquire().await.map_err(|_| conflict("thumbnail service unavailable"))?;
	if let Ok(file) = tokio::fs::File::open(&cache).await {
		let len = file.metadata().await?.len();
		return cached_thumb_response(&etag, file, len);
	}

	let source = core::safe_join_async(&app.storage.join(&owner), &path).await.map_err(bad)?;
	let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
		let mut reader = image::ImageReader::open(source)
			.map_err(|e| bad(format!("could not open image: {e}")))?
			.with_guessed_format()
			.map_err(|e| bad(format!("could not inspect image: {e}")))?;
		let mut limits = image::Limits::default();
		limits.max_image_width = Some(16_384);
		limits.max_image_height = Some(16_384);
		limits.max_alloc = Some(256 * 1024 * 1024);
		reader.limits(limits);
		let decoded =
			reader.decode().map_err(|e| bad(format!("unsupported or invalid image: {e}")))?;
		let thumb = decoded.thumbnail(320, 320);
		let mut out = Cursor::new(Vec::new());
		thumb.write_to(&mut out, image::ImageFormat::WebP).map_err(anyhow::Error::from)?;
		Ok(out.into_inner())
	})
	.await??;

	let temp = dir.join(format!("{}.{}.tmp", record.hash, Uuid::new_v4()));
	tokio::fs::write(&temp, &bytes).await?;
	if let Err(err) = tokio::fs::rename(&temp, &cache).await {
		let _ = tokio::fs::remove_file(&temp).await;
		if !tokio::fs::try_exists(&cache).await? {
			return Err(err.into());
		}
	}
	Ok(Response::builder()
		.header(header::CONTENT_TYPE, "image/webp")
		.header(header::CONTENT_LENGTH, bytes.len().to_string())
		.header(header::ETAG, etag)
		.header(header::CACHE_CONTROL, THUMBNAIL_CACHE_CONTROL)
		.body(Body::from(bytes))?)
}

pub(super) async fn evict_unused_thumbnails(
	app: &App,
	owner: &str,
	candidates: &HashSet<String>,
) -> Result<()> {
	let live: Vec<FileRecord> = app.db.list_async(&db::files(owner)).await?;
	let personal = super::profile::load(app, owner).await?;
	let mut referenced: HashSet<&str> = live.iter().map(|r| r.hash.as_str()).collect();
	for entry in personal.trash.values() {
		if entry.state != super::profile::TrashState::Purging {
			for record in &entry.records {
				referenced.insert(record.hash.as_str());
			}
		}
	}
	let dir = app.data.join("thumbnails").join(owner);
	for hash in candidates {
		if !referenced.contains(hash.as_str()) {
			for ext in ["webp", "png"] {
				match tokio::fs::remove_file(dir.join(format!("{hash}.{ext}"))).await {
					Ok(()) => {}
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
					Err(e) => return Err(e.into()),
				}
			}
		}
	}
	Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Purge {
	owner: Option<String>,
	#[serde(default)]
	all: bool,
	#[serde(default)]
	confirm: bool,
}

pub(super) async fn purge(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<Purge>,
) -> Result<Json<serde_json::Value>> {
	let p = auth::authenticate(&app, &headers).await?;
	if req.all && (!p.admin() || req.owner.is_some()) {
		return Err(forbidden());
	}
	if !p.manage() {
		return Err(forbidden());
	}
	let owners: Vec<String> = if req.all {
		app.db.list_async::<User>("user/").await?.into_iter().map(|user| user.id).collect()
	} else {
		vec![files::owner(&app, &p, req.owner.as_deref()).await?]
	};
	let active_uploads: Vec<Upload> = app
		.db
		.list_async::<Upload>("upload/")
		.await?
		.into_iter()
		.filter(|u| owners.contains(&u.owner))
		.collect();
	let mut trash_items = Vec::new();
	let mut trash_count = 0usize;
	for owner in &owners {
		let _user_guard = app.user_locks.lock(owner).await;
		trash::recover_user(&app, owner).await?;
		let items = trash::ready_items(&app, owner).await?;
		trash_count += items.len();
		trash_items.push((owner.clone(), items));
	}
	if req.confirm {
		let locks: Vec<_> = active_uploads.iter().map(|u| app.transfer_locks.get(&u.id)).collect();
		for lock in &locks {
			lock.cancel.cancel();
		}
		for (upload, lock) in active_uploads.iter().zip(&locks) {
			let _upload_guard = lock.gate.write().await;
			if app.db.get_async::<Upload>(&db::upload(&upload.id)).await?.is_some() {
				app.db.delete_async(&db::upload(&upload.id)).await?;
				uploads::remove_upload_artifacts(&app, &upload.owner, &upload.id).await?;
			}
			app.transfer_locks.remove(&upload.id);
		}
		for (owner, items) in &trash_items {
			let _user_guard = app.user_locks.lock(owner).await;
			trash::purge_items(&app, owner, items).await?;
		}
	}
	Ok(Json(
		serde_json::json!({"trash": trash_count, "uploads": active_uploads.len(), "purged": req.confirm}),
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn forwarded_headers_respect_proxy_trust_and_rightmost_untrusted_hop() {
		let trusted: Vec<ipnet::IpNet> = vec!["10.0.0.0/8".parse().unwrap()];
		assert!(is_trusted_proxy(&trusted, "10.1.2.3".parse().unwrap()));
		assert!(!is_trusted_proxy(&trusted, "203.0.113.5".parse().unwrap()));

		let v6: IpAddr = "2001:db8:abcd:1234:ffff::99".parse().unwrap();
		let v6_same_64: IpAddr = "2001:db8:abcd:1234::1".parse().unwrap();
		assert_eq!(rate_bucket_ip(v6), rate_bucket_ip(v6_same_64));
	}
}
