//! Stream a single-request upload directly to storage staging without intermediate chunk files.

use super::{App, Result, auth, bad, uploads};
use crate::core::{self, ConflictPolicy, FileRecord, UploadRequest};
use axum::{
	Json,
	body::Body,
	extract::{Query, State},
	http::{HeaderMap, header},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
	let principal = auth::authenticate(&app, &headers).await?;
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
		.map(|value| -> Result<String> {
			let s = value.to_str().map_err(|_| bad("invalid XXH3-128 hash"))?;
			if !core::hex::is_hex_128(s) {
				return Err(bad("invalid XXH3-128 hash"));
			}
			Ok(s.to_owned())
		})
		.transpose()?;
	let mut reservation = uploads::begin_upload(
		&app,
		&principal,
		UploadRequest { path: query.path, owner: query.owner, size, conflict: query.conflict },
		false,
	)
	.await?;
	let upload_id = reservation.id.clone();
	match uploads::stream_direct_upload(
		&app,
		&mut reservation,
		size,
		expected_hash.as_deref(),
		body,
	)
	.await
	{
		Ok(record) => Ok(Json(record)),
		Err(error) => {
			let _ = uploads::cancel_upload(&app, &principal, &upload_id).await;
			Err(error)
		}
	}
}
