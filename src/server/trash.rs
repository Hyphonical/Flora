//! Trash, restore, retention, and recovery using each user's .flora file.

use super::{
	App, Result, auth, bad, conflict, db, files, forbidden, missing,
	profile::{self, TrashEntry, TrashState},
};
use crate::core::{self, FileRecord, Upload, User};
use axum::{
	Json,
	extract::{Path, Query, State},
	http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Default, Deserialize)]
pub(super) struct TrashQuery {
	owner: Option<String>,
}

async fn paths(
	app: &App,
	owner: &str,
	id: &str,
	entry: &TrashEntry,
) -> Result<(PathBuf, PathBuf, PathBuf)> {
	Uuid::parse_str(id).map_err(|_| bad("invalid trash identifier"))?;
	let path = core::clean_path(&entry.path, false).map_err(|error| bad(error.to_string()))?;
	if path != entry.path {
		return Err(bad("invalid original trash path"));
	}
	for record in &entry.records {
		if record.owner != owner
			|| !core::scoped(&path, &record.path)
			|| core::clean_path(&record.path, false).ok().as_deref() != Some(record.path.as_str())
		{
			return Err(bad("invalid trash file information"));
		}
	}
	let root = app.storage.join(owner);
	let directory = core::safe_join_async(&root, &format!(".trash/{id}"))
		.await
		.map_err(|error| bad(error.to_string()))?;
	let stored = core::safe_join_async(&root, &format!(".trash/{id}/content/{path}"))
		.await
		.map_err(|error| bad(error.to_string()))?;
	let original = core::safe_join_async(&root, &path)
		.await
		.map_err(|error| bad(error.to_string()))?;
	Ok((directory, stored, original))
}

pub(super) async fn delete(app: &App, owner: &str, path: &str) -> Result<()> {
	recover_user(app, owner).await?;
	let records = files::scoped_records(app, owner, path)?;
	let id = Uuid::new_v4().to_string();
	let entry = TrashEntry {
		path: path.to_owned(),
		deleted: core::now(),
		size: records.iter().map(|r| r.size).sum(),
		records,
		state: TrashState::Deleting,
	};
	let (_, stored, original) = paths(app, owner, &id, &entry).await?;
	if let Some(parent) = stored.parent() {
		tokio::fs::create_dir_all(parent).await?;
	}
	let mut personal = profile::load(app, owner).await?;
	personal.trash.insert(id.clone(), entry);
	profile::save(app, owner, &personal).await?;
	tokio::task::spawn_blocking(move || core::rename::no_replace(&original, &stored)).await??;
	let deletes: Vec<String> = personal
		.trash
		.get(&id)
		.ok_or_else(missing)?
		.records
		.iter()
		.map(|record| db::file(owner, &record.path))
		.collect();
	app.db.write_batch::<FileRecord>(&[], &deletes)?;
	personal.trash.get_mut(&id).ok_or_else(missing)?.state = TrashState::Ready;
	profile::save(app, owner, &personal).await?;
	Ok(())
}

pub(super) async fn list(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<TrashQuery>,
) -> Result<Json<Vec<serde_json::Value>>> {
	let principal = auth::authenticate(&app, &headers)?;
	let owner = files::owner(&app, &principal, query.owner.as_deref())?;
	if !files::allowed(&principal, &owner, "", true) {
		return Err(forbidden());
	}
	let _guard = app.mutation.lock().await;
	recover_user(&app, &owner).await?;
	let personal = profile::load(&app, &owner).await?;
	let entries = personal
		.trash
		.into_iter()
		.filter(|(_, entry)| entry.state == TrashState::Ready)
		.map(|(id, entry)| {
			serde_json::json!({
				"id": id,
				"path": entry.path,
				"deleted": entry.deleted,
				"size": entry.size,
			})
		})
		.collect();
	Ok(Json(entries))
}

pub(super) async fn restore(
	State(app): State<App>,
	Path(id): Path<String>,
	headers: HeaderMap,
	Query(query): Query<TrashQuery>,
) -> Result<StatusCode> {
	let principal = auth::authenticate(&app, &headers)?;
	let owner = files::owner(&app, &principal, query.owner.as_deref())?;
	if !files::allowed(&principal, &owner, "", true) {
		return Err(forbidden());
	}
	let _guard = app.mutation.lock().await;
	recover_user(&app, &owner).await?;
	let mut personal = profile::load(&app, &owner).await?;
	let entry = personal.trash.get(&id).cloned().ok_or_else(missing)?;
	if entry.state != TrashState::Ready {
		return Err(conflict("trash item is being recovered"));
	}
	let active_uploads: Vec<Upload> = app.db.list("upload/")?;
	if active_uploads.iter().any(|u| {
		u.owner == owner
			&& (core::scoped(&entry.path, &u.path) || core::scoped(&u.path, &entry.path))
	}) {
		return Err(conflict("an active upload is using the restore path"));
	}
	let mut ancestor = entry.path.as_str();
	while let Some((parent, _)) = ancestor.rsplit_once('/') {
		if app.db.get::<FileRecord>(&db::file(&owner, parent))?.is_some() {
			return Err(conflict("a parent of the original path is now a file"));
		}
		ancestor = parent;
	}
	let (directory, stored, original) = paths(&app, &owner, &id, &entry).await?;
	if !tokio::fs::try_exists(&stored).await? {
		return Err(missing());
	}
	if tokio::fs::try_exists(&original).await? {
		return Err(conflict("a file already exists at the original path"));
	}
	let root = app.storage.join(&owner);
	let mut dir_check = original.parent();
	while let Some(dir) = dir_check {
		if dir == root {
			break;
		}
		if let Ok(meta) = tokio::fs::metadata(dir).await
			&& !meta.is_dir()
		{
			return Err(conflict("a parent of the original path is now a file"));
		}
		dir_check = dir.parent();
	}
	if let Some(parent) = original.parent() {
		tokio::fs::create_dir_all(parent).await?;
	}
	personal.trash.get_mut(&id).ok_or_else(missing)?.state = TrashState::Restoring;
	profile::save(&app, &owner, &personal).await?;
	tokio::task::spawn_blocking(move || core::rename::no_replace(&stored, &original)).await??;
	let puts: Vec<(String, FileRecord)> = entry
		.records
		.into_iter()
		.map(|record| (db::file(&owner, &record.path), record))
		.collect();
	app.db.write_batch(&puts, &[])?;
	personal.trash.remove(&id);
	profile::save(&app, &owner, &personal).await?;
	if tokio::fs::try_exists(&directory).await? {
		tokio::fs::remove_dir_all(directory).await?;
	}
	Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn recover(app: &App) -> Result<()> {
	let users: Vec<User> = app.db.list("user/")?;
	for user in users {
		recover_user(app, &user.id).await?;
	}
	Ok(())
}

pub(super) async fn recover_user(app: &App, owner: &str) -> Result<()> {
	let mut personal = profile::load(app, owner).await?;
	let entries: Vec<_> = personal
		.trash
		.iter()
		.filter(|(_, entry)| entry.state != TrashState::Ready)
		.map(|(id, entry)| (id.clone(), entry.clone()))
		.collect();
	let mut changed = false;
	for (id, entry) in entries {
		let (directory, stored, original) = paths(app, owner, &id, &entry).await?;
		let stored_exists = tokio::fs::try_exists(&stored).await?;
		let original_exists = tokio::fs::try_exists(&original).await?;
		match entry.state {
			TrashState::Deleting if stored_exists && !original_exists => {
				let deletes: Vec<String> =
					entry.records.iter().map(|r| db::file(owner, &r.path)).collect();
				app.db.write_batch::<FileRecord>(&[], &deletes)?;
				if let Some(item) = personal.trash.get_mut(&id) {
					item.state = TrashState::Ready;
				}
				changed = true;
			}
			TrashState::Deleting if original_exists && !stored_exists => {
				personal.trash.remove(&id);
				changed = true;
			}
			TrashState::Restoring if original_exists && !stored_exists => {
				let puts: Vec<(String, FileRecord)> = entry
					.records
					.into_iter()
					.map(|r| (db::file(owner, &r.path), r))
					.collect();
				app.db.write_batch(&puts, &[])?;
				personal.trash.remove(&id);
				if tokio::fs::try_exists(&directory).await? {
					tokio::fs::remove_dir_all(directory).await?;
				}
				changed = true;
			}
			TrashState::Restoring if stored_exists && !original_exists => {
				if let Some(item) = personal.trash.get_mut(&id) {
					item.state = TrashState::Ready;
				}
				changed = true;
			}
			TrashState::Purging => {
				if tokio::fs::try_exists(&directory).await? {
					tokio::fs::remove_dir_all(directory).await?;
				}
				personal.trash.remove(&id);
				changed = true;
			}
			_ => tracing::warn!(user = owner, item = %id, "trash item needs attention"),
		}
	}
	if changed {
		profile::save(app, owner, &personal).await?;
	}
	Ok(())
}

pub(super) async fn ready_items(app: &App, owner: &str) -> Result<Vec<String>> {
	let personal = profile::load(app, owner).await?;
	Ok(personal
		.trash
		.into_iter()
		.filter_map(|(id, entry)| (entry.state == TrashState::Ready).then_some(id))
		.collect())
}

pub(super) async fn purge_items(app: &App, owner: &str, ids: &[String]) -> Result<()> {
	if ids.is_empty() {
		return Ok(());
	}
	let mut personal = profile::load(app, owner).await?;
	for id in ids {
		if let Some(entry) = personal.trash.get(id) {
			if entry.state != TrashState::Ready {
				return Err(conflict("trash item is being recovered"));
			}
			paths(app, owner, id, entry).await?;
		}
	}
	for id in ids {
		if let Some(entry) = personal.trash.get_mut(id) {
			if entry.state != TrashState::Ready {
				return Err(conflict("trash item is being recovered"));
			}
			entry.state = TrashState::Purging;
		}
	}
	profile::save(app, owner, &personal).await?;
	for id in ids {
		if let Some(entry) = personal.trash.get(id) {
			let (directory, _, _) = paths(app, owner, id, entry).await?;
			if tokio::fs::try_exists(&directory).await? {
				tokio::fs::remove_dir_all(directory).await?;
			}
		}
		personal.trash.remove(id);
	}
	profile::save(app, owner, &personal).await?;
	Ok(())
}

pub(super) async fn expire(app: &App, user: &User, now: u64) -> Result<()> {
	recover_user(app, &user.id).await?;
	let retention = core::duration(user.retention.as_deref().unwrap_or(&app.trash_retention))?;
	let Some(retention) = retention else {
		return Ok(());
	};
	let personal = profile::load(app, &user.id).await?;
	let expired: Vec<String> = personal
		.trash
		.into_iter()
		.filter_map(|(id, entry)| {
			(entry.state == TrashState::Ready && now.saturating_sub(entry.deleted) >= retention)
				.then_some(id)
		})
		.collect();
	purge_items(app, &user.id, &expired).await
}
