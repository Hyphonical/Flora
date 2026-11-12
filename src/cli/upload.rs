//! Directory and file uploads, chunk retries, and cached remote listing checks.

use super::{
	client::{Client, checked, client, send},
	render::{self, ProgressBar, ProgressUnit, blank, bytes, done, failure, row, rule},
	style::Color,
};
use crate::core::{
	self, CompleteRequest, Entry, FileRecord, Upload, UploadRequest,
	config::{GlobalOptions, OutputFlags, UploadConfig},
};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use std::{
	collections::{HashMap, HashSet},
	path::{Path, PathBuf},
	time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt};
use xxhash_rust::xxh3::Xxh3;

enum LocalItem {
	Directory { remote: String },
	File { local: PathBuf, remote: String, size: u64 },
}

struct RemoteTreeCache {
	listings: HashMap<String, Option<HashMap<String, Entry>>>,
	folders: HashSet<String>,
}

impl RemoteTreeCache {
	fn new() -> Self {
		Self { listings: HashMap::new(), folders: HashSet::new() }
	}

	async fn listing(
		&mut self,
		client: &Client,
		parent: &str,
	) -> Result<Option<&HashMap<String, Entry>>> {
		if !self.listings.contains_key(parent) {
			let response =
				send(client.request(reqwest::Method::GET, "/api/files").query(&[("path", parent)]))
					.await?;
			let fetched = match response.status() {
				reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::FORBIDDEN => None,
				_ => {
					let entries: Vec<Entry> = checked(response).await?.json().await?;
					let mut map = HashMap::with_capacity(entries.len());
					for entry in entries {
						if entry.directory {
							self.folders.insert(entry.path.clone());
						}
						map.insert(entry.path.clone(), entry);
					}
					Some(map)
				}
			};
			self.listings.insert(parent.to_owned(), fetched);
		}
		Ok(self.listings.get(parent).and_then(Option::as_ref))
	}

	async fn ensure_folder(&mut self, client: &Client, path: &str) -> Result<()> {
		if self.folders.contains(path) {
			return Ok(());
		}
		let parent = parent_dir(path);
		if let Some(entries) = self.listing(client, parent).await?
			&& let Some(existing) = entries.get(path)
		{
			if existing.directory {
				self.folders.insert(path.to_owned());
				return Ok(());
			}
			bail!("remote path {path} already exists as a file");
		}
		let response = send(
			client
				.request(reqwest::Method::POST, "/api/files")
				.json(&serde_json::json!({"path": path})),
		)
		.await?;
		if response.status() != reqwest::StatusCode::CONFLICT {
			checked(response).await?;
		}
		self.folders.insert(path.to_owned());
		Ok(())
	}

	async fn same_remote(
		&mut self,
		client: &Client,
		source: &Path,
		path: &str,
		size: u64,
	) -> Result<bool> {
		let parent = parent_dir(path);
		let Some(entries) = self.listing(client, parent).await? else {
			return Ok(false);
		};
		let Some(entry) = entries.get(path) else {
			return Ok(false);
		};
		if entry.directory || entry.size != size {
			return Ok(false);
		}
		let Some(expected) = &entry.hash else {
			return Ok(false);
		};
		Ok(core::hex::hash_file_async(source).await?.eq_ignore_ascii_case(expected))
	}
}

pub(super) async fn run(global: &GlobalOptions, up: &UploadConfig) -> Result<()> {
	let flags = OutputFlags { quiet: global.quiet, no_color: global.no_color };
	let verbose_log = matches!(global.log.to_ascii_lowercase().as_str(), "debug" | "trace");
	let client = client(global)?;
	let cmd = up.label;
	let destination = up.destination.as_str();
	rule(flags, cmd);
	if let Ok(url) = reqwest::Url::parse(&client.server)
		&& let Some(host) = url.host_str()
	{
		row(flags, cmd, "server", host, Color::Dim);
	}
	if up.source == "-" {
		let name = up.name.as_deref().context("--name is required when uploading stdin")?;
		let path = join(destination, name);
		row(flags, cmd, "file", name, Color::Text);
		row(
			flags,
			cmd,
			"into",
			if destination.is_empty() { "/" } else { destination },
			Color::Accent,
		);
		blank(flags);
		upload_reader(flags, verbose_log, up, &client, tokio::io::stdin(), None, &path, false)
			.await?;
		return Ok(());
	}

	let source = Path::new(&up.source);
	let meta = tokio::fs::symlink_metadata(source).await?;
	if meta.file_type().is_symlink() {
		bail!("symbolic-link upload sources are not supported");
	}
	if meta.is_dir() {
		let root_name = source
			.file_name()
			.context("directory needs a name")?
			.to_str()
			.context("directory name is not UTF-8")?
			.to_owned();
		let root = join(destination, &root_name);
		let source_buf = source.to_path_buf();
		let root_clone = root.clone();
		let (items, file_count, total_bytes, mut failed) =
			tokio::task::spawn_blocking(move || scan_directory(&source_buf, &root_clone)).await??;

		row(
			flags,
			cmd,
			"folder",
			&format!("{} · {file_count} files · {}", source.display(), bytes(total_bytes)),
			Color::Text,
		);
		row(
			flags,
			cmd,
			"into",
			if destination.is_empty() { "/" } else { destination },
			Color::Accent,
		);
		row(flags, cmd, "conflict", up.on_conflict.as_str(), Color::Accent);
		blank(flags);

		let mut cache = RemoteTreeCache::new();
		cache.ensure_folder(&client, &root).await?;
		let mut uploaded = 0usize;
		let mut skipped = 0usize;
		let mut renamed = 0usize;

		for item in items {
			match item {
				LocalItem::Directory { remote } => {
					if let Err(error) = cache.ensure_folder(&client, &remote).await {
						failure(flags, cmd, &format!("{remote}: {error:#}"));
						failed += 1;
					}
				}
				LocalItem::File { local, remote, size } => {
					let outcome: Result<Option<FileRecord>> = async {
						if cache.same_remote(&client, &local, &remote, size).await? {
							row(
								flags,
								cmd,
								"skip",
								&format!("{remote}  {}", bytes(size)),
								Color::Dim,
							);
							return Ok(None);
						}
						let file = tokio::fs::File::open(&local).await?;
						let current_size = file.metadata().await?.len();
						let record = upload_reader(
							flags,
							verbose_log,
							up,
							&client,
							file,
							Some(current_size),
							&remote,
							true,
						)
						.await?;
						Ok(Some(record))
					}
					.await;

					match outcome {
						Ok(Some(record)) => {
							uploaded += 1;
							let was_renamed = record.path != remote;
							if was_renamed {
								renamed += 1;
							}
							row(
								flags,
								cmd,
								"stored",
								&format!("{}  {}", record.path, bytes(record.size)),
								if was_renamed { Color::Warning } else { Color::Accent },
							);
						}
						Ok(None) => skipped += 1,
						Err(error) => {
							failure(flags, cmd, &format!("{remote}: {error:#}"));
							failed += 1;
						}
					}
				}
			}
		}

		blank(flags);
		row(
			flags,
			cmd,
			"summary",
			&format!(
				"{uploaded} uploaded · {skipped} skipped · {failed} failed · {renamed} renamed"
			),
			Color::Text,
		);
		if failed != 0 {
			bail!("{failed} item(s) failed during directory upload");
		}
	} else {
		let name = match &up.name {
			Some(name) => name.clone(),
			None => source
				.file_name()
				.context("file needs a name")?
				.to_str()
				.context("file name is not valid UTF-8")?
				.to_owned(),
		};
		let path = join(destination, &name);
		let file = tokio::fs::File::open(source).await?;
		let size = file.metadata().await?.len();
		row(flags, cmd, "file", &source.display().to_string(), Color::Text);
		row(flags, cmd, "size", &bytes(size), Color::Text);
		row(
			flags,
			cmd,
			"into",
			if destination.is_empty() { "/" } else { destination },
			Color::Accent,
		);
		blank(flags);
		upload_reader(flags, verbose_log, up, &client, file, Some(size), &path, false).await?;
	}
	Ok(())
}

fn scan_directory(source: &Path, root: &str) -> Result<(Vec<LocalItem>, usize, u64, usize)> {
	let mut items = Vec::new();
	let mut file_count = 0usize;
	let mut total_bytes = 0u64;
	let mut failed = 0usize;
	for entry in core::walk::walk(source, |_| true)? {
		let Ok(entry) = entry else {
			failed += 1;
			continue;
		};
		let entry_path = entry.path();
		let Some(relative) = entry_path.strip_prefix(source).ok().and_then(|p| p.to_str()) else {
			failed += 1;
			continue;
		};
		let remote = join(root, &relative.replace('\\', "/"));
		let Ok(kind) = entry.file_type() else {
			failed += 1;
			continue;
		};
		if kind.is_dir() {
			items.push(LocalItem::Directory { remote });
		} else if kind.is_file() {
			match entry.metadata() {
				Ok(meta) => {
					let size = meta.len();
					file_count += 1;
					total_bytes = total_bytes.saturating_add(size);
					items.push(LocalItem::File { local: entry_path, remote, size });
				}
				Err(_) => failed += 1,
			}
		}
	}
	Ok((items, file_count, total_bytes, failed))
}

fn parent_dir(path: &str) -> &str {
	path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn join(dir: &str, name: &str) -> String {
	if dir.trim_matches('/').is_empty() {
		name.trim_start_matches('/').to_owned()
	} else {
		format!("{}/{}", dir.trim_matches('/'), name.trim_start_matches('/'))
	}
}

async fn send_chunk(
	client: &Client,
	upload: &str,
	number: usize,
	data: bytes::Bytes,
) -> Result<(usize, u64)> {
	let url = format!("/api/uploads/{upload}/{number}");
	let size = data.len() as u64;
	let mut last_error = None;
	for attempt in 0..3 {
		match send(client.request(reqwest::Method::PUT, &url).body(data.clone())).await {
			Ok(response) if response.status().is_success() => return Ok((number, size)),
			Ok(response) => {
				let retry = response.status().is_server_error()
					|| response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS;
				let error = checked(response).await.err().context("chunk request failed")?;
				if !retry {
					return Err(error);
				}
				last_error = Some(error);
			}
			Err(error) => last_error = Some(error),
		}
		if attempt < 2 {
			tracing::debug!(chunk = number, attempt = attempt + 1, "retrying chunk upload");
			tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await;
		}
	}
	Err(last_error.context("chunk upload failed")?)
}

#[allow(clippy::too_many_arguments)]
async fn upload_reader<R: AsyncRead + Unpin>(
	flags: OutputFlags,
	verbose_log: bool,
	up: &UploadConfig,
	client: &Client,
	mut reader: R,
	size: Option<u64>,
	path: &str,
	directory: bool,
) -> Result<FileRecord> {
	let path = core::clean_path(path, false)?;
	let started = Instant::now();
	let request = UploadRequest { path: path.clone(), owner: None, size, conflict: up.on_conflict };
	let upload: Upload =
		checked(send(client.request(reqwest::Method::POST, "/api/uploads").json(&request)).await?)
			.await?
			.json()
			.await?;
	tracing::debug!(chunk_size = upload.chunk_size, reserved = upload.reserved, "upload reserved");
	let chunk_size = usize::try_from(upload.chunk_size).context("chunk size too large")?;
	let mut buf = vec![0u8; chunk_size];
	let mut whole = Xxh3::new();
	let mut total = 0u64;
	let mut number = 0usize;
	let mut finished = 0u64;
	let mut reused = 0u64;
	let mut transferred = 0u64;
	let expected = size.map(|s| s.div_ceil(upload.chunk_size));
	let cmd = up.label;
	let mut bar = (!directory).then(|| {
		ProgressBar::new(flags, cmd, "chunks", expected, ProgressUnit::Count, verbose_log)
	});
	let mut pending = futures_util::stream::FuturesUnordered::new();

	loop {
		let mut filled = 0;
		while filled < buf.len() {
			let n = reader.read(&mut buf[filled..]).await?;
			if n == 0 {
				break;
			}
			filled += n;
		}
		if filled == 0 {
			break;
		}
		whole.update(&buf[..filled]);
		total += filled as u64;
		let chunk_hash = core::hex::u128(xxhash_rust::xxh3::xxh3_128(&buf[..filled]));
		let present = upload
			.chunks
			.get(number)
			.and_then(Option::as_ref)
			.is_some_and(|c| c.size == filled as u64 && c.hash == chunk_hash);
		if present {
			finished += 1;
			reused += 1;
			if let Some(bar) = &mut bar {
				bar.update(finished);
			}
		} else {
			pending.push(send_chunk(
				client,
				&upload.id,
				number,
				bytes::Bytes::copy_from_slice(&buf[..filled]),
			));
			if pending.len() >= up.parallel
				&& let Some(result) = pending.next().await
			{
				let (_, sent) = result?;
				transferred += sent;
				finished += 1;
				if let Some(bar) = &mut bar {
					bar.update(finished);
				}
			}
		}
		number += 1;
		if filled < buf.len() {
			break;
		}
	}
	while let Some(result) = pending.next().await {
		let (_, sent) = result?;
		transferred += sent;
		finished += 1;
		if let Some(bar) = &mut bar {
			bar.update(finished);
		}
	}
	drop(bar);

	if size.is_some_and(|s| s != total) {
		bail!("source file changed during upload");
	}
	let complete = CompleteRequest { hash: Some(core::hex::u128(whole.digest128())), size: total };
	let url = format!("/api/uploads/{}/complete", upload.id);
	let record: FileRecord =
		checked(send(client.request(reqwest::Method::POST, &url).json(&complete)).await?)
			.await?
			.json()
			.await?;
	tracing::debug!(reused, transferred, "upload completed");
	if !directory {
		if reused > 0 {
			row(flags, cmd, "reused", &format!("{reused} chunks"), Color::Dim);
		}
		done(flags, cmd, "stored", &record.path);
		row(flags, cmd, "size", &bytes(record.size), Color::Text);
		let elapsed = started.elapsed();
		row(flags, cmd, "elapsed", &render::elapsed(elapsed), Color::Dim);
		if transferred > 0 && elapsed.as_secs_f64() > 0.0 {
			let rate = (transferred as f64 / elapsed.as_secs_f64()) as u64;
			row(flags, cmd, "rate", &format!("{}/s", bytes(rate)), Color::Dim);
		}
	}
	Ok(record)
}
