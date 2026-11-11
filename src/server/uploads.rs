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
		let staging =
			app.storage.join(&upload.owner).join(".trash").join(".staging").join(&upload.id);
		let copied = if upload.copying {
			match tokio::fs::metadata(staging).await {
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
	let p = auth::authenticate(&app, &headers)?;
	Ok(Json(begin_upload(&app, &p, req, true).await?))
}

pub(super) async fn begin_upload(
	app: &App,
	p: &auth::Principal,
	req: UploadRequest,
	allow_resume: bool,
) -> Result<Upload> {
	let owner = files::owner(app, p, req.owner.as_deref())?;
	let user: core::User = app.db.get(&db::user(&owner))?.ok_or_else(missing)?;
	let mut path = core::clean_path(&req.path, false).map_err(|e| bad(e.to_string()))?;
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
	let _guard = app.mutation.lock().await;
	let existing_uploads: Vec<Upload> = app.db.list("upload/")?;
	if allow_resume
		&& req.size.is_some()
		&& let Some(existing) = existing_uploads.iter().find(|u| {
			u.owner == owner
				&& (u.requested_path == path || (u.requested_path.is_empty() && u.path == path))
				&& u.size == req.size
				&& u.conflict == req.conflict
		})
	{
		if existing.copying {
			return Err(conflict("the verified upload is awaiting storage"));
		}
		let locks = app.transfer_locks.get(&existing.id);
		let Ok(_idle) = locks.gate.try_write() else {
			return Err(conflict("another client is actively uploading this file"));
		};
		if existing.conflict == core::ConflictPolicy::Reject
			&& app.db.get::<FileRecord>(&db::file(&owner, &existing.path))?.is_some()
		{
			return Err(conflict("file already exists"));
		}
		tracing::debug!(upload = %existing.id, "resuming buffered upload");
		return Ok(existing.clone());
	}
	let root = app.storage.join(&owner);
	let mut exists = app.db.get::<FileRecord>(&db::file(&owner, &path))?.is_some();
	if exists && req.conflict == core::ConflictPolicy::Reject {
		return Err(conflict("file already exists"));
	}
	if exists && req.conflict == core::ConflictPolicy::Rename {
		for n in 2..=u32::MAX {
			let candidate = renamed(&path, n);
			core::clean_path(&candidate, false).map_err(|e| bad(e.to_string()))?;
			let destination =
				core::safe_join_async(&root, &candidate).await.map_err(|e| bad(e.to_string()))?;
			if !tokio::fs::try_exists(&destination).await?
				&& app.db.get::<FileRecord>(&db::file(&owner, &candidate))?.is_none()
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
	let destination = core::safe_join_async(&root, &path).await.map_err(|e| bad(e.to_string()))?;
	if let Ok(metadata) = tokio::fs::metadata(&destination).await {
		if metadata.is_dir() {
			return Err(conflict("destination is a directory"));
		}
		if app.db.get::<FileRecord>(&db::file(&owner, &path))?.is_none() {
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
	tokio::fs::create_dir_all(app.buffer.join(&upload.owner).join(&upload.id)).await?;
	app.db.put(&db::upload(&upload.id), &upload)?;
	tracing::debug!(upload = %upload.id, reserved = upload.reserved, "upload reserved");
	Ok(upload)
}

pub(super) async fn status(
	State(app): State<App>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> Result<Json<Upload>> {
	let p = auth::authenticate(&app, &headers)?;
	let upload: Upload = app.db.get(&db::upload(&id))?.ok_or_else(missing)?;
	ensure_access(&p, &upload)?;
	Ok(Json(upload))
}

pub(super) async fn chunk(
	State(app): State<App>,
	Path((id, number)): Path<(String, usize)>,
	headers: HeaderMap,
	body: Body,
) -> Result<Json<Chunk>> {
	let p = auth::authenticate(&app, &headers)?;
	let declared: u64 = headers
		.get(axum::http::header::CONTENT_LENGTH)
		.and_then(|v| v.to_str().ok())
		.and_then(|v| v.parse().ok())
		.ok_or_else(|| bad("chunk Content-Length is required"))?;
	Ok(Json(write_chunk_stream(&app, &p, &id, number, declared, body.into_data_stream()).await?))
}

pub(super) async fn write_chunk_bytes(
	app: &App,
	p: &auth::Principal,
	id: &str,
	number: usize,
	data: bytes::Bytes,
) -> Result<Chunk> {
	let declared = data.len() as u64;
	let stream =
		futures_util::stream::once(std::future::ready(Ok::<_, std::convert::Infallible>(data)));
	write_chunk_stream(app, p, id, number, declared, stream).await
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
	let _permit =
		app.chunk_permits.acquire().await.map_err(|_| conflict("chunk service unavailable"))?;
	let locks = app.transfer_locks.get(id);
	let _upload_guard = locks.gate.read().await;
	let chunk_lock = locks.chunk(number);
	let _chunk_guard = chunk_lock.lock().await;
	if locks.cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
	let _guard = app.mutation.lock().await;
	let mut upload: Upload = app.db.get(&db::upload(id))?.ok_or_else(missing)?;
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
	if upload.size.is_none() {
		upload.reserved = upload.reserved.saturating_sub(previous);
	}
	app.db.put(&db::upload(&upload.id), &upload)?;
	if upload.size.is_none() {
		let uploads: Vec<Upload> = app.db.list("upload/")?;
		available(app, &uploads, declared).await?;
		let owner: core::User = app.db.get(&db::user(&upload.owner))?.ok_or_else(missing)?;
		reserve_quota(app, &owner, &uploads, declared).await?;
		upload.reserved = upload.reserved.saturating_add(declared);
	}
	upload.chunks[number] = Some(Chunk { size: declared, hash: String::new() });
	upload.updated = core::now();
	app.db.put(&db::upload(&upload.id), &upload)?;
	drop(_guard);
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
	let result = Chunk { size, hash: format!("{:032x}", hash.digest128()) };
	let _guard = app.mutation.lock().await;
	if locks.cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
	let mut upload: Upload = app.db.get(&db::upload(id))?.ok_or_else(missing)?;
	upload.chunks[number] = Some(result.clone());
	upload.updated = core::now();
	app.db.put(&db::upload(&upload.id), &upload)?;
	tracing::debug!(upload = %id, chunk = number, bytes = size, "chunk received");
	Ok(result)
}

pub(super) async fn complete(
	State(app): State<App>,
	Path(id): Path<String>,
	headers: HeaderMap,
	Json(req): Json<CompleteRequest>,
) -> Result<Json<FileRecord>> {
	let p = auth::authenticate(&app, &headers)?;
	Ok(Json(complete_upload(&app, &p, &id, req).await?))
}

pub(super) async fn complete_upload(
	app: &App,
	p: &auth::Principal,
	id: &str,
	req: CompleteRequest,
) -> Result<FileRecord> {
	let locks = app.transfer_locks.get(id);
	let _upload_guard = locks.gate.write().await;
	if locks.cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
	let _guard = app.mutation.lock().await;
	let mut upload: Upload = app.db.get(&db::upload(id))?.ok_or_else(missing)?;
	ensure_access(p, &upload)?;
	if upload.copying {
		return Err(conflict("upload is being stored"));
	}
	if upload.size.is_some_and(|size| size != req.size) {
		return Err(bad("file size does not match reservation"));
	}
	let count = req.size.div_ceil(upload.chunk_size) as usize;
	if upload.size.is_none() && upload.chunks.len() > count {
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
		app.db.put(&db::upload(&upload.id), &upload)?;
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
	drop(_guard);
	tracing::debug!(upload = %id, chunks = count, "verifying upload");
	let mut hash = Xxh3::new();
	let mut buf = vec![0u8; 64 * 1024];
	for i in 0..count {
		let mut file = tokio::fs::File::open(chunk_path(app, &upload, i)).await?;
		loop {
			let n = locks
				.cancel
				.run_until_cancelled(file.read(&mut buf))
				.await
				.ok_or_else(|| conflict("upload was cancelled"))??;
			if n == 0 {
				break;
			}
			hash.update(&buf[..n]);
		}
	}
	let normalized_hash = req.hash.to_ascii_lowercase();
	if format!("{:032x}", hash.digest128()) != normalized_hash {
		return Err(bad("XXH3-128 checksum mismatch"));
	}
	let _guard = app.mutation.lock().await;
	if locks.cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
	upload.copying = true;
	upload.final_hash = Some(normalized_hash.clone());
	upload.size = Some(req.size);
	upload.updated = core::now();
	app.db.put(&db::upload(&upload.id), &upload)?;
	drop(_guard);
	tracing::debug!(upload = %id, "storing verified upload");
	let record = store(app, &upload, normalized_hash, req.size, &locks.cancel).await?;
	tracing::debug!(upload = %id, "upload stored");
	Ok(record)
}

async fn store(
	app: &App,
	upload: &Upload,
	hash: String,
	size: u64,
	cancel: &tokio_util::sync::CancellationToken,
) -> Result<FileRecord> {
	let root = app.storage.join(&upload.owner);
	let destination =
		core::safe_join_async(&root, &upload.path).await.map_err(|e| bad(e.to_string()))?;
	if let Some(parent) = destination.parent() {
		tokio::fs::create_dir_all(parent).await?;
	}
	if app.db.get::<FileRecord>(&db::file(&upload.owner, &upload.path))?.is_none()
		&& tokio::fs::try_exists(&destination).await?
	{
		let mut existing = tokio::fs::File::open(&destination).await?;
		if existing.metadata().await?.len() == size {
			let mut digest = Xxh3::new();
			let mut buffer = vec![0u8; 64 * 1024];
			loop {
				let n = cancel
					.run_until_cancelled(existing.read(&mut buffer))
					.await
					.ok_or_else(|| conflict("upload was cancelled"))??;
				if n == 0 {
					break;
				}
				digest.update(&buffer[..n]);
			}
			if format!("{:032x}", digest.digest128()) == hash {
				let _guard = app.mutation.lock().await;
				if cancel.is_cancelled() {
					return Err(conflict("upload was cancelled"));
				}
				return finish(app, upload, hash, size).await;
			}
		}
		return Err(conflict("destination appeared outside the file index"));
	}
	let staging_dir = root.join(".trash").join(".staging");
	tokio::fs::create_dir_all(&staging_dir).await?;
	let staging = staging_dir.join(&upload.id);
	let mut output = tokio::fs::File::create(&staging).await?;
	for i in 0..upload.chunks.len() {
		let mut chunk = tokio::fs::File::open(chunk_path(app, upload, i)).await?;
		cancel
			.run_until_cancelled(tokio::io::copy(&mut chunk, &mut output))
			.await
			.ok_or_else(|| conflict("upload was cancelled"))??;
	}
	output.sync_all().await?;
	drop(output);
	let _guard = app.mutation.lock().await;
	if cancel.is_cancelled() {
		return Err(conflict("upload was cancelled"));
	}
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
	app.db.replace(&db::upload(&upload.id), &db::file(&upload.owner, &upload.path), &record)?;
	let directory = app.buffer.join(&upload.owner).join(&upload.id);
	match tokio::fs::remove_dir_all(directory).await {
		Ok(()) => {}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		Err(e) => return Err(e.into()),
	}
	Ok(record)
}

pub(super) async fn cancel(
	State(app): State<App>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> Result<StatusCode> {
	let p = auth::authenticate(&app, &headers)?;
	cancel_upload(&app, &p, &id).await?;
	Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn cancel_upload(app: &App, p: &auth::Principal, id: &str) -> Result<()> {
	let upload: Upload = app.db.get(&db::upload(id))?.ok_or_else(missing)?;
	ensure_access(p, &upload)?;
	let locks = app.transfer_locks.get(id);
	locks.cancel.cancel();
	let _upload_guard = locks.gate.write().await;
	let _guard = app.mutation.lock().await;
	let Some(upload) = app.db.get::<Upload>(&db::upload(id))? else {
		return Ok(());
	};
	ensure_access(p, &upload)?;
	app.db.delete(&db::upload(id))?;
	match tokio::fs::remove_dir_all(app.buffer.join(&upload.owner).join(id)).await {
		Ok(()) => {}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		Err(e) => return Err(e.into()),
	}
	let staging = app.storage.join(&upload.owner).join(".trash").join(".staging").join(id);
	match tokio::fs::remove_file(staging).await {
		Ok(()) => {}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		Err(e) => return Err(e.into()),
	}
	Ok(())
}

pub(super) async fn recover(app: &App) -> anyhow::Result<()> {
	let uploads: Vec<Upload> = app.db.list("upload/")?;
	for upload in uploads.into_iter().filter(|u| u.copying) {
		let locks = app.transfer_locks.get(&upload.id);
		let Ok(_upload_guard) = locks.gate.try_write() else {
			continue;
		};
		let Some(upload) = app.db.get::<Upload>(&db::upload(&upload.id))? else {
			continue;
		};
		if !upload.copying {
			continue;
		}
		let Some(hash) = upload.final_hash.clone() else {
			continue;
		};
		if let Err(error) = store(app, &upload, hash, upload.reserved, &locks.cancel).await {
			tracing::error!(upload = %upload.id, error = %error.1, "could not recover upload");
		}
	}
	Ok(())
}
