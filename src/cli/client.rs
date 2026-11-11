//! HTTP client construction, sanitized transport helpers, and local account credentials.

use crate::core::config::GlobalOptions;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Serialize, Deserialize)]
pub(super) struct Credentials {
	pub server: String,
	pub token: String,
}

pub(super) struct Client {
	pub http: reqwest::Client,
	pub server: String,
	pub token: String,
}

pub(super) fn http_client() -> Result<reqwest::Client> {
	Ok(reqwest::Client::builder()
		.user_agent(concat!("Flora/", env!("CARGO_PKG_VERSION")))
		.timeout(REQUEST_TIMEOUT)
		.build()?)
}

impl Client {
	pub(super) fn new(server: &str, token: String) -> Result<Self> {
		let url = reqwest::Url::parse(server).context("invalid server URL")?;
		if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
			bail!("server URL must use http or https");
		}
		Ok(Self {
			http: http_client()?,
			server: server.trim_end_matches('/').to_owned(),
			token,
		})
	}

	pub(super) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
		self.http.request(method, format!("{}{path}", self.server)).bearer_auth(&self.token)
	}
}

pub(super) async fn send(request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
	request.send().await.map_err(|error| error.without_url().into())
}

pub(super) async fn next_chunk(response: &mut reqwest::Response) -> Result<Option<bytes::Bytes>> {
	response.chunk().await.map_err(|error| error.without_url().into())
}

pub(super) fn credentials_path() -> Result<PathBuf> {
	Ok(dirs::config_dir()
		.context("could not find the user configuration directory")?
		.join("flora")
		.join("credentials"))
}

pub(super) fn load_credentials() -> Result<Credentials> {
	let path = credentials_path()?;
	let data = std::fs::read(&path)
		.with_context(|| format!("not signed in ({})", path.display()))?;
	Ok(serde_json::from_slice(&data)?)
}

pub(super) fn save_credentials(credentials: &Credentials) -> Result<()> {
	let path = credentials_path()?;
	let parent = path.parent().context("invalid credentials path")?;
	std::fs::create_dir_all(parent)?;
	#[cfg(unix)]
	{
		use std::io::Write;
		use std::os::unix::fs::OpenOptionsExt;
		let mut file = std::fs::OpenOptions::new()
			.write(true)
			.create(true)
			.truncate(true)
			.mode(0o600)
			.open(&path)?;
		file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
		file.write_all(&serde_json::to_vec(credentials)?)?;
	}
	#[cfg(not(unix))]
	std::fs::write(path, serde_json::to_vec(credentials)?)?;
	Ok(())
}

pub(super) fn client(global: &GlobalOptions) -> Result<Client> {
	let stored = if global.server.is_some() && global.token.is_some() {
		None
	} else {
		load_credentials().ok()
	};
	let server = global
		.server
		.clone()
		.or_else(|| stored.as_ref().map(|c| c.server.clone()))
		.context("not signed in: missing server URL")?;
	if global.token.is_none()
		&& stored
			.as_ref()
			.is_some_and(|saved| saved.server.trim_end_matches('/') != server.trim_end_matches('/'))
	{
		bail!(
			"refusing to send saved credentials to a different server; set FLORA_TOKEN explicitly"
		);
	}
	let token = global
		.token
		.clone()
		.or_else(|| stored.map(|c| c.token))
		.context("not signed in: missing authentication token")?;
	Client::new(&server, token)
}

pub(super) async fn checked(response: reqwest::Response) -> Result<reqwest::Response> {
	if response.status().is_success() {
		return Ok(response);
	}
	let status = response.status();
	tracing::debug!(%status, "server request failed");
	let message = response.text().await.unwrap_or_default();
	let message = serde_json::from_str::<serde_json::Value>(&message)
		.ok()
		.and_then(|v| v.get("error").and_then(|v| v.as_str()).map(str::to_owned))
		.unwrap_or(message);
	bail!("server returned {status}: {message}")
}
