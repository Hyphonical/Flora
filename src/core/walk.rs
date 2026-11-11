//! Iterative directory traversal that never descends through symbolic links.

use std::{
	fs::{self, DirEntry, ReadDir},
	io,
	path::Path,
};

pub(crate) struct Walk<F> {
	dirs: Vec<ReadDir>,
	include: F,
}

pub(crate) fn walk<F>(root: &Path, include: F) -> io::Result<Walk<F>>
where
	F: FnMut(&DirEntry) -> bool,
{
	if !fs::symlink_metadata(root)?.file_type().is_dir() {
		return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			"traversal root is not a directory",
		));
	}
	Ok(Walk { dirs: vec![fs::read_dir(root)?], include })
}

impl<F: FnMut(&DirEntry) -> bool> Iterator for Walk<F> {
	type Item = io::Result<DirEntry>;

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			let dir = self.dirs.last_mut()?;
			let entry = match dir.next() {
				None => {
					self.dirs.pop();
					continue;
				}
				Some(Err(error)) => return Some(Err(error)),
				Some(Ok(entry)) => entry,
			};
			if !(self.include)(&entry) {
				continue;
			}
			let kind = match entry.file_type() {
				Ok(kind) => kind,
				Err(error) => return Some(Err(error)),
			};
			if kind.is_symlink() {
				continue;
			}
			if kind.is_dir() {
				match fs::read_dir(entry.path()) {
					Ok(dir) => self.dirs.push(dir),
					Err(error) => return Some(Err(error)),
				}
			}
			return Some(Ok(entry));
		}
	}
}
