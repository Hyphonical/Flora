//! Fjall-backed typed metadata records and batched durable mutations.

use anyhow::Result;
use fjall::{Database, Keyspace, KeyspaceCreateOptions, PersistMode};
use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;

#[derive(Clone)]
pub(crate) struct Store {
	db: Database,
	records: Keyspace,
}

impl Store {
	pub(crate) fn open(path: &Path) -> Result<Self> {
		let db = Database::builder(path).open()?;
		let records = db.keyspace("records", KeyspaceCreateOptions::default)?;
		Ok(Self { db, records })
	}

	pub(crate) fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
		self.records
			.get(key)?
			.map(|value| serde_json::from_slice(&value).map_err(Into::into))
			.transpose()
	}

	pub(crate) fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
		self.records.insert(key, serde_json::to_vec(value)?)?;
		self.db.persist(PersistMode::SyncAll)?;
		Ok(())
	}

	pub(crate) fn put_buffered<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
		self.records.insert(key, serde_json::to_vec(value)?)?;
		Ok(())
	}

	pub(crate) fn delete(&self, key: &str) -> Result<()> {
		self.records.remove(key)?;
		self.db.persist(PersistMode::SyncAll)?;
		Ok(())
	}

	pub(crate) fn replace<T: Serialize>(&self, remove: &str, key: &str, value: &T) -> Result<()> {
		let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
		batch.insert(&self.records, key, serde_json::to_vec(value)?);
		batch.remove(&self.records, remove);
		batch.commit()?;
		Ok(())
	}

	pub(crate) fn write_batch<T: Serialize>(
		&self,
		puts: &[(String, T)],
		deletes: &[String],
	) -> Result<()> {
		if puts.is_empty() && deletes.is_empty() {
			return Ok(());
		}
		let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
		for key in deletes {
			batch.remove(&self.records, key.as_str());
		}
		for (key, value) in puts {
			batch.insert(&self.records, key.as_str(), serde_json::to_vec(value)?);
		}
		batch.commit()?;
		Ok(())
	}

	pub(crate) fn list<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<T>> {
		self.records
			.prefix(prefix)
			.map(|guard| {
				let value = guard.value()?;
				Ok(serde_json::from_slice(&value)?)
			})
			.collect()
	}

	pub(crate) fn list_rev_limit<T: DeserializeOwned>(
		&self,
		prefix: &str,
		limit: usize,
	) -> Result<Vec<T>> {
		self.records
			.prefix(prefix)
			.rev()
			.take(limit)
			.map(|guard| {
				let value = guard.value()?;
				Ok(serde_json::from_slice(&value)?)
			})
			.collect()
	}
}

pub(super) fn user(id: &str) -> String {
	format!("user/{id}")
}

pub(super) fn token(hash: &str) -> String {
	format!("token/{hash}")
}

pub(super) fn upload(id: &str) -> String {
	format!("upload/{id}")
}

pub(super) fn file(owner: &str, path: &str) -> String {
	format!("file/{owner}/{path}")
}

pub(super) fn files(owner: &str) -> String {
	format!("file/{owner}/")
}

pub(super) fn files_under(owner: &str, prefix: &str) -> String {
	if prefix.is_empty() {
		format!("file/{owner}/")
	} else {
		format!("file/{owner}/{prefix}/")
	}
}
