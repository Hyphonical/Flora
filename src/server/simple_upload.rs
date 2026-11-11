//! Stream a single-request upload through the shared chunk buffer.

use super::{App, Result, auth, bad, uploads};
use crate::core::{CompleteRequest, ConflictPolicy, FileRecord, UploadRequest};
use axum::{
	Json,
	body::Body,
	extract::{Query, State},
	http::{HeaderMap, header},
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::Deserialize;
use xxhash_rust::xxh3::Xxh3;

#[derive(Deserialize)]
pub(super) struct UploadQuery {
	path: String,
	owner: Option<String>,
	#[serde(default)]
	conflict: ConflictPolicy,
}

pub(super) async fn upload(
	State(app): State<App>,
	Query(query): Query<UploadQuery>,
	headers: HeaderMap,
	body: Body,
) -> Result<Json<FileRecord>> {
	let principal = auth::authenticate(&app, &headers)?;
	let size = headers
		.get(header::CONTENT_LENGTH)
		.map(|value| {
			value
				.to_str()
				.ok()
				.and_then(|value| value.parse::<u64>().ok())
				.ok_or_else(|| bad("invalid Content-Length"))
		})
		.transpose()?;
	let expected_hash = headers
		.get("x-xxh3-128")
		.map(|value| value.to_str().map(str::to_owned).map_err(|_| bad("invalid XXH3-128 hash")))
		.transpose()?;
	let reservation = uploads::begin_upload(
		&app,
		&principal,
		UploadRequest { path: query.path, owner: query.owner, size, conflict: query.conflict },
		false,
	)
	.await?;
	let upload_id = reservation.id.clone();
	let chunk_size =
		usize::try_from(reservation.chunk_size).map_err(|_| bad("chunk size is too large"))?;

	let outcome: Result<FileRecord> = async {
		let mut buffer = Vec::with_capacity(chunk_size);
		let mut stream = body.into_data_stream();
		let mut hash = Xxh3::new();
		let mut total = 0u64;
		let mut number = 0usize;
		while let Some(data) = stream.next().await {
			let data = data.map_err(|error| bad(format!("reading upload: {error}")))?;
			total = total
				.checked_add(data.len() as u64)
				.ok_or_else(|| bad("upload too large"))?;
			if total > app.buffer_size {
				return Err(bad(format!(
					"file is larger than buffer (FLORA_BUFFER_SIZE): {} bytes",
					app.buffer_size
				)));
			}
			if size.is_some_and(|expected| total > expected) {
				return Err(bad("upload exceeds Content-Length"));
			}
			hash.update(&data);
			let mut remaining = data.as_ref();
			while !remaining.is_empty() {
				let count = remaining.len().min(chunk_size - buffer.len());
				buffer.extend_from_slice(&remaining[..count]);
				remaining = &remaining[count..];
				if buffer.len() == chunk_size {
					let payload = Bytes::from(std::mem::replace(
						&mut buffer,
						Vec::with_capacity(chunk_size),
					));
					uploads::write_chunk_bytes(&app, &principal, &upload_id, number, payload)
						.await?;
					number += 1;
				}
			}
		}
		if size.is_some_and(|expected| total != expected) {
			return Err(bad("upload is shorter than Content-Length"));
		}
		if !buffer.is_empty() {
			uploads::write_chunk_bytes(&app, &principal, &upload_id, number, Bytes::from(buffer))
				.await?;
		}
		let digest = format!("{:032x}", hash.digest128());
		if expected_hash.is_some_and(|expected| !expected.eq_ignore_ascii_case(&digest)) {
			return Err(bad("XXH3-128 checksum mismatch"));
		}
		uploads::complete_upload(
			&app,
			&principal,
			&upload_id,
			CompleteRequest { hash: digest, size: total },
		)
		.await
	}
	.await;

	match outcome {
		Ok(record) => Ok(Json(record)),
		Err(error) => {
			let _ = uploads::cancel_upload(&app, &principal, &upload_id).await;
			Err(error)
		}
	}
}
