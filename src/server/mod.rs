//! Server startup, shared state, HTTP routing, and error responses.

mod auth;
mod db;
mod files;
mod install;
mod ops;
mod profile;
mod rebuild;
mod search;
mod simple_upload;
mod transfer_locks;
mod trash;
mod uploads;
mod volume;

use crate::core::{self, config::ServeConfig};
use anyhow::bail;
use axum::{
	Json, Router,
	extract::DefaultBodyLimit,
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::{get, post, put},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct App {
	pub db: db::Store,
	pub data: PathBuf,
	pub buffer: PathBuf,
	pub storage: PathBuf,
	pub buffer_size: u64,
	pub chunk_size: u64,
	pub disk_limit: Option<u64>,
	pub buffer_retention: Option<u64>,
	pub trash_retention: String,
	pub public_url: Option<String>,
	pub rate_limit: u32,
	pub behind_proxy: bool,
	pub login_rate_limit: u32,
	pub trusted_proxies: Vec<ipnet::IpNet>,
	pub limits: Arc<Mutex<ops::Limits>>,
	pub setup_code: Arc<Mutex<Option<String>>>,
	pub transfer_locks: Arc<transfer_locks::Registry>,
	pub user_locks: Arc<transfer_locks::UserLocks>,
	pub accounts: Arc<Mutex<()>>,
	pub reservation: Arc<Mutex<()>>,
	pub chunk_permits: Arc<tokio::sync::Semaphore>,
	pub thumb_permits: Arc<tokio::sync::Semaphore>,
}

pub(crate) struct Error(pub StatusCode, pub anyhow::Error);
type Result<T, E = Error> = std::result::Result<T, E>;

macro_rules! internal_errors {
	($($kind:ty),+ $(,)?) => {
		$(impl From<$kind> for Error {
			fn from(error: $kind) -> Self {
				Self(StatusCode::INTERNAL_SERVER_ERROR, error.into())
			}
		})+
	};
}

internal_errors!(
	anyhow::Error,
	std::io::Error,
	serde_json::Error,
	tokio::task::JoinError,
	axum::http::Error
);

impl IntoResponse for Error {
	fn into_response(self) -> Response {
		let message = if self.0 == StatusCode::INTERNAL_SERVER_ERROR {
			tracing::error!(error = %self.1, "server request failed");
			"Internal server error".to_owned()
		} else {
			self.1.to_string()
		};
		(self.0, Json(json!({"error": message}))).into_response()
	}
}

fn bad(message: impl std::fmt::Display) -> Error {
	Error(StatusCode::BAD_REQUEST, anyhow::anyhow!("{message}"))
}

fn forbidden() -> Error {
	Error(StatusCode::FORBIDDEN, anyhow::anyhow!("permission denied"))
}

fn missing() -> Error {
	Error(StatusCode::NOT_FOUND, anyhow::anyhow!("not found"))
}

fn conflict(message: impl std::fmt::Display) -> Error {
	Error(StatusCode::CONFLICT, anyhow::anyhow!("{message}"))
}

pub(crate) async fn run(config: &ServeConfig) -> anyhow::Result<()> {
	for path in [&config.data, &config.buffer, &config.storage] {
		std::fs::create_dir_all(path)?;
	}
	let data = std::fs::canonicalize(&config.data)?;
	let buffer = std::fs::canonicalize(&config.buffer)?;
	let storage = std::fs::canonicalize(&config.storage)?;
	std::fs::create_dir_all(storage.join(".staging"))?;
	if data.starts_with(&buffer)
		|| data.starts_with(&storage)
		|| buffer.starts_with(&data)
		|| buffer.starts_with(&storage)
		|| storage.starts_with(&data)
		|| storage.starts_with(&buffer)
	{
		bail!("data, buffer and storage must be separate directories");
	}
	let capacity = fs2::total_space(&buffer)?;
	let buffer_size = config.buffer_size.resolve(capacity)?;
	let disk_limit = config.disk_limit.map(|limit| limit.resolve(capacity)).transpose()?;
	if disk_limit.is_some()
		&& (!volume::same_volume(&buffer, &data)? || !volume::same_volume(&buffer, &storage)?)
	{
		bail!(
			"data, buffer, and storage must be on the same filesystem when a disk limit is configured"
		);
	}
	let chunk_size = config.chunk_size;
	if chunk_size == 0 || buffer_size == 0 || chunk_size > buffer_size {
		bail!("chunk size must be positive and no larger than buffer size");
	}
	if disk_limit.is_some_and(|limit| buffer_size > limit) {
		bail!("buffer size exceeds disk limit");
	}
	let db = db::Store::open(&data.join("fjall"))?;
	let users: Vec<core::User> = db.list("user/")?;
	let setup_code = if users.iter().any(|u| u.admin) { None } else { Some(auth::random_secret()) };
	if let Some(code) = &setup_code {
		tracing::info!(setup_code = %code, "first-run setup code generated");
	}
	let app = App {
		db,
		data,
		buffer,
		storage,
		buffer_size,
		chunk_size,
		disk_limit,
		buffer_retention: config.buffer_retention,
		trash_retention: config.trash_retention.clone(),
		public_url: config.public_url.clone(),
		rate_limit: config.rate_limit,
		login_rate_limit: config.login_rate_limit,
		behind_proxy: config.behind_proxy,
		trusted_proxies: config.trusted_proxies.clone(),
		limits: Arc::new(Mutex::new(ops::Limits::default())),
		transfer_locks: Arc::new(transfer_locks::Registry::default()),
		user_locks: Arc::new(transfer_locks::UserLocks::default()),
		accounts: Arc::new(Mutex::new(())),
		reservation: Arc::new(Mutex::new(())),
		chunk_permits: Arc::new(tokio::sync::Semaphore::new(config.upload_concurrency)),
		thumb_permits: Arc::new(tokio::sync::Semaphore::new(4)),
		setup_code: Arc::new(Mutex::new(setup_code)),
	};
	tokio::task::spawn_blocking({
		let app = app.clone();
		move || -> anyhow::Result<()> {
			profile::recover_writes(&app)?;
			rebuild::rebuild(&app)?;
			rebuild::sync_profiles(&app)?;
			app.db.ensure_indexes()?;
			Ok(())
		}
	})
	.await??;
	trash::recover(&app).await.map_err(|error| error.1)?;
	uploads::recover(&app).await?;
	tokio::task::spawn_blocking({
		let app = app.clone();
		move || rebuild::reconcile(&app)
	})
	.await??;

	let routes = build_router(&app);
	let listener = tokio::net::TcpListener::bind(&config.listen).await?;
	tracing::info!(address = %listener.local_addr()?, "listening");
	let jobs = app.clone();
	tokio::spawn(async move {
		loop {
			tokio::time::sleep(std::time::Duration::from_secs(60)).await;
			if let Err(error) = files::maintenance(&jobs).await {
				tracing::error!(%error, "maintenance failed");
			}
			if let Err(error) = uploads::recover(&jobs).await {
				tracing::error!(%error, "upload recovery failed");
			}
		}
	});
	axum::serve(listener, routes.into_make_service_with_connect_info::<std::net::SocketAddr>())
		.await?;
	Ok(())
}

fn build_router(app: &App) -> Router {
	let json_routes = Router::new()
		.route("/install.sh", get(install::shell))
		.route("/install.ps1", get(install::powershell))
		.route("/api/client/{target}", get(install::binary))
		.route("/api/setup", post(auth::setup))
		.route("/api/login", post(auth::login))
		.route("/api/me", get(auth::me))
		.route("/api/tokens", get(auth::list_tokens).post(auth::create_token))
		.route("/api/tokens/{hash}", axum::routing::delete(auth::revoke_token))
		.route("/api/users", get(auth::list_users).post(auth::create_user))
		.route("/api/users/{id}", axum::routing::patch(auth::update_user))
		.route("/api/uploads", post(uploads::create))
		.route("/api/uploads/{id}", get(uploads::status).delete(uploads::cancel))
		.route("/api/uploads/{id}/complete", post(uploads::complete))
		.route(
			"/api/files",
			get(files::list)
				.post(files::create_folder)
				.delete(files::remove)
				.patch(files::move_file),
		)
		.route("/api/trash", get(trash::list))
		.route("/api/trash/{id}/restore", post(trash::restore))
		.route("/api/download", get(files::download))
		.route("/t/{token}", get(files::shared_file))
		.route("/t/{token}/{*path}", get(files::shared_path))
		.route("/api/dashboard", get(ops::dashboard))
		.route("/api/activity", get(ops::activity))
		.route("/api/search", get(search::search))
		.route("/api/thumbnail", get(ops::thumbnail))
		.route("/api/purge", post(ops::purge))
		.layer(DefaultBodyLimit::max(1024 * 1024));

	let stream_routes = Router::new()
		.route("/api/upload", put(simple_upload::upload))
		.route("/api/uploads/{id}/{number}", put(uploads::chunk))
		.layer(DefaultBodyLimit::disable());

	json_routes
		.merge(stream_routes)
		.layer(axum::middleware::from_fn_with_state(app.clone(), ops::guard))
		.with_state(app.clone())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Cursor;
	use uuid::Uuid;

	struct TempWorkspace {
		root: PathBuf,
	}

	impl TempWorkspace {
		fn new() -> Self {
			let root = std::env::temp_dir().join(format!("flora-test-{}", Uuid::new_v4()));
			std::fs::create_dir_all(&root).expect("create temp workspace");
			Self { root }
		}
	}

	impl Drop for TempWorkspace {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.root);
		}
	}

	async fn spawn_test_server(ws: &TempWorkspace) -> (String, String, App) {
		let data = ws.root.join("data");
		let buffer = ws.root.join("buffer");
		let storage = ws.root.join("storage");
		for dir in [&data, &buffer, &storage, &storage.join(".staging")] {
			std::fs::create_dir_all(dir).expect("create server dirs");
		}
		let db = db::Store::open(&data.join("fjall")).expect("open fjall");
		let setup_code = "test-setup-secret-code".to_owned();
		let app = App {
			db,
			data,
			buffer,
			storage,
			buffer_size: 16 * 1024 * 1024,
			chunk_size: 64 * 1024,
			disk_limit: None,
			buffer_retention: Some(3600),
			trash_retention: "7d".to_owned(),
			public_url: Some("https://files.example.com".to_owned()),
			rate_limit: 600,
			login_rate_limit: 20,
			behind_proxy: true,
			trusted_proxies: vec!["127.0.0.1/32".parse().unwrap()],
			limits: Arc::new(Mutex::new(ops::Limits::default())),
			transfer_locks: Arc::new(transfer_locks::Registry::default()),
			user_locks: Arc::new(transfer_locks::UserLocks::default()),
			accounts: Arc::new(Mutex::new(())),
			reservation: Arc::new(Mutex::new(())),
			chunk_permits: Arc::new(tokio::sync::Semaphore::new(8)),
			thumb_permits: Arc::new(tokio::sync::Semaphore::new(2)),
			setup_code: Arc::new(Mutex::new(Some(setup_code.clone()))),
		};
		let router = build_router(&app);
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
		let addr = listener.local_addr().expect("local addr");
		tokio::spawn(async move {
			let _ = axum::serve(
				listener,
				router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
			)
			.await;
		});
		(format!("http://{addr}"), setup_code, app)
	}

	#[tokio::test]
	async fn end_to_end_transfers_thumbnails_trash_purge_and_rebuild() {
		let ws = TempWorkspace::new();
		let (base, setup_code, app) = spawn_test_server(&ws).await;
		let http = reqwest::Client::new();

		// Unknown JSON fields are rejected instead of silently ignored.
		let bad_setup = http
			.post(format!("{base}/api/setup"))
			.json(
				&json!({"code": setup_code, "name": "alice", "password": "correct-horse", "typo": true}),
			)
			.send()
			.await
			.unwrap();
		assert_eq!(bad_setup.status(), StatusCode::UNPROCESSABLE_ENTITY);

		// Valid setup + login.
		let setup = http
			.post(format!("{base}/api/setup"))
			.json(&json!({"code": setup_code, "name": "Alice", "password": "correct-horse"}))
			.send()
			.await
			.unwrap();
		assert_eq!(setup.status(), StatusCode::OK);

		let login = http
			.post(format!("{base}/api/login"))
			.header("X-Forwarded-For", "198.51.100.42, 127.0.0.1")
			.json(&json!({"name": "alice", "password": "correct-horse"}))
			.send()
			.await
			.unwrap();
		assert_eq!(login.status(), StatusCode::OK);
		let login_json: serde_json::Value = login.json().await.unwrap();
		let token = login_json["token"].as_str().unwrap().to_owned();
		let user_id = login_json["user"]["id"].as_str().unwrap().to_owned();

		// Direct streaming upload rejects malformed X-XXH3-128 upfront.
		let bad_hash = http
			.put(format!("{base}/api/upload?path=docs/bad.txt"))
			.bearer_auth(&token)
			.header("X-XXH3-128", "not-a-valid-hex-digest")
			.body("hello")
			.send()
			.await
			.unwrap();
		assert_eq!(bad_hash.status(), StatusCode::BAD_REQUEST);

		// Multi-chunk upload, single-pass assembly, HEAD probe, and ranged download.
		let payload: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
		let expected_hash = core::hex::u128(xxhash_rust::xxh3::xxh3_128(&payload));
		let reservation: core::Upload = http
			.post(format!("{base}/api/uploads"))
			.bearer_auth(&token)
			.json(&json!({"path": "docs/blob.bin", "size": payload.len()}))
			.send()
			.await
			.unwrap()
			.json()
			.await
			.unwrap();
		for (idx, part) in payload.chunks(64 * 1024).enumerate() {
			let put_resp = http
				.put(format!("{base}/api/uploads/{}/{idx}", reservation.id))
				.bearer_auth(&token)
				.header(axum::http::header::CONTENT_LENGTH, part.len())
				.body(part.to_vec())
				.send()
				.await
				.unwrap();
			assert_eq!(put_resp.status(), StatusCode::OK);
		}
		let completed: core::FileRecord = http
			.post(format!("{base}/api/uploads/{}/complete", reservation.id))
			.bearer_auth(&token)
			.json(&json!({"size": payload.len(), "hash": expected_hash}))
			.send()
			.await
			.unwrap()
			.json()
			.await
			.unwrap();
		assert_eq!(completed.hash, expected_hash);

		let head_resp = http
			.head(format!("{base}/api/download?path=docs/blob.bin"))
			.bearer_auth(&token)
			.send()
			.await
			.unwrap();
		assert_eq!(head_resp.status(), StatusCode::OK);
		assert_eq!(
			head_resp.headers().get(axum::http::header::CONTENT_LENGTH).unwrap(),
			&payload.len().to_string()
		);
		assert!(head_resp.bytes().await.unwrap().is_empty());

		let range_resp = http
			.get(format!("{base}/api/download?path=docs/blob.bin"))
			.bearer_auth(&token)
			.header(axum::http::header::RANGE, "bytes=100-199")
			.send()
			.await
			.unwrap();
		assert_eq!(range_resp.status(), StatusCode::PARTIAL_CONTENT);
		assert_eq!(range_resp.bytes().await.unwrap().as_ref(), &payload[100..=199]);

		// Upload PNG via simple_upload, fetch WebP thumbnail + 304 Not Modified, then trash & purge.
		let mut png_bytes = Cursor::new(Vec::new());
		image::DynamicImage::new_rgba8(16, 16)
			.write_to(&mut png_bytes, image::ImageFormat::Png)
			.unwrap();
		let png_raw = png_bytes.into_inner();
		let img_record: core::FileRecord = http
			.put(format!("{base}/api/upload?path=photos/pic.png"))
			.bearer_auth(&token)
			.body(png_raw)
			.send()
			.await
			.unwrap()
			.json()
			.await
			.unwrap();

		let thumb = http
			.get(format!("{base}/api/thumbnail?path=photos/pic.png"))
			.bearer_auth(&token)
			.send()
			.await
			.unwrap();
		assert_eq!(thumb.status(), StatusCode::OK);
		assert_eq!(thumb.headers().get(axum::http::header::CONTENT_TYPE).unwrap(), "image/webp");
		let thumb_etag =
			thumb.headers().get(axum::http::header::ETAG).unwrap().to_str().unwrap().to_owned();
		let cached_webp =
			app.data.join("thumbnails").join(&user_id).join(format!("{}.webp", img_record.hash));
		assert!(cached_webp.exists());

		let not_modified = http
			.get(format!("{base}/api/thumbnail?path=photos/pic.png"))
			.bearer_auth(&token)
			.header(axum::http::header::IF_NONE_MATCH, &thumb_etag)
			.send()
			.await
			.unwrap();
		assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);

		// Listing root returns top-level folders without scanning/leaking internal directories.
		let root_entries: Vec<core::Entry> = http
			.get(format!("{base}/api/files"))
			.bearer_auth(&token)
			.send()
			.await
			.unwrap()
			.json()
			.await
			.unwrap();
		let names: Vec<&str> = root_entries.iter().map(|e| e.name.as_str()).collect();
		assert_eq!(names, vec!["docs", "photos"]);

		// Trash & purge evicts cached thumbnail.
		let del = http
			.delete(format!("{base}/api/files?path=photos/pic.png"))
			.bearer_auth(&token)
			.send()
			.await
			.unwrap();
		assert_eq!(del.status(), StatusCode::NO_CONTENT);
		let purge_resp = http
			.post(format!("{base}/api/purge"))
			.bearer_auth(&token)
			.json(&json!({"confirm": true}))
			.send()
			.await
			.unwrap();
		assert_eq!(purge_resp.status(), StatusCode::OK);
		assert!(!cached_webp.exists(), "purging file should evict its cached WebP thumbnail");

		// Windows .exe asset lookup works for /api/client/windows-x86_64.
		let assets_dir = app.data.join("assets");
		std::fs::create_dir_all(&assets_dir).unwrap();
		std::fs::write(assets_dir.join("flora-windows-x86_64.exe"), b"MZ-flora-stub").unwrap();
		let win_bin = http.get(format!("{base}/api/client/windows-x86_64")).send().await.unwrap();
		assert_eq!(win_bin.status(), StatusCode::OK);
		assert_eq!(win_bin.bytes().await.unwrap().as_ref(), b"MZ-flora-stub");

		// Rebuild recovers user & live files from storage/.flora when Fjall is recreated.
		let fresh_db_dir = ws.root.join("rebuilt-fjall");
		let rebuilt_app = App { db: db::Store::open(&fresh_db_dir).unwrap(), ..app.clone() };
		rebuild::rebuild(&rebuilt_app).unwrap();
		rebuilt_app.db.ensure_indexes().unwrap();
		let recovered_user = rebuilt_app.db.find_user_by_name_async("ALICE").await.unwrap();
		assert_eq!(recovered_user.expect("recovered user").id, user_id);
		let recovered_file: Option<core::FileRecord> =
			rebuilt_app.db.get(&db::file(&user_id, "docs/blob.bin")).unwrap();
		assert_eq!(recovered_file.expect("recovered file").hash, expected_hash);
	}
}
