//! Per-upload and per-user concurrency coordination.

use std::{
	collections::HashMap,
	sync::{Arc, Mutex, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, RwLock};

#[derive(Default)]
pub(crate) struct UploadLocks {
	pub gate: RwLock<()>,
	pub meta: AsyncMutex<()>,
	pub cancel: tokio_util::sync::CancellationToken,
	chunks: Mutex<HashMap<usize, Arc<AsyncMutex<()>>>>,
}

impl UploadLocks {
	pub(crate) fn chunk(&self, number: usize) -> Arc<AsyncMutex<()>> {
		let mut chunks = self.chunks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
		Arc::clone(chunks.entry(number).or_insert_with(|| Arc::new(AsyncMutex::new(()))))
	}
}

#[derive(Default)]
pub(crate) struct Registry(Mutex<HashMap<String, Arc<UploadLocks>>>);

impl Registry {
	pub(crate) fn get(&self, id: &str) -> Arc<UploadLocks> {
		let mut uploads = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
		Arc::clone(uploads.entry(id.to_owned()).or_insert_with(|| Arc::new(UploadLocks::default())))
	}

	pub(crate) fn remove(&self, id: &str) {
		let mut uploads = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
		uploads.remove(id);
	}
}

#[derive(Default)]
pub(crate) struct UserLocks(Mutex<HashMap<String, Weak<AsyncMutex<()>>>>);

impl UserLocks {
	pub(crate) async fn lock(&self, owner: &str) -> OwnedMutexGuard<()> {
		let mutex = {
			let mut map = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
			if map.len() >= 256 {
				map.retain(|_, entry| entry.strong_count() != 0);
			}
			if let Some(existing) = map.get(owner).and_then(Weak::upgrade) {
				existing
			} else {
				let created = Arc::new(AsyncMutex::new(()));
				map.insert(owner.to_owned(), Arc::downgrade(&created));
				created
			}
		};
		mutex.lock_owned().await
	}
}
