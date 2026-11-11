//! User-editable profiles and durable trash information in each .flora file.

use super::App;
use crate::core::{self, FileRecord};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, io::Write, path::Path};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Profile {
	pub version: u32,
	pub name: String,
	#[serde(default)]
	pub settings: Settings,
	#[serde(default)]
	pub trash: BTreeMap<String, TrashEntry>,
	#[serde(flatten)]
	pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Settings {
	pub retention: Option<String>,
	#[serde(flatten)]
	pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct TrashEntry {
	pub path: String,
	pub deleted: u64,
	pub size: u64,
	pub records: Vec<FileRecord>,
	pub state: TrashState,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum TrashState {
	Deleting,
	Ready,
	Restoring,
	Purging,
}

#[derive(Serialize, Deserialize)]
struct PendingWrite {
	owner: String,
	contents: String,
}

impl Profile {
	pub(super) fn new(name: String) -> Self {
		Self {
			version: 1,
			name,
			settings: Settings::default(),
			trash: BTreeMap::new(),
			extra: BTreeMap::new(),
		}
	}
}

pub(super) fn load_sync(app: &App, owner: &str) -> Result<Profile> {
	let key = format!("profile-write/{owner}");
	if let Some(pending) = app.db.get::<PendingWrite>(&key)? {
		if pending.owner != owner {
			bail!("invalid pending profile owner");
		}
		write_contents_sync(app, &pending)?;
		app.db.delete(&key)?;
	}
	read(&app.storage.join(owner).join(".flora"))
}

pub(super) async fn load(app: &App, owner: &str) -> Result<Profile> {
	let app = app.clone();
	let owner = owner.to_owned();
	tokio::task::spawn_blocking(move || load_sync(&app, &owner)).await?
}

pub(super) fn read(path: &Path) -> Result<Profile> {
	let profile: Profile = serde_json::from_slice(&std::fs::read(path)?)
		.with_context(|| format!("invalid profile {}", path.display()))?;
	if profile.version != 1 {
		bail!("unsupported profile version in {}", path.display());
	}
	if let Some(retention) = &profile.settings.retention {
		core::duration(retention)?;
	}
	Ok(profile)
}

pub(super) async fn save(app: &App, owner: &str, profile: &Profile) -> Result<()> {
	let pending =
		PendingWrite { owner: owner.to_owned(), contents: serde_json::to_string_pretty(profile)? };
	let app = app.clone();
	tokio::task::spawn_blocking(move || save_pending_sync(&app, &pending)).await?
}

fn save_pending_sync(app: &App, pending: &PendingWrite) -> Result<()> {
	let key = format!("profile-write/{}", pending.owner);
	app.db.put(&key, pending)?;
	write_contents_sync(app, pending)?;
	app.db.delete(&key)?;
	Ok(())
}

pub(super) fn recover_writes(app: &App) -> Result<()> {
	let pending: Vec<PendingWrite> = app.db.list("profile-write/")?;
	for item in pending {
		uuid::Uuid::parse_str(&item.owner).context("invalid pending profile owner")?;
		write_contents_sync(app, &item)?;
		app.db.delete(&format!("profile-write/{}", item.owner))?;
	}
	Ok(())
}

fn write_contents_sync(app: &App, pending: &PendingWrite) -> Result<()> {
	let root = app.storage.join(&pending.owner);
	let path = core::safe_join(&root, ".flora")?;
	let mut file =
		std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
	file.write_all(pending.contents.as_bytes())?;
	file.sync_all()?;
	Ok(())
}
