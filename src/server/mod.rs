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
	pub chunk_permits: Arc<tokio::sync::Semaphore>,
	pub mutation: Arc<Mutex<()>>,
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

fn bad(message: impl Into<String>) -> Error {
	Error(StatusCode::BAD_REQUEST, anyhow::anyhow!(message.into()))
}

fn forbidden() -> Error {
	Error(StatusCode::FORBIDDEN, anyhow::anyhow!("permission denied"))
}

fn missing() -> Error {
	Error(StatusCode::NOT_FOUND, anyhow::anyhow!("not found"))
}

fn conflict(message: impl Into<String>) -> Error {
	Error(StatusCode::CONFLICT, anyhow::anyhow!(message.into()))
}

pub(crate) async fn run(config: &ServeConfig) -> anyhow::Result<()> {
	for path in [&config.data, &config.buffer, &config.storage] {
		std::fs::create_dir_all(path)?;
	}
	let data = std::fs::canonicalize(&config.data)?;
	let buffer = std::fs::canonicalize(&config.buffer)?;
	let storage = std::fs::canonicalize(&config.storage)?;
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
		chunk_permits: Arc::new(tokio::sync::Semaphore::new(config.upload_concurrency)),
		setup_code: Arc::new(Mutex::new(setup_code)),
		mutation: Arc::new(Mutex::new(())),
	};
	tokio::task::spawn_blocking({
		let app = app.clone();
		move || -> anyhow::Result<()> {
			profile::recover_writes(&app)?;
			rebuild::rebuild(&app)?;
			rebuild::sync_profiles(&app)?;
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

	let routes = json_routes
		.merge(stream_routes)
		.layer(axum::middleware::from_fn_with_state(app.clone(), ops::guard))
		.with_state(app.clone());

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
