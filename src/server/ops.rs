//! Rate limiting, activity, dashboard, thumbnail cache, and purge.

use super::{App, Result, auth, bad, db, files, forbidden, trash};
use crate::core::{self, FileRecord, Upload, User};
use axum::{
	Json,
	body::Body,
	extract::{ConnectInfo, Query, Request, State},
	http::{HeaderMap, StatusCode, header},
	middleware::Next,
	response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Cursor, net::IpAddr};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

const MAX_ACTIVITY_LIMIT: usize = 1000;
const DEFAULT_ACTIVITY_LIMIT: usize = 200;
const MAX_THUMBNAIL_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Limits {
	requests: HashMap<(IpAddr, bool), (u64, u32)>,
}

#[derive(Serialize, Deserialize)]
struct Event {
	time: u64,
	client: String,
	method: String,
	path: String,
	status: u16,
}

pub(super) async fn guard(State(app): State<App>, request: Request, next: Next) -> Response {
	let peer = request.extensions().get::<ConnectInfo<std::net::SocketAddr>>().map(|p| p.0.ip());
	let mut ip = peer.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
	if app.behind_proxy
		&& peer.is_some_and(|peer| {
			if app.trusted_proxies.is_empty() {
				peer.is_loopback()
			} else {
				app.trusted_proxies.iter().any(|net| net.contains(&peer))
			}
		})
		&& let Some(forwarded) = request
			.headers()
			.get("CF-Connecting-IP")
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.parse().ok())
	{
		ip = forwarded;
	}
	let login = request.uri().path() == "/api/login" || request.uri().path() == "/api/setup";
	let now = core::now();
	{
		let mut limits = app.limits.lock().await;
		limits.requests.retain(|_, (start, _)| now.saturating_sub(*start) < 120);
		let (start, count) = limits.requests.entry((ip, login)).or_insert((now, 0));
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
	let path = request.uri().path().to_owned();
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
pub(super) struct ActivityQuery {
	limit: Option<usize>,
}

pub(super) async fn activity(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<ActivityQuery>,
) -> Result<Json<Vec<serde_json::Value>>> {
	let principal = auth::authenticate(&app, &headers)?;
	if !principal.admin() {
		return Err(forbidden());
	}
	let limit = query.limit.unwrap_or(DEFAULT_ACTIVITY_LIMIT).clamp(1, MAX_ACTIVITY_LIMIT);
	let events: Vec<serde_json::Value> = app.db.list_rev_limit("activity/", limit)?;
	Ok(Json(events))
}

pub(super) async fn dashboard(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
	let p = auth::authenticate(&app, &headers)?;
	if !p.manage("") {
		return Err(forbidden());
	}
	let users: Vec<User> = if p.admin() { app.db.list("user/")? } else { vec![p.user.clone()] };
	let uploads: Vec<Upload> = app.db.list("upload/")?;
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
pub(super) struct FileQuery {
	path: String,
	owner: Option<String>,
}

pub(super) async fn thumbnail(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<FileQuery>,
) -> Result<Response> {
	let p = auth::authenticate(&app, &headers)?;
	let path = core::clean_path(&query.path, false).map_err(|e| bad(e.to_string()))?;
	let owner = files::owner(&app, &p, query.owner.as_deref())?;
	if !files::allowed(&p, &owner, &path, false) {
		return Err(forbidden());
	}
	let record: FileRecord = app.db.get(&db::file(&owner, &path))?.ok_or_else(super::missing)?;
	if record.size > MAX_THUMBNAIL_SOURCE_BYTES {
		return Err(bad("image is too large for thumbnail generation"));
	}
	let source = core::safe_join_async(&app.storage.join(&owner), &path)
		.await
		.map_err(|e| bad(e.to_string()))?;
	let dir = app.data.join("thumbnails").join(&owner);
	tokio::fs::create_dir_all(&dir).await?;
	let cache = dir.join(format!("{}.png", record.hash));
	match tokio::fs::File::open(&cache).await {
		Ok(file) => {
			let len = file.metadata().await?.len();
			return Ok(Response::builder()
				.header(header::CONTENT_TYPE, "image/png")
				.header(header::CONTENT_LENGTH, len.to_string())
				.body(Body::from_stream(ReaderStream::new(file)))?);
		}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		Err(e) => return Err(e.into()),
	}

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
		let decoded = reader
			.decode()
			.map_err(|e| bad(format!("unsupported or invalid image: {e}")))?;
		let thumb = decoded.thumbnail(320, 320);
		let mut out = Cursor::new(Vec::new());
		thumb.write_to(&mut out, image::ImageFormat::Png).map_err(anyhow::Error::from)?;
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
		.header(header::CONTENT_TYPE, "image/png")
		.header(header::CONTENT_LENGTH, bytes.len().to_string())
		.body(Body::from(bytes))?)
}

#[derive(Deserialize)]
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
	let p = auth::authenticate(&app, &headers)?;
	if req.all && (!p.admin() || req.owner.is_some()) {
		return Err(forbidden());
	}
	let owners: Vec<String> = if req.all {
		app.db.list::<User>("user/")?.into_iter().map(|user| user.id).collect()
	} else {
		vec![files::owner(&app, &p, req.owner.as_deref())?]
	};
	if !p.manage("") {
		return Err(forbidden());
	}
	let _guard = app.mutation.lock().await;
	let uploads: Vec<Upload> = app
		.db
		.list::<Upload>("upload/")?
		.into_iter()
		.filter(|u| owners.contains(&u.owner))
		.collect();
	let mut trash_items = Vec::new();
	let mut trash_count = 0usize;
	for owner in &owners {
		trash::recover_user(&app, owner).await?;
		let items = trash::ready_items(&app, owner).await?;
		trash_count += items.len();
		trash_items.push((owner.clone(), items));
	}
	if req.confirm {
		let locks: Vec<_> =
			uploads.iter().map(|upload| app.transfer_locks.get(&upload.id)).collect();
		for lock in &locks {
			lock.cancel.cancel();
		}
		drop(_guard);
		let mut _upload_guards = Vec::with_capacity(locks.len());
		for lock in &locks {
			_upload_guards.push(lock.gate.write().await);
		}
		let _guard = app.mutation.lock().await;
		for upload in &uploads {
			if app.db.get::<Upload>(&db::upload(&upload.id))?.is_none() {
				continue;
			}
			app.db.delete(&db::upload(&upload.id))?;
			let directory = app.buffer.join(&upload.owner).join(&upload.id);
			match tokio::fs::remove_dir_all(directory).await {
				Ok(()) => {}
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
				Err(e) => return Err(e.into()),
			}
			let staging =
				app.storage.join(&upload.owner).join(".trash").join(".staging").join(&upload.id);
			match tokio::fs::remove_file(staging).await {
				Ok(()) => {}
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
				Err(e) => return Err(e.into()),
			}
		}
		for (owner, items) in &trash_items {
			trash::purge_items(&app, owner, items).await?;
		}
	}
	Ok(Json(
		serde_json::json!({"trash": trash_count, "uploads": uploads.len(), "purged": req.confirm}),
	))
}
