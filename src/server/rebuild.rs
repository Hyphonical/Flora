//! Rebuild users and reconcile file metadata with local storage on startup.

use super::{App, db, profile};
use crate::core::{self, FileRecord, User};
use anyhow::{Context, Result};
use std::{collections::HashMap, path::Path};
use uuid::Uuid;
pub(super) fn rebuild(app: &App) -> Result<()> {
	let existing: Vec<User> = app.db.list("user/")?;
	let rebuilding = app.db.get::<bool>("system/rebuilding")?.unwrap_or(false);
	if !existing.is_empty() && !rebuilding {
		return Ok(());
	}
	app.db.put("system/rebuilding", &true)?;
	for entry in std::fs::read_dir(&app.storage)? {
		let entry = entry?;
		if !entry.file_type()?.is_dir() {
			continue;
		}
		let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
			continue;
		};
		if Uuid::parse_str(&id).is_err() {
			continue;
		}
		let profile_path = entry.path().join(".flora");
		if !profile_path.exists() {
			continue;
		}
		let profile = profile::read(&profile_path)?;
		let user = User {
			id: id.clone(),
			name: profile.name,
			password_hash: String::new(),
			admin: false,
			quota: None,
			retention: profile.settings.retention,
		};
		app.db.put_user_sync(None, &user)?;
		reconcile_user(app, &id)?;
	}
	app.db.delete("system/rebuilding")?;
	Ok(())
}

pub(super) fn sync_profiles(app: &App) -> Result<()> {
	let users: Vec<User> = app.db.list("user/")?;
	for mut user in users {
		let profile = profile::load_sync(app, &user.id)?;
		if user.name != profile.name || user.retention != profile.settings.retention {
			let old_name = user.name.clone();
			user.name = profile.name;
			user.retention = profile.settings.retention;
			app.db.put_user_sync(Some(&old_name), &user)?;
		}
	}
	Ok(())
}

pub(super) fn reconcile(app: &App) -> Result<()> {
	let users: Vec<User> = app.db.list("user/")?;
	for user in users {
		reconcile_user(app, &user.id)?;
	}
	Ok(())
}

fn reconcile_user(app: &App, owner: &str) -> Result<()> {
	let root = app.storage.join(owner);
	let mut existing: HashMap<String, FileRecord> = app
		.db
		.list::<FileRecord>(&db::files(owner))?
		.into_iter()
		.map(|record| (record.path.clone(), record))
		.collect();
	let mut puts = Vec::new();
	for item in core::walk::walk(&root, |item| {
		let name = item.file_name();
		name != ".trash" && name != ".flora" && name != ".flora.tmp"
	})? {
		let item = item?;
		if !item.file_type()?.is_file() {
			continue;
		}
		let item_path = item.path();
		let relative = item_path
			.strip_prefix(&root)?
			.to_str()
			.context("storage path is not UTF-8")?
			.replace('\\', "/");
		let path = core::clean_path(&relative, false)?;
		let metadata = item.metadata()?;
		let modified = metadata
			.modified()
			.ok()
			.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
			.map_or(0, |d| d.as_secs());
		let previous = existing.remove(&path);
		if previous
			.as_ref()
			.is_some_and(|record| record.size == metadata.len() && record.modified == modified)
		{
			continue;
		}
		let record = make_record(owner, &path, &item_path, metadata.len(), modified)?;
		puts.push((db::file(owner, &path), record));
	}
	let deletes: Vec<String> = existing.keys().map(|path| db::file(owner, path)).collect();
	app.db.write_batch(&puts, &deletes)?;
	Ok(())
}

fn make_record(
	owner: &str,
	path: &str,
	source: &Path,
	size: u64,
	modified: u64,
) -> Result<FileRecord> {
	Ok(FileRecord {
		owner: owner.to_owned(),
		path: path.to_owned(),
		size,
		hash: core::hex::hash_file_sync(source)?,
		modified,
	})
}
