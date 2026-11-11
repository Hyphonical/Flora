//! Buffer reservations, resumable chunk uploads, verification, and crash recovery.

use super::{App, Result, auth, bad, conflict, db, files, forbidden, missing, trash};
use crate::core::{self, Chunk, CompleteRequest, FileRecord, Upload, UploadRequest};
use axum::{
	Json,
	body::Body,
	extract::{Path, State},
	http::{HeaderMap, StatusCode},
};
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;
use xxhash_rust::xxh3::Xxh3;

fn chunk_path(app: &App, upload: &Upload, number: usize) -> std::path::PathBuf {
	app.buffer.join(&upload.owner).join(&upload.id).join(number.to_string())
}

pub(super) fn staging_path(app: &App, id: &str) -> std::path::PathBuf {
	app.storage.join(".staging").join(id)
}

fn validate_upload_id(id: &str) -> Result<()> {
	Uuid::parse_str(id).map_err(|_| missing())?;
	Ok(())
}

pub(super) async fn remove_upload_artifacts(
	app: &App,
	owner: &str,
	id: &str,
) -> std::io::Result<()> {
	match tokio::fs::remove_dir_all(app.buffer.join(owner).join(id)).await {
		Ok(()) => {}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		Err(e) => return Err(e),
	}
	let legacy_staging = app.storage.join(owner).join(".trash").join(".staging").join(id);
	for path in [staging_path(app, id), legacy_staging] {
		match tokio::fs::remove_file(path).await {
			Ok(()) => {}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(e),
		}
	}
	Ok(())
}

fn ensure_access(p: &auth::Principal, upload: &Upload) -> Result<()> {
	if (p.user.id != upload.owner && !p.admin()) || !p.permits(&upload.path, true) {
		return Err(forbidden());
	}
	Ok(())
}

async fn available(app: &App, uploads: &[Upload], additional: u64) -> Result<()> {
	let used = uploads
		.iter()
		.try_fold(0u64, |a, u| a.checked_add(u.reserved))
		.ok_or_else(|| bad("buffer reservations overflow"))?;
	if additional > app.buffer_size.saturating_sub(used) {
		return Err(conflict(format!(
			"buffer is busy: {} bytes free, {additional} needed",
			app.buffer_size.saturating_sub(used)
		)));
	}
	let mut unwritten_buffer = additional;
	let mut unwritten_storage = additional;
	for upload in uploads {
		let written: u64 = upload
			.chunks
			.iter()
			.flatten()
			.filter(|c| !c.hash.is_empty())
			.fold(0u64, |sum, c| sum.saturating_add(c.size));
		unwritten_buffer = unwritten_buffer.saturating_add(upload.reserved.saturating_sub(written));
		let copied = if upload.copying {
			match tokio::fs::metadata(staging_path(app, &upload.id)).await {
				Ok(metadata) => metadata.len(),
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
				Err(error) => return Err(error.into()),
			}
		} else {
			0
		};
		unwritten_storage =
			unwritten_storage.saturating_add(upload.reserved.saturating_sub(copied));
	}
	let buffer_dir = app.buffer.clone();
	let disk_limit = app.disk_limit;
	let (free, total) = tokio::task::spawn_blocking(move || -> std::io::Result<(u64, u64)> {
		let free = fs2::available_space(&buffer_dir)?;
		let total = if disk_limit.is_some() { fs2::total_space(&buffer_dir)? } else { 0 };
		Ok((free, total))
	})
	.await??;
	if unwritten_buffer > free {
		return Err(conflict("not enough free disk space for buffer reservations"));
	}
	if let Some(limit) = disk_limit {
		let used_disk = total.saturating_sub(free);
		let pending = unwritten_buffer.saturating_add(unwritten_storage);
		if pending > free || used_disk.saturating_add(pending) > limit {
			return Err(conflict("disk limit would be exceeded"));
		}
	}
	Ok(())
}

async fn reserve_quota(
	app: &App,
	user: &core::User,
	uploads: &[Upload],
	requested: u64,
) -> Result<()> {
	if let Some(quota) = user.quota {
		let used = files::usage(app, &user.id).await?;
		let reserved = uploads
			.iter()
			.filter(|u| u.owner == user.id)
			.map(|u| u.reserved)
			.fold(0u64, u64::saturating_add);
		if used.saturating_add(reserved).saturating_add(requested) > quota {
			return Err(conflict("user quota would be exceeded"));
		}
	}
	Ok(())
}

fn renamed(path: &str, n: u32) -> String {
	let (dir, name) = path.rsplit_once('/').map_or(("", path), |(a, b)| (a, b));
	let (stem, ext) = name
		.rsplit_once('.')
		.filter(|(stem, _)| !stem.is_empty())
		.map_or((name, ""), |(s, e)| (s, e));
	let ext = if ext.is_empty() { String::new() } else { format!(".{ext}") };
	if dir.is_empty() { format!("{stem} ({n}){ext}") } else { format!("{dir}/{stem} ({n}){ext}") }
}

pub(super) async fn create(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<UploadRequest>,
) -> Result<Json<Upload>> {
	let p = auth::authenticate(&app, &headers).await?;
	Ok(Json(begin_upload(&app, &p, req, true).await?))
}

pub(super) async fn begin_upload(
	app: &App,
	p: &auth::Principal,
	req: UploadRequest,
	allow_resume: bool,
) -> Result<Upload> {
	let owner = files::owner(app, p, req.owner.as_deref()).await?;
	let user: core::User = app.db.get_async(&db::user(&owner)).await?.ok_or_else(missing)?;
	let mut path = core::clean_path(&req.path, false).map_err(bad)?;
	let requested_path = path.clone();
	if !p.permits(&path, true) {
		return Err(forbidden());
	}
	if req.size.is_some_and(|s| s > app.buffer_size) {
		return Err(bad(format!(
			"file is larger than buffer (FLORA_BUFFER_SIZE): {} bytes",
			app.buffer_size
		)));
	}
	let _user_guard = app.user_locks.lock(&owner).await;
	let _res_guard = app.reservation.lock().await;
	let existing_uploads: Vec<Upload> = app.db.list_async("upload/").await?;
	if allow_resume
		&& req.size.is_some()
		&& let Some(existing) = existing_uploads.iter().find(|u| {
			u.owner == owner
				&& (u.requested_path == path || (u.requested_path.is_empty() && u.path == path))
				&& u.size == req.size
				&& u.conflict == req.conflict
		}) {
		if existing.copying {
			return Err(conflict("the verified upload is awaiting storage"));
		}
		let locks = app.transfer_locks.get(&existing.id);
		let Ok(_idle) = locks.gate.try_write() else {
			return Err(conflict("another client is actively uploading this file"));
		};
		if existing.conflict == core::ConflictPolicy::Reject
			&& app.db.get_async::<FileRecord>(&db::file(&owner, &existing.path)).await?.is_some()
		{
			return Err(conflict("file already exists"));
		}
		tracing::debug!(upload = %existing.id, "resuming buffered upload");
		return Ok(existing.clone());
	}
	let root = app.storage.join(&owner);
	let mut exists = app.db.get_async::<FileRecord>(&db::file(&owner, &path)).await?.is_some();
	if exists && req.conflict == core::ConflictPolicy::Reject {
		return Err(conflict("file already exists"));
	}
	if exists && req.conflict == core::ConflictPolicy::Rename {
		for n in 2..=u32::MAX {
			let candidate = renamed(&path, n);
			core::clean_path(&candidate, false).map_err(bad)?;
			let destination = core::safe_join_async(&root, &candidate).await.map_err(bad)?;
			if !tokio::fs::try_exists(&destination).await?
				&& app.db.get_async::<FileRecord>(&db::file(&owner, &candidate)).await?.is_none()
				&& !existing_uploads.iter().any(|u| u.owner == owner && u.path == candidate)
			{
				path = candidate;
				exists = false;
				break;
			}
		}
		if exists {
			return Err(conflict("could not find an unused file name"));
		}
	}
	if existing_uploads.iter().any(|u| u.owner == owner && u.path == path) {
		return Err(conflict("another upload is using this path"));
	}
	if !p.permits(&path, true) {
		return Err(forbidden());
	}
	let destination = core::safe_join_async(&root, &path).await.map_err(bad)?;
	if let Ok(metadata) = tokio::fs::metadata(&destination).await {
		if metadata.is_dir() {
			return Err(conflict("destination is a directory"));
		}
		if app.db.get_async::<FileRecord>(&db::file(&owner, &path)).await?.is_none() {
			return Err(conflict("destination exists outside the file index"));
		}
	}
	let mut parent = destination.parent();
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
	let reserved = req.size.unwrap_or(0);
	available(app, &existing_uploads, reserved).await?;
	reserve_quota(app, &user, &existing_uploads, reserved).await?;
	let upload = Upload {
		id: Uuid::new_v4().to_string(),
		owner,
		path,
		requested_path,
		size: req.size,
		reserved,
		chunk_size: app.chunk_size,
		chunks: Vec::new(),
		conflict: req.conflict,
		updated: core::now(),
		copying: false,
		final_hash: None,
	};
	if allow_resume {
		tokio::fs::create_dir_all(app.buffer.join(&upload.owner).join(&upload.id)).await?;
	}
	app.db.put_async(&db::upload(&upload.id), &upload).await?;
	tracing::debug!(upload = %upload.id, reserved = upload.reserved, "upload reserved");
	Ok(upload)
}

pub(super) async fn status(
	State(app): State<App>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> Result<Json<Upload>> {
	validate_upload_id(&id)?;
	let p = auth::authenticate(&app, &headers).await?;
	let upload: Upload = app.db.get_async(&db::upload(&id)).await?.ok_or_else(missing)?;
	ensure_access(&p, &upload)?;
	Ok(Json(upload))
}

pub(super) async fn chunk(
	State(app): State<App>,
	Path((id, number)): Path<(String, usize)>,
	headers: HeaderMap,
	body: Body,
) -> Result<Json<Chunk>> {
	validate_upload_id(&id)?;
	let p = auth::authenticate(&app, &headers).await?;
	let declared: u64 = headers
		.get(axum::http::header::CONTENT_LENGTH)
		.and_then(|v| v.to_str().ok())
		.and_then(|v| v.parse().ok())
		.ok_or_else(|| bad("chunk Content-Length is required"))?;
	Ok(Json(write_chunk_stream(&app, &p, &id, number, declared, body.into_data_stream()).await?))
}

async fn write_chunk_stream<S, E>(
	app: &App,
	p: &auth::Principal,
	id: &str,
	number: usize,
	declared: u64,
	mut stream: S,
) -> Result<Chunk>
where
	S: futures_util::Stream<Item = std::result::Result<bytes::Bytes, E>> + Unpin,
	E: std::fmt::Display,
{
	let initial: Upload = app.db.get_async(&db::upload(id)).await?.ok_or_else(missing)?;
	ensure_access(p, &initial)?;
	let _permit =
		app.chunk_permits.acquire().await.map_err(|_| conflict("chunk service unavailable"))?;
	let locks = app.transfer_locks.get(id);
	let _upload_guard = locks.gate.read().await;
	let chunk_lock = locks.chunk(number);
	let _chunk_guard = chunk_lock.lock().await;
	if locks.cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}

	let (final_path, temp_path) = {
		let _meta_guard = locks.meta.lock().await;
		let mut upload: Upload = app.db.get_async(&db::upload(id)).await?.ok_or_else(missing)?;
		ensure_access(p, &upload)?;
		if upload.copying {
			return Err(conflict("upload is being stored"));
		}
		let max_chunks = app.buffer_size.div_ceil(upload.chunk_size);
		if number as u64 >= max_chunks {
			return Err(bad("chunk number is outside the buffer"));
		}
		if let Some(size) = upload.size
			&& number as u64 >= size.div_ceil(upload.chunk_size)
		{
			return Err(bad("chunk number is outside the file"));
		}
		if declared == 0 || declared > upload.chunk_size {
			return Err(bad("invalid chunk length"));
		}
		if let Some(total) = upload.size {
			let expected = total
				.saturating_sub((number as u64).saturating_mul(upload.chunk_size))
				.min(upload.chunk_size);
			if declared != expected {
				return Err(bad("chunk has an unexpected size"));
			}
		}
		let final_path = chunk_path(app, &upload, number);
		let temp_path = final_path.with_extension("tmp");
		for path in [&final_path, &temp_path] {
			match tokio::fs::remove_file(path).await {
				Ok(()) => {}
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
				Err(e) => return Err(e.into()),
			}
		}
		if upload.chunks.len() <= number {
			upload.chunks.resize(number + 1, None);
		}
		let previous = upload.chunks[number].take().map_or(0, |c| c.size);
		upload.chunks[number] = Some(Chunk { size: declared, hash: String::new() });
		upload.updated = core::now();
		if upload.size.is_none() {
			let _res_guard = app.reservation.lock().await;
			upload.reserved = upload.reserved.saturating_sub(previous);
			app.db.put_buffered(&db::upload(&upload.id), &upload)?;
			let uploads: Vec<Upload> = app.db.list_async("upload/").await?;
			available(app, &uploads, declared).await?;
			let owner: core::User =
				app.db.get_async(&db::user(&upload.owner)).await?.ok_or_else(missing)?;
			reserve_quota(app, &owner, &uploads, declared).await?;
			upload.reserved = upload.reserved.saturating_add(declared);
			app.db.put_async(&db::upload(&upload.id), &upload).await?;
		} else if previous > 0 {
			app.db.put_buffered(&db::upload(&upload.id), &upload)?;
		}
		(final_path, temp_path)
	};

	let mut file = tokio::fs::File::create(&temp_path).await?;
	let mut hash = Xxh3::new();
	let mut size = 0u64;
	while let Some(next) = locks
		.cancel
		.run_until_cancelled(stream.next())
		.await
		.ok_or_else(|| conflict("upload was cancelled"))?
	{
		let bytes = next.map_err(|e| bad(format!("reading chunk: {e}")))?;
		size = size.checked_add(bytes.len() as u64).ok_or_else(|| bad("chunk too large"))?;
		if size > declared {
			return Err(bad("chunk exceeds declared length"));
		}
		hash.update(&bytes);
		file.write_all(&bytes).await?;
	}
	if size != declared {
		return Err(bad("chunk is shorter than declared"));
	}
	file.sync_all().await?;
	drop(file);
	tokio::fs::rename(temp_path, final_path).await?;
	let result = Chunk { size, hash: core::hex::u128(hash.digest128()) };
	let _meta_guard = locks.meta.lock().await;
	if locks.cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
	let mut upload: Upload = app.db.get_async(&db::upload(id)).await?.ok_or_else(missing)?;
	if upload.chunks.len() <= number {
		upload.chunks.resize(number + 1, None);
	}
	upload.chunks[number] = Some(result.clone());
	upload.updated = core::now();
	app.db.put_async(&db::upload(&upload.id), &upload).await?;
	tracing::debug!(upload = %id, chunk = number, bytes = size, "chunk received");
	Ok(result)
}

pub(super) async fn complete(
	State(app): State<App>,
	Path(id): Path<String>,
	headers: HeaderMap,
	Json(req): Json<CompleteRequest>,
) -> Result<Json<FileRecord>> {
	validate_upload_id(&id)?;
	let p = auth::authenticate(&app, &headers).await?;
	Ok(Json(complete_upload(&app, &p, &id, req).await?))
}

pub(super) async fn complete_upload(
	app: &App,
	p: &auth::Principal,
	id: &str,
	req: CompleteRequest,
) -> Result<FileRecord> {
	if !core::hex::is_hex_128(&req.hash) {
		return Err(bad("invalid XXH3-128 hash"));
	}
	let initial: Upload = app.db.get_async(&db::upload(id)).await?.ok_or_else(missing)?;
	ensure_access(p, &initial)?;
	let locks = app.transfer_locks.get(id);
	let _upload_guard = locks.gate.write().await;
	if locks.cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
	let mut upload: Upload = app.db.get_async(&db::upload(id)).await?.ok_or_else(missing)?;
	ensure_access(p, &upload)?;
	if upload.copying {
		return Err(conflict("upload is being stored"));
	}
	if upload.size.is_some_and(|size| size != req.size) {
		return Err(bad("file size does not match reservation"));
	}
	let count = req.size.div_ceil(upload.chunk_size) as usize;
	if upload.size.is_none() && upload.chunks.len() > count {
		let _res_guard = app.reservation.lock().await;
		for index in count..upload.chunks.len() {
			let path = chunk_path(app, &upload, index);
			for candidate in [&path, &path.with_extension("tmp")] {
				match tokio::fs::remove_file(candidate).await {
					Ok(()) => {}
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
					Err(e) => return Err(e.into()),
				}
			}
			if let Some(chunk) = &upload.chunks[index] {
				upload.reserved = upload.reserved.saturating_sub(chunk.size);
			}
		}
		upload.chunks.truncate(count);
		app.db.put_async(&db::upload(&upload.id), &upload).await?;
	}
	if req.size != upload.reserved {
		return Err(bad("file size does not match reservation"));
	}
	if upload.chunks.len() != count
		|| upload.chunks.iter().any(|c| c.as_ref().is_none_or(|c| c.hash.is_empty()))
	{
		return Err(conflict("some chunks are missing"));
	}
	for (i, c) in upload.chunks.iter().enumerate() {
		let expected =
			req.size.saturating_sub((i as u64) * upload.chunk_size).min(upload.chunk_size);
		if c.as_ref().is_none_or(|c| c.size != expected) {
			return Err(bad("chunk sizes do not match file size"));
		}
	}
	tracing::debug!(upload = %id, chunks = count, "assembling and verifying upload");
	let staging = staging_path(app, &upload.id);
	let normalized_hash = req.hash.to_ascii_lowercase();
	assemble_and_verify_chunks(app, &upload, count, &staging, &normalized_hash, &locks.cancel)
		.await?;
	if locks.cancel.is_cancelled() {
		let _ = tokio::fs::remove_file(&staging).await;
		return Err(conflict("upload was cancelled"));
	}
	upload.copying = true;
	upload.final_hash = Some(normalized_hash.clone());
	upload.size = Some(req.size);
	upload.updated = core::now();
	app.db.put_async(&db::upload(&upload.id), &upload).await?;
	let record = commit_staged(app, &upload, normalized_hash, req.size, &locks.cancel).await?;
	app.transfer_locks.remove(id);
	tracing::debug!(upload = %id, "upload stored");
	Ok(record)
}

pub(super) async fn stream_direct_upload(
	app: &App,
	upload: &mut Upload,
	expected_size: Option<u64>,
	expected_hash: Option<&str>,
	body: Body,
) -> Result<FileRecord> {
	let _permit =
		app.chunk_permits.acquire().await.map_err(|_| conflict("chunk service unavailable"))?;
	let locks = app.transfer_locks.get(&upload.id);
	let _upload_guard = locks.gate.write().await;
	let staging = staging_path(app, &upload.id);
	let mut output = tokio::fs::File::create(&staging).await?;
	let mut stream = body.into_data_stream();
	let mut hash = Xxh3::new();
	let mut total = 0u64;
	while let Some(next) = locks
		.cancel
		.run_until_cancelled(stream.next())
		.await
		.ok_or_else(|| conflict("upload was cancelled"))?
	{
		let data = next.map_err(|error| bad(format!("reading upload: {error}")))?;
		total = total.checked_add(data.len() as u64).ok_or_else(|| bad("upload too large"))?;
		if total > app.buffer_size {
			return Err(bad(format!(
				"file is larger than buffer (FLORA_BUFFER_SIZE): {} bytes",
				app.buffer_size
			)));
		}
		if expected_size.is_some_and(|expected| total > expected) {
			return Err(bad("upload exceeds Content-Length"));
		}
		if expected_size.is_none() && total > upload.reserved {
			let needed = total
				.saturating_sub(upload.reserved)
				.max(upload.chunk_size)
				.min(app.buffer_size - upload.reserved);
			let _res_guard = app.reservation.lock().await;
			let uploads: Vec<Upload> = app.db.list_async("upload/").await?;
			available(app, &uploads, needed).await?;
			let owner: core::User =
				app.db.get_async(&db::user(&upload.owner)).await?.ok_or_else(missing)?;
			reserve_quota(app, &owner, &uploads, needed).await?;
			upload.reserved = upload.reserved.saturating_add(needed);
			upload.updated = core::now();
			app.db.put_buffered(&db::upload(&upload.id), upload)?;
		}
		hash.update(&data);
		output.write_all(&data).await?;
	}
	if expected_size.is_some_and(|expected| total != expected) {
		return Err(bad("upload is shorter than Content-Length"));
	}
	let digest = core::hex::u128(hash.digest128());
	if expected_hash.is_some_and(|expected| !expected.eq_ignore_ascii_case(&digest)) {
		return Err(bad("XXH3-128 checksum mismatch"));
	}
	output.sync_all().await?;
	drop(output);
	if locks.cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
	upload.reserved = total;
	upload.size = Some(total);
	upload.copying = true;
	upload.final_hash = Some(digest.clone());
	upload.updated = core::now();
	app.db.put_async(&db::upload(&upload.id), upload).await?;
	let record = commit_staged(app, upload, digest, total, &locks.cancel).await?;
	app.transfer_locks.remove(&upload.id);
	Ok(record)
}

async fn assemble_and_verify_chunks(
	app: &App,
	upload: &Upload,
	count: usize,
	staging: &std::path::Path,
	expected_hash: &str,
	cancel: &tokio_util::sync::CancellationToken,
) -> Result<()> {
	let mut output = tokio::fs::File::create(staging).await?;
	let mut hash = Xxh3::new();
	let mut buf = vec![0u8; 64 * 1024];
	let write_res: Result<()> = async {
		for i in 0..count {
			let mut file = tokio::fs::File::open(chunk_path(app, upload, i)).await?;
			loop {
				let n = cancel
					.run_until_cancelled(file.read(&mut buf))
					.await
					.ok_or_else(|| conflict("upload was cancelled"))??;
				if n == 0 {
					break;
				}
				hash.update(&buf[..n]);
				output.write_all(&buf[..n]).await?;
			}
		}
		if core::hex::u128(hash.digest128()) != expected_hash {
			return Err(bad("XXH3-128 checksum mismatch"));
		}
		output.sync_all().await?;
		Ok(())
	}
	.await;
	drop(output);
	if write_res.is_err() {
		let _ = tokio::fs::remove_file(staging).await;
	}
	write_res
}

async fn commit_staged(
	app: &App,
	upload: &Upload,
	hash: String,
	size: u64,
	cancel: &tokio_util::sync::CancellationToken,
) -> Result<FileRecord> {
	let root = app.storage.join(&upload.owner);
	let destination = core::safe_join_async(&root, &upload.path).await.map_err(bad)?;
	if let Some(parent) = destination.parent() {
		tokio::fs::create_dir_all(parent).await?;
	}
	let _user_guard = app.user_locks.lock(&upload.owner).await;
	if cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
	if app.db.get_async::<FileRecord>(&db::file(&upload.owner, &upload.path)).await?.is_none()
		&& tokio::fs::try_exists(&destination).await?
	{
		if tokio::fs::metadata(&destination).await?.len() == size
			&& core::hex::hash_file_async(&destination).await? == hash
		{
			return finish(app, upload, hash, size).await;
		}
		return Err(conflict("destination appeared outside the file index"));
	}
	let staging = staging_path(app, &upload.id);
	if tokio::fs::try_exists(&destination).await? {
		if upload.conflict != core::ConflictPolicy::Replace {
			return Err(conflict("file appeared while uploading"));
		}
		trash::delete(app, &upload.owner, &upload.path).await?;
	}
	tokio::task::spawn_blocking(move || core::rename::no_replace(&staging, &destination)).await??;
	finish(app, upload, hash, size).await
}

async fn finish(app: &App, upload: &Upload, hash: String, size: u64) -> Result<FileRecord> {
	let record = FileRecord {
		owner: upload.owner.clone(),
		path: upload.path.clone(),
		size,
		hash,
		modified: core::now(),
	};
	app.db
		.replace_async(&db::upload(&upload.id), &db::file(&upload.owner, &upload.path), &record)
		.await?;
	remove_upload_artifacts(app, &upload.owner, &upload.id).await?;
	Ok(record)
}

pub(super) async fn cancel(
	State(app): State<App>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> Result<StatusCode> {
	validate_upload_id(&id)?;
	let p = auth::authenticate(&app, &headers).await?;
	cancel_upload(&app, &p, &id).await?;
	Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn cancel_upload(app: &App, p: &auth::Principal, id: &str) -> Result<()> {
	let upload: Upload = app.db.get_async(&db::upload(id)).await?.ok_or_else(missing)?;
	ensure_access(p, &upload)?;
	let locks = app.transfer_locks.get(id);
	locks.cancel.cancel();
	let _upload_guard = locks.gate.write().await;
	if let Some(upload) = app.db.get_async::<Upload>(&db::upload(id)).await? {
		ensure_access(p, &upload)?;
		app.db.delete_async(&db::upload(id)).await?;
		remove_upload_artifacts(app, &upload.owner, id).await?;
	}
	app.transfer_locks.remove(id);
	Ok(())
}

pub(super) async fn recover(app: &App) -> anyhow::Result<()> {
	let uploads: Vec<Upload> = app.db.list_async("upload/").await?;
	for upload in uploads.into_iter().filter(|u| u.copying) {
		let locks = app.transfer_locks.get(&upload.id);
		let Ok(_upload_guard) = locks.gate.try_write() else {
			continue;
		};
		let Some(upload) = app.db.get_async::<Upload>(&db::upload(&upload.id)).await? else {
			app.transfer_locks.remove(&upload.id);
			continue;
		};
		if !upload.copying {
			continue;
		}
		let Some(hash) = upload.final_hash.clone() else {
			continue;
		};
		let size = upload.size.unwrap_or(upload.reserved);
		let outcome: Result<FileRecord> = async {
			let staging = staging_path(app, &upload.id);
			let legacy =
				app.storage.join(&upload.owner).join(".trash").join(".staging").join(&upload.id);
			if !tokio::fs::try_exists(&staging).await? && tokio::fs::try_exists(&legacy).await? {
				tokio::fs::rename(&legacy, &staging).await?;
			}
			let root = app.storage.join(&upload.owner);
			let dest = core::safe_join_async(&root, &upload.path).await.map_err(bad)?;
			let dest_ready = app
				.db
				.get_async::<FileRecord>(&db::file(&upload.owner, &upload.path))
				.await?
				.is_none()
				&& tokio::fs::try_exists(&dest).await?;
			if !dest_ready {
				let staged_valid =
					tokio::fs::metadata(&staging).await.is_ok_and(|m| m.len() == size)
						&& core::hex::hash_file_async(&staging).await.is_ok_and(|h| h == hash);
				if !staged_valid {
					let count = size.div_ceil(upload.chunk_size) as usize;
					assemble_and_verify_chunks(app, &upload, count, &staging, &hash, &locks.cancel)
						.await?;
				}
			}
			commit_staged(app, &upload, hash, size, &locks.cancel).await
		}
		.await;
		match outcome {
			Ok(_) => app.transfer_locks.remove(&upload.id),
			Err(error) => {
				tracing::error!(upload = %upload.id, error = %error.1, "could not recover upload");
			}
		}
	}
	Ok(())
}
