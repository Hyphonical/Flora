//! Search filenames and UTF-8 file contents on demand without storing text.

use super::{App, Result, auth, bad, db, files};
use crate::core::{self, FileRecord};
use axum::{
	Json,
	extract::{Query, State},
	http::HeaderMap,
};
use serde::Deserialize;
use std::path::Path;
use tokio::io::AsyncReadExt;

const MAX_SEARCH_RESULTS: usize = 500;
const MAX_TEXT_SCAN_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Deserialize)]
pub(super) struct Search {
	q: String,
	owner: Option<String>,
}

pub(super) async fn search(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<Search>,
) -> Result<Json<Vec<FileRecord>>> {
	let principal = auth::authenticate(&app, &headers)?;
	let owner = files::owner(&app, &principal, query.owner.as_deref())?;
	let term: String = query.q.chars().flat_map(char::to_lowercase).collect();
	if term.is_empty() || term.len() > 1024 {
		return Err(bad("search term must be between 1 and 1024 bytes"));
	}
	let records: Vec<FileRecord> = app.db.list(&db::files(&owner))?;
	let mut matches = Vec::new();
	for record in records {
		if matches.len() >= MAX_SEARCH_RESULTS {
			break;
		}
		if !files::allowed(&principal, &owner, &record.path, false) {
			continue;
		}
		let name: String = record.path.chars().flat_map(char::to_lowercase).collect();
		let matched = if name.contains(&term) {
			true
		} else if record.size <= MAX_TEXT_SCAN_BYTES {
			let path = core::safe_join_async(&app.storage.join(&owner), &record.path)
				.await
				.map_err(|error| bad(error.to_string()))?;
			contains_text(&path, &term).await?
		} else {
			false
		};
		if matched {
			matches.push(record);
		}
	}
	Ok(Json(matches))
}

async fn contains_text(path: &Path, term: &str) -> Result<bool> {
	let mut file = match tokio::fs::File::open(path).await {
		Ok(file) => file,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
		Err(error) => return Err(error.into()),
	};
	let mut bytes = [0u8; 64 * 1024 + 3];
	let mut remaining = 0usize;
	let mut window = String::new();
	let mut found = false;
	loop {
		let read = file.read(&mut bytes[remaining..]).await?;
		let length = remaining + read;
		if length == 0 {
			break;
		}
		let valid = match std::str::from_utf8(&bytes[..length]) {
			Ok(_) => length,
			Err(error) if error.error_len().is_none() && read != 0 => error.valid_up_to(),
			Err(_) => return Ok(false),
		};
		let text = std::str::from_utf8(&bytes[..valid]).map_err(anyhow::Error::from)?;
		if !found {
			window.extend(text.chars().flat_map(char::to_lowercase));
			found = window.contains(term);
			let mut start = window.len().saturating_sub(term.len().saturating_sub(1));
			while !window.is_char_boundary(start) {
				start -= 1;
			}
			window.drain(..start);
		}
		remaining = length - valid;
		bytes.copy_within(valid..length, 0);
		if read == 0 {
			break;
		}
	}
	Ok(found && remaining == 0)
}
