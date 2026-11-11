//! File listing, moves, range downloads, and retention maintenance.

use super::{App, Result, auth, bad, conflict, db, forbidden, missing, profile, trash};
use crate::core::{self, Entry, FileRecord, Upload, User};
use axum::{
	Json,
	body::Body,
	extract::{Path, Query, State},
	http::{HeaderMap, HeaderValue, StatusCode, header},
	response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

#[derive(Default, Deserialize)]
pub(super) struct FileQuery {
	#[serde(default)]
	path: String,
	owner: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct MoveRequest {
	from: String,
	to: String,
	owner: Option<String>,
}

pub(super) fn owner(
	app: &App,
	principal: &auth::Principal,
	requested: Option<&str>,
) -> Result<String> {
	let Some(requested) = requested else {
		return Ok(principal.user.id.clone());
	};
	if requested == principal.user.id || requested.eq_ignore_ascii_case(&principal.user.name) {
		return Ok(principal.user.id.clone());
	}
	if !principal.admin() {
		return Err(forbidden());
	}
	let users: Vec<User> = app.db.list("user/")?;
	users
		.into_iter()
		.find(|u| u.id == requested || u.name.eq_ignore_ascii_case(requested))
		.map(|u| u.id)
		.ok_or_else(missing)
}

pub(super) fn allowed(p: &auth::Principal, owner: &str, path: &str, write: bool) -> bool {
	if owner != p.user.id {
		return p.admin();
	}
	if write { p.manage(path) } else { p.permits(path, false) }
}

pub(super) fn scoped_records(app: &App, owner: &str, prefix: &str) -> Result<Vec<FileRecord>> {
	let mut records: Vec<FileRecord> = app.db.list(&db::files_under(owner, prefix))?;
	if !prefix.is_empty()
		&& let Some(exact) = app.db.get::<FileRecord>(&db::file(owner, prefix))?
	{
		records.push(exact);
	}
	Ok(records)
}

pub(super) async fn shared_file(
	State(app): State<App>,
	Path(token): Path<String>,
	headers: HeaderMap,
) -> Result<Response> {
	let p = auth::authenticate_token(&app, &token)?;
	if p.token.kind != "file" && p.token.kind != "folder" {
		return Err(forbidden());
	}
	let path = p.token.scope.clone();
	shared_response(app, p, headers, path).await
}

pub(super) async fn shared_path(
	State(app): State<App>,
	Path((token, path)): Path<(String, String)>,
	headers: HeaderMap,
) -> Result<Response> {
	let p = auth::authenticate_token(&app, &token)?;
	if p.token.kind != "folder" {
		return Err(forbidden());
	}
	let path = if p.token.scope.is_empty() { path } else { format!("{}/{}", p.token.scope, path) };
	shared_response(app, p, headers, path).await
}

async fn shared_response(
	app: App,
	principal: auth::Principal,
	headers: HeaderMap,
	path: String,
) -> Result<Response> {
	let allow_root = principal.token.kind == "folder";
	let path = core::clean_path(&path, allow_root).map_err(|e| bad(e.to_string()))?;
	if !principal.permits(&path, false) {
		return Err(forbidden());
	}
	let target = core::safe_join_async(&app.storage.join(&principal.user.id), &path)
		.await
		.map_err(|e| bad(e.to_string()))?;
	let metadata = tokio::fs::metadata(target).await.map_err(|error| {
		if error.kind() == std::io::ErrorKind::NotFound { missing() } else { error.into() }
	})?;
	if metadata.is_dir() {
		if principal.token.kind != "folder" {
			return Err(forbidden());
		}
		Ok(list_for(&app, &principal, &principal.user.id, &path).await?.into_response())
	} else {
		if path.is_empty() {
			return Err(forbidden());
		}
		download_for(&app, &principal, &headers, &principal.user.id, &path).await
	}
}

pub(super) async fn usage(app: &App, owner: &str) -> Result<u64> {
	let live_files: Vec<FileRecord> = app.db.list(&db::files(owner))?;
	let live = live_files.iter().fold(0u64, |sum, r| sum.saturating_add(r.size));
	let trashed = match profile::load(app, owner).await {
		Ok(personal) => personal
			.trash
			.values()
			.filter(|entry| entry.state != profile::TrashState::Purging)
			.fold(0u64, |sum, entry| sum.saturating_add(entry.size)),
		Err(_) => 0,
	};
	Ok(live.saturating_add(trashed))
}

pub(super) async fn list(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<FileQuery>,
) -> Result<Json<Vec<Entry>>> {
	let principal = auth::authenticate(&app, &headers)?;
	let path = core::clean_path(&query.path, true).map_err(|e| bad(e.to_string()))?;
	let owner = owner(&app, &principal, query.owner.as_deref())?;
	list_for(&app, &principal, &owner, &path).await
}

async fn list_for(
	app: &App,
	principal: &auth::Principal,
	owner: &str,
	path: &str,
) -> Result<Json<Vec<Entry>>> {
	if !allowed(principal, owner, path, false) {
		return Err(forbidden());
	}
	let files: Vec<FileRecord> = app.db.list(&db::files_under(owner, path))?;
	let prefix = if path.is_empty() { String::new() } else { format!("{path}/") };
	let mut entries = BTreeMap::<String, Entry>::new();
	let directory = core::safe_join_async(&app.storage.join(owner), path)
		.await
		.map_err(|e| bad(e.to_string()))?;
	let mut children = tokio::fs::read_dir(directory).await.map_err(|error| {
		if error.kind() == std::io::ErrorKind::NotFound { missing() } else { error.into() }
	})?;
	while let Some(child) = children.next_entry().await? {
		if !child.file_type().await?.is_dir() {
			continue;
		}
		let name = child.file_name().into_string().map_err(|_| bad("folder name is not UTF-8"))?;
		if name.eq_ignore_ascii_case(".flora") || name.eq_ignore_ascii_case(".trash") {
			continue;
		}
		let full = format!("{prefix}{name}");
		if allowed(principal, owner, &full, false) {
			entries.insert(
				name.clone(),
				Entry { name, path: full, directory: true, size: 0, modified: 0, hash: None },
			);
		}
	}
	for record in files {
		let Some(rest) = record.path.strip_prefix(&prefix) else {
			continue;
		};
		if rest.is_empty() {
			continue;
		}
		let name = rest.split('/').next().unwrap_or(rest);
		let full = format!("{prefix}{name}");
		if !allowed(principal, owner, &full, false) {
			continue;
		}
		if rest.contains('/') {
			entries.entry(name.to_owned()).or_insert(Entry {
				name: name.to_owned(),
				path: full,
				directory: true,
				size: 0,
				modified: 0,
				hash: None,
			});
		} else {
			entries.insert(
				name.to_owned(),
				Entry {
					name: name.to_owned(),
					path: full,
					directory: false,
					size: record.size,
					modified: record.modified,
					hash: Some(record.hash),
				},
			);
		}
	}
	Ok(Json(entries.into_values().collect()))
}

pub(super) async fn remove(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<FileQuery>,
) -> Result<StatusCode> {
	let p = auth::authenticate(&app, &headers)?;
	let path = core::clean_path(&query.path, false).map_err(|e| bad(e.to_string()))?;
	let owner = owner(&app, &p, query.owner.as_deref())?;
	if !allowed(&p, &owner, &path, true) {
		return Err(forbidden());
	}
	let _guard = app.mutation.lock().await;
	let source = core::safe_join_async(&app.storage.join(&owner), &path)
		.await
		.map_err(|e| bad(e.to_string()))?;
	if !tokio::fs::try_exists(source).await? {
		return Err(missing());
	}
	trash::delete(&app, &owner, &path).await?;
	Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn move_file(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<MoveRequest>,
) -> Result<StatusCode> {
	let p = auth::authenticate(&app, &headers)?;
	let from = core::clean_path(&req.from, false).map_err(|e| bad(e.to_string()))?;
	let to = core::clean_path(&req.to, false).map_err(|e| bad(e.to_string()))?;
	let owner = owner(&app, &p, req.owner.as_deref())?;
	if !allowed(&p, &owner, &from, true) || !allowed(&p, &owner, &to, true) {
		return Err(forbidden());
	}
	if from == to || core::scoped(&from, &to) {
		return Err(bad("destination is inside source"));
	}
	let _guard = app.mutation.lock().await;
	let active_uploads: Vec<Upload> = app.db.list("upload/")?;
	if active_uploads.iter().any(|u| {
		u.owner == owner
			&& (core::scoped(&from, &u.path)
				|| core::scoped(&u.path, &from)
				|| core::scoped(&to, &u.path)
				|| core::scoped(&u.path, &to))
	}) {
		return Err(conflict("an active upload is using the source or destination path"));
	}
	let selected = scoped_records(&app, &owner, &from)?;
	if !scoped_records(&app, &owner, &to)?.is_empty() {
		return Err(conflict("destination already exists"));
	}
	let mut ancestor = to.as_str();
	while let Some((parent, _)) = ancestor.rsplit_once('/') {
		if app.db.get::<FileRecord>(&db::file(&owner, parent))?.is_some() {
			return Err(conflict("a parent of destination is a file"));
		}
		ancestor = parent;
	}
	let root = app.storage.join(&owner);
	let source = core::safe_join_async(&root, &from).await.map_err(|e| bad(e.to_string()))?;
	let dest = core::safe_join_async(&root, &to).await.map_err(|e| bad(e.to_string()))?;
	if !tokio::fs::try_exists(&source).await? {
		return Err(missing());
	}
	if tokio::fs::try_exists(&dest).await? {
		return Err(conflict("destination already exists"));
	}
	if let Some(parent) = dest.parent() {
		tokio::fs::create_dir_all(parent).await?;
	}
	tokio::task::spawn_blocking(move || core::rename::no_replace(&source, &dest)).await??;
	let mut deletes = Vec::with_capacity(selected.len());
	let mut puts = Vec::with_capacity(selected.len());
	for mut record in selected {
		deletes.push(db::file(&owner, &record.path));
		record.path = format!("{to}{}", &record.path[from.len()..]);
		puts.push((db::file(&owner, &record.path), record));
	}
	app.db.write_batch(&puts, &deletes)?;
	Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(super) struct CreateFolder {
	path: String,
	owner: Option<String>,
}

pub(super) async fn create_folder(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<CreateFolder>,
) -> Result<StatusCode> {
	let p = auth::authenticate(&app, &headers)?;
	let path = core::clean_path(&req.path, false).map_err(|e| bad(e.to_string()))?;
	let owner = owner(&app, &p, req.owner.as_deref())?;
	if !allowed(&p, &owner, &path, true) {
		return Err(forbidden());
	}
	let _guard = app.mutation.lock().await;
	let root = app.storage.join(&owner);
	let target = core::safe_join_async(&root, &path).await.map_err(|e| bad(e.to_string()))?;
	if tokio::fs::try_exists(&target).await? {
		return Err(conflict("destination already exists"));
	}
	let mut parent = target.parent();
	while let Some(dir) = parent {
		if dir == root {
			break;
		}
		if let Ok(meta) = tokio::fs::metadata(dir).await
			&& !meta.is_dir()
		{
			return Err(conflict("a parent path is a file"));
		}
		parent = dir.parent();
	}
	tokio::fs::create_dir_all(target).await?;
	Ok(StatusCode::CREATED)
}

pub(super) async fn download(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<FileQuery>,
) -> Result<Response> {
	let p = auth::authenticate(&app, &headers)?;
	let path = core::clean_path(&query.path, false).map_err(|e| bad(e.to_string()))?;
	let owner = owner(&app, &p, query.owner.as_deref())?;
	download_for(&app, &p, &headers, &owner, &path).await
}

async fn download_for(
	app: &App,
	p: &auth::Principal,
	headers: &HeaderMap,
	owner: &str,
	path: &str,
) -> Result<Response> {
	if !allowed(p, owner, path, false) {
		return Err(forbidden());
	}
	let record: FileRecord = app.db.get(&db::file(owner, path))?.ok_or_else(missing)?;
	let mut start = 0u64;
	let mut end = record.size.saturating_sub(1);
	let mut partial = false;
	let etag = format!("\"{}\"", record.hash);
	let range_matches =
		headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()).is_none_or(|v| v == etag);
	if let Some(range) = headers.get(header::RANGE).filter(|_| range_matches) {
		let text = range.to_str().map_err(|_| bad("invalid range"))?;
		let range = text.strip_prefix("bytes=").ok_or_else(|| bad("invalid range"))?;
		let (a, b) = range.split_once('-').ok_or_else(|| bad("invalid range"))?;
		if record.size == 0 || b.contains(',') {
			return Err(super::Error(
				StatusCode::RANGE_NOT_SATISFIABLE,
				anyhow::anyhow!("range not satisfiable"),
			));
		}
		if a.is_empty() {
			let suffix: u64 = b.parse().map_err(|_| bad("invalid range"))?;
			if suffix == 0 {
				return Err(bad("invalid range"));
			}
			start = record.size.saturating_sub(suffix);
		} else {
			start = a.parse().map_err(|_| bad("invalid range"))?;
			if !b.is_empty() {
				end = b.parse::<u64>().map_err(|_| bad("invalid range"))?.min(end);
			}
		}
		if start > end || start >= record.size {
			return Err(super::Error(
				StatusCode::RANGE_NOT_SATISFIABLE,
				anyhow::anyhow!("range not satisfiable"),
			));
		}
		partial = true;
	}
	let length = if record.size == 0 { 0 } else { end - start + 1 };
	let file_path = core::safe_join_async(&app.storage.join(owner), path)
		.await
		.map_err(|e| bad(e.to_string()))?;
	let mut file = tokio::fs::File::open(file_path).await.map_err(|error| {
		if error.kind() == std::io::ErrorKind::NotFound { missing() } else { error.into() }
	})?;
	file.seek(std::io::SeekFrom::Start(start)).await?;
	let body = Body::from_stream(ReaderStream::new(file.take(length)));
	let mut response = Response::builder()
		.status(if partial { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
		.header(header::ACCEPT_RANGES, "bytes")
		.header(header::CONTENT_LENGTH, length.to_string())
		.header(header::ETAG, etag)
		.header(
			header::CONTENT_TYPE,
			mime_guess::from_path(path).first_or_octet_stream().as_ref(),
		);
	if partial {
		response =
			response.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{}", record.size));
	}
	let name = path.rsplit('/').next().unwrap_or("download");
	let ascii: String = name
		.chars()
		.map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' { c } else { '_' })
		.collect();
	let encoded = percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC);
	if let Ok(value) =
		HeaderValue::from_str(&format!("inline; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"))
	{
		response = response.header(header::CONTENT_DISPOSITION, value);
	}
	response.body(body).map_err(Into::into)
}

pub(super) async fn maintenance(app: &App) -> anyhow::Result<()> {
	let _guard = app.mutation.lock().await;
	let now = core::now();
	let uploads: Vec<Upload> = app.db.list("upload/")?;
	for upload in uploads {
		if !upload.copying
			&& app.buffer_retention.is_some_and(|age| now.saturating_sub(upload.updated) >= age)
		{
			let locks = app.transfer_locks.get(&upload.id);
			let Ok(_upload_guard) = locks.gate.try_write() else {
				continue;
			};
			app.db.delete(&db::upload(&upload.id))?;
			match tokio::fs::remove_dir_all(app.buffer.join(&upload.owner).join(&upload.id)).await {
				Ok(()) => {}
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
				Err(e) => return Err(e.into()),
			}
		}
	}
	let users: Vec<User> = app.db.list("user/")?;
	for user in users {
		trash::expire(app, &user, now).await.map_err(|error| error.1)?;
	}
	Ok(())
}
