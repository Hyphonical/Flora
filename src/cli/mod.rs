//! Dispatch file-focused CLI commands and transfer workflows.

mod client;
mod download;
mod help;
mod render;
mod style;
mod upload;

use crate::core::{
	Entry,
	config::{Command, Config, GlobalOptions, OutputFlags},
};
use anyhow::{Result, bail};
use client::{Client, Credentials, checked, client, credentials_path, save_credentials, send};
use render::{blank, bytes, column, done, event, interactive, line, row, rule};
use style::Color;

pub(crate) use render::error;

pub(crate) fn help(flags: OutputFlags, command: Option<&str>) {
	help::show(flags, command);
}

pub(crate) fn version() {
	help::version();
}

pub(crate) async fn run(config: &Config) -> Result<()> {
	let flags = config.output_flags();
	tracing::debug!(command = %config.action.name(), "running CLI command");
	match &config.action {
		Command::Login { url } => login(&config.global, flags, url).await,
		Command::Upload(up) => upload::run(&config.global, up).await,
		Command::Download(down) => download::run(&config.global, down).await,
		Command::Cat { target } => download::cat(&config.global, target).await,
		Command::Ls { path } => list(&config.global, flags, path).await,
		Command::Mv { from, to } => move_file(&config.global, flags, from, to).await,
		Command::Rm { path } => remove(&config.global, flags, path).await,
		Command::Help(_) | Command::Version | Command::Serve(_) => {
			bail!("invalid CLI command dispatch")
		}
	}
}

async fn login(global: &GlobalOptions, flags: OutputFlags, server_url: &str) -> Result<()> {
	rule(flags, "login");
	let server = server_url.trim_end_matches('/').to_owned();
	let token = match &global.token {
		Some(token) => token.clone(),
		None => rpassword::prompt_password("  > token: ")?,
	};
	let client = Client::new(&server, token.clone())?;
	let response = send(client.request(reqwest::Method::GET, "/api/me")).await?;
	if response.status() == reqwest::StatusCode::FORBIDDEN {
		bail!("token cannot be used for account login");
	}
	let response: serde_json::Value = checked(response).await?.json().await?;
	if response["kind"].as_str() != Some("account") {
		bail!("login requires an account token");
	}
	let path = credentials_path()?;
	save_credentials(&Credentials { server, token })?;
	let name = response["user"]["name"].as_str().unwrap_or("user");
	let role = if response["user"]["admin"].as_bool() == Some(true) { "admin" } else { "user" };
	done(flags, "login", "login", &format!("signed in as {name} ({role})"));
	#[cfg(unix)]
	row(flags, "login", "saved", &format!("{} (owner-only)", path.display()), Color::Dim);
	#[cfg(not(unix))]
	row(flags, "login", "saved", &path.display().to_string(), Color::Dim);
	tracing::debug!("account credentials saved");
	Ok(())
}

async fn list(global: &GlobalOptions, flags: OutputFlags, path: &str) -> Result<()> {
	rule(flags, "ls");
	let client = client(global)?;
	let entries: Vec<Entry> = checked(
		send(client.request(reqwest::Method::GET, "/api/files").query(&[("path", path)])).await?,
	)
	.await?
	.json()
	.await?;
	tracing::debug!(entries = entries.len(), "listing received");
	let count = entries.len();
	let total = entries
		.iter()
		.filter(|entry| !entry.directory)
		.fold(0u64, |sum, entry| sum.saturating_add(entry.size));
	let name_width = 34;
	if interactive() {
		line(
			flags,
			&format!("{}  {:<10}  modified", column("name", name_width), "size"),
			Color::Muted,
		);
	}
	for entry in entries {
		let name = format!("{}{}", entry.name, if entry.directory { "/" } else { "" });
		let size = if entry.directory { "—".to_owned() } else { bytes(entry.size) };
		let modified = if entry.directory {
			"—".to_owned()
		} else {
			chrono::DateTime::from_timestamp(entry.modified as i64, 0)
				.map(|v| {
					if interactive() {
						v.format("%b %d %H:%M UTC").to_string()
					} else {
						v.format("%Y-%m-%dT%H:%M:%SZ").to_string()
					}
				})
				.unwrap_or_else(|| "—".to_owned())
		};
		if interactive() {
			line(
				flags,
				&format!("{}  {:<10}  {modified}", column(&name, name_width), size),
				if entry.directory { Color::Accent } else { Color::Text },
			);
		} else {
			event(flags, "ls", &format!("{name}  {size}  {modified}"));
		}
	}
	blank(flags);
	event(flags, "ls", &format!("{count} items · {}", bytes(total)));
	Ok(())
}

async fn move_file(global: &GlobalOptions, flags: OutputFlags, from: &str, to: &str) -> Result<()> {
	rule(flags, "mv");
	let client = client(global)?;
	checked(
		send(
			client
				.request(reqwest::Method::PATCH, "/api/files")
				.json(&serde_json::json!({"from": from, "to": to})),
		)
		.await?,
	)
	.await?;
	done(flags, "mv", "moved", &format!("{from} → {to}"));
	Ok(())
}

async fn remove(global: &GlobalOptions, flags: OutputFlags, path: &str) -> Result<()> {
	rule(flags, "rm");
	let client = client(global)?;
	checked(
		send(client.request(reqwest::Method::DELETE, "/api/files").query(&[("path", path)]))
			.await?,
	)
	.await?;
	done(flags, "rm", "trashed", path);
	Ok(())
}
