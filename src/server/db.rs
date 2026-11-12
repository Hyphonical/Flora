//! Fjall-backed typed metadata records, secondary indexes, and non-blocking durable mutations.

use crate::core::{FileRecord, Token, TokenKind, User};
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

	pub(crate) fn ensure_indexes(&self) -> Result<()> {
		let users: Vec<User> = self.list("user/")?;
		let tokens: Vec<Token> = self.list("token/")?;
		let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
		let mut wrote = false;
		for u in &users {
			let idx = user_name(&u.name);
			if self.get::<String>(&idx)?.as_deref() != Some(u.id.as_str()) {
				batch.insert(&self.records, idx, serde_json::to_vec(&u.id)?);
				wrote = true;
			}
		}
		for t in &tokens {
			let idx = user_token(&t.owner, &t.hash);
			if self.records.get(&idx)?.is_none() {
				batch.insert(&self.records, idx, b"true".as_slice());
				wrote = true;
			}
		}
		if wrote {
			batch.commit()?;
		}
		Ok(())
	}

	pub(crate) fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
		self.records
			.get(key)?
			.map(|value| serde_json::from_slice(&value).map_err(Into::into))
			.transpose()
	}

	pub(crate) async fn get_async<T: DeserializeOwned + Send + 'static>(
		&self,
		key: &str,
	) -> Result<Option<T>> {
		let store = self.clone();
		let key = key.to_owned();
		tokio::task::spawn_blocking(move || store.get(&key)).await?
	}

	pub(crate) fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
		self.records.insert(key, serde_json::to_vec(value)?)?;
		self.db.persist(PersistMode::SyncAll)?;
		Ok(())
	}

	pub(crate) async fn put_async<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
		let store = self.clone();
		let key = key.to_owned();
		let bytes = serde_json::to_vec(value)?;
		tokio::task::spawn_blocking(move || -> Result<()> {
			store.records.insert(key, bytes)?;
			store.db.persist(PersistMode::SyncAll)?;
			Ok(())
		})
		.await?
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

	pub(crate) async fn delete_async(&self, key: &str) -> Result<()> {
		let store = self.clone();
		let key = key.to_owned();
		tokio::task::spawn_blocking(move || store.delete(&key)).await?
	}

	pub(crate) async fn replace_async<T: Serialize>(
		&self,
		remove: &str,
		key: &str,
		value: &T,
	) -> Result<()> {
		let store = self.clone();
		let remove = remove.to_owned();
		let key = key.to_owned();
		let bytes = serde_json::to_vec(value)?;
		tokio::task::spawn_blocking(move || -> Result<()> {
			let mut batch = store.db.batch().durability(Some(PersistMode::SyncAll));
			batch.insert(&store.records, key, bytes);
			batch.remove(&store.records, remove);
			batch.commit()?;
			Ok(())
		})
		.await?
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

	pub(crate) async fn write_batch_async<T: Serialize>(
		&self,
		puts: &[(String, T)],
		deletes: &[String],
	) -> Result<()> {
		if puts.is_empty() && deletes.is_empty() {
			return Ok(());
		}
		let serialized: Vec<(String, Vec<u8>)> = puts
			.iter()
			.map(|(k, v)| Ok((k.clone(), serde_json::to_vec(v)?)))
			.collect::<Result<_>>()?;
		let deletes = deletes.to_vec();
		let store = self.clone();
		tokio::task::spawn_blocking(move || -> Result<()> {
			let mut batch = store.db.batch().durability(Some(PersistMode::SyncAll));
			for key in &deletes {
				batch.remove(&store.records, key.as_str());
			}
			for (key, bytes) in &serialized {
				batch.insert(&store.records, key.as_str(), bytes.as_slice());
			}
			batch.commit()?;
			Ok(())
		})
		.await?
	}

	pub(crate) fn put_user_sync(&self, old_name: Option<&str>, u: &User) -> Result<()> {
		let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
		if let Some(prev) = old_name
			&& !prev.eq_ignore_ascii_case(&u.name)
		{
			batch.remove(&self.records, user_name(prev));
		}
		batch.insert(&self.records, user(&u.id), serde_json::to_vec(u)?);
		batch.insert(&self.records, user_name(&u.name), serde_json::to_vec(&u.id)?);
		batch.commit()?;
		Ok(())
	}

	pub(crate) async fn put_user_async(&self, old_name: Option<&str>, u: &User) -> Result<()> {
		let store = self.clone();
		let old_name = old_name.map(str::to_owned);
		let u = u.clone();
		tokio::task::spawn_blocking(move || store.put_user_sync(old_name.as_deref(), &u)).await?
	}

	pub(crate) async fn find_user_by_name_async(&self, name: &str) -> Result<Option<User>> {
		let store = self.clone();
		let idx = user_name(name);
		tokio::task::spawn_blocking(move || -> Result<Option<User>> {
			let Some(id) = store.get::<String>(&idx)? else {
				return Ok(None);
			};
			store.get(&user(&id))
		})
		.await?
	}

	pub(crate) async fn put_token_async(&self, record: &Token) -> Result<()> {
		let store = self.clone();
		let key = token(&record.hash);
		let idx = user_token(&record.owner, &record.hash);
		let bytes = serde_json::to_vec(record)?;
		tokio::task::spawn_blocking(move || -> Result<()> {
			let mut batch = store.db.batch().durability(Some(PersistMode::SyncAll));
			batch.insert(&store.records, key, bytes);
			batch.insert(&store.records, idx, b"true".as_slice());
			batch.commit()?;
			Ok(())
		})
		.await?
	}

	pub(crate) async fn delete_token_async(&self, owner: &str, hash: &str) -> Result<()> {
		let store = self.clone();
		let key = token(hash);
		let idx = user_token(owner, hash);
		tokio::task::spawn_blocking(move || -> Result<()> {
			let mut batch = store.db.batch().durability(Some(PersistMode::SyncAll));
			batch.remove(&store.records, key);
			batch.remove(&store.records, idx);
			batch.commit()?;
			Ok(())
		})
		.await?
	}

	pub(crate) async fn list_user_tokens_async(&self, owner: &str) -> Result<Vec<Token>> {
		let store = self.clone();
		let prefix = format!("user_token/{owner}/");
		tokio::task::spawn_blocking(move || -> Result<Vec<Token>> {
			let mut out = Vec::new();
			for guard in store.records.prefix(&prefix) {
				let key_bytes = guard.key()?;
				let Ok(key_str) = std::str::from_utf8(&key_bytes) else {
					continue;
				};
				let Some(hash) = key_str.strip_prefix(&prefix) else {
					continue;
				};
				if let Some(tok) = store.get::<Token>(&token(hash))? {
					out.push(tok);
				}
			}
			Ok(out)
		})
		.await?
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

	pub(crate) async fn list_async<T: DeserializeOwned + Send + 'static>(
		&self,
		prefix: &str,
	) -> Result<Vec<T>> {
		let store = self.clone();
		let prefix = prefix.to_owned();
		tokio::task::spawn_blocking(move || store.list(&prefix)).await?
	}

	pub(crate) async fn list_rev_limit_async<T: DeserializeOwned + Send + 'static>(
		&self,
		prefix: &str,
		limit: usize,
	) -> Result<Vec<T>> {
		let store = self.clone();
		let prefix = prefix.to_owned();
		tokio::task::spawn_blocking(move || -> Result<Vec<T>> {
			store
				.records
				.prefix(&prefix)
				.rev()
				.take(limit)
				.map(|guard| {
					let value = guard.value()?;
					Ok(serde_json::from_slice(&value)?)
				})
				.collect()
		})
		.await?
	}

	pub(crate) async fn prune_prefix_keep_newest_async(
		&self,
		prefix: &str,
		keep: usize,
	) -> Result<()> {
		let store = self.clone();
		let prefix = prefix.to_owned();
		tokio::task::spawn_blocking(move || -> Result<()> {
			let mut stale = Vec::new();
			for guard in store.records.prefix(&prefix).rev().skip(keep) {
				stale.push(guard.key()?);
			}
			if stale.is_empty() {
				return Ok(());
			}
			let mut batch = store.db.batch();
			for key in stale {
				batch.remove(&store.records, key);
			}
			batch.commit()?;
			Ok(())
		})
		.await?
	}

	pub(crate) async fn sum_file_sizes_async(&self, owner: &str) -> Result<u64> {
		let store = self.clone();
		let prefix = files(owner);
		tokio::task::spawn_blocking(move || -> Result<u64> {
			let mut sum = 0u64;
			for guard in store.records.prefix(&prefix) {
				let value = guard.value()?;
				let record: FileRecord = serde_json::from_slice(&value)?;
				sum = sum.saturating_add(record.size);
			}
			Ok(sum)
		})
		.await?
	}

	pub(crate) fn share_map_sync(
		&self,
		owner: &str,
	) -> Result<std::collections::HashMap<String, String>> {
		let prefix = format!("user_token/{owner}/");
		let mut map = std::collections::HashMap::new();
		for guard in self.records.prefix(&prefix) {
			let key_bytes = guard.key()?;
			let Ok(key_str) = std::str::from_utf8(&key_bytes) else {
				continue;
			};
			let Some(hash) = key_str.strip_prefix(&prefix) else {
				continue;
			};
			if let Some(tok) = self.get::<Token>(&token(hash))?
				&& matches!(tok.kind, TokenKind::File | TokenKind::Folder)
				&& let Some(secret) = tok.secret
			{
				map.insert(tok.scope, secret);
			}
		}
		Ok(map)
	}
}

pub(super) fn user(id: &str) -> String {
	format!("user/{id}")
}

pub(super) fn user_name(name: &str) -> String {
	format!("user_name/{}", name.to_ascii_lowercase())
}

pub(super) fn token(hash: &str) -> String {
	format!("token/{hash}")
}

pub(super) fn user_token(owner: &str, hash: &str) -> String {
	format!("user_token/{owner}/{hash}")
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
	if prefix.is_empty() { format!("file/{owner}/") } else { format!("file/{owner}/{prefix}/") }
}
