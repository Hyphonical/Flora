//! Coordinate each upload independently while bounding server-wide chunk work.

use std::{
	collections::HashMap,
	sync::{Arc, Mutex, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, RwLock};

#[derive(Default)]
pub(crate) struct UploadLocks {
	pub gate: RwLock<()>,
	pub cancel: tokio_util::sync::CancellationToken,
	chunks: Mutex<HashMap<usize, Weak<AsyncMutex<()>>>>,
}

impl UploadLocks {
	pub(crate) fn chunk(&self, number: usize) -> Arc<AsyncMutex<()>> {
		let mut chunks = self.chunks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
		chunks.retain(|_, lock| lock.strong_count() != 0);
		if let Some(lock) = chunks.get(&number).and_then(Weak::upgrade) {
			return lock;
		}
		let lock = Arc::new(AsyncMutex::new(()));
		chunks.insert(number, Arc::downgrade(&lock));
		lock
	}
}

#[derive(Default)]
pub(crate) struct Registry(Mutex<HashMap<String, Weak<UploadLocks>>>);

impl Registry {
	pub(crate) fn get(&self, id: &str) -> Arc<UploadLocks> {
		let mut uploads = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
		uploads.retain(|_, lock| lock.strong_count() != 0);
		if let Some(lock) = uploads.get(id).and_then(Weak::upgrade) {
			return lock;
		}
		let lock = Arc::new(UploadLocks::default());
		uploads.insert(id.to_owned(), Arc::downgrade(&lock));
		lock
	}
}
