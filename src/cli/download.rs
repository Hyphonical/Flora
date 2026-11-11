//! Parallel downloads saved in order to a single resumable .partial file.

use super::{
	client::{checked, client, http_client, next_chunk, send},
	render::{self, ProgressBar, ProgressUnit, blank, bytes, done, row, rule},
	style::Color,
};
use crate::core::{
	self,
	config::{DownloadConfig, GlobalOptions, OutputFlags},
};
use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream};
use std::{
	path::{Path, PathBuf},
	time::Instant,
};
use tokio::io::AsyncWriteExt;
use xxhash_rust::xxh3::Xxh3;

const RANGE_SIZE: u64 = 8_000_000;

struct Source {
	http: reqwest::Client,
	url: String,
	token: Option<String>,
	path: Option<String>,
}

impl Source {
	fn from_target(global: &GlobalOptions, target: &str) -> Result<Self> {
		if target.starts_with("http://") || target.starts_with("https://") {
			let url = reqwest::Url::parse(target).context("invalid download URL")?;
			Ok(Self { http: http_client()?, url: url.to_string(), token: None, path: None })
		} else {
			let client = client(global)?;
			Ok(Self {
				http: client.http,
				url: format!("{}/api/download", client.server),
				token: Some(client.token),
				path: Some(target.to_owned()),
			})
		}
	}

	fn request_method(&self, method: reqwest::Method) -> reqwest::RequestBuilder {
		let mut request = self.http.request(method, &self.url);
		if let Some(token) = &self.token {
			request = request.bearer_auth(token);
		}
		if let Some(path) = &self.path {
			request = request.query(&[("path", path)]);
		}
		request
	}

	fn request(&self) -> reqwest::RequestBuilder {
		self.request_method(reqwest::Method::GET)
	}

	fn head(&self) -> reqwest::RequestBuilder {
		self.request_method(reqwest::Method::HEAD)
	}

	async fn range(
		&self,
		start: u64,
		end: u64,
		size: u64,
		etag: &str,
	) -> Result<reqwest::Response> {
		let response = checked(
			send(
				self.request()
					.header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
					.header(reqwest::header::IF_RANGE, etag),
			)
			.await?,
		)
		.await?;
		let expected = format!("bytes {start}-{end}/{size}");
		if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
			|| response.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok())
				!= Some(expected.as_str())
			|| response.headers().get(reqwest::header::ETAG).and_then(|v| v.to_str().ok())
				!= Some(etag)
		{
			bail!("the server file changed or did not provide the requested range");
		}
		Ok(response)
	}
}

pub(super) async fn cat(global: &GlobalOptions, target: &str) -> Result<()> {
	let source = Source::from_target(global, target)?;
	let mut response = checked(send(source.request()).await?).await?;
	stream_stdout(&mut response).await
}

pub(super) async fn run(global: &GlobalOptions, down: &DownloadConfig) -> Result<()> {
	let flags = OutputFlags { quiet: global.quiet, no_color: global.no_color };
	let verbose_log = matches!(global.log.to_ascii_lowercase().as_str(), "debug" | "trace");
	let started = Instant::now();
	let source = Source::from_target(global, &down.target)?;
	if down.output.as_deref() == Some("-") {
		let mut response = checked(send(source.request()).await?).await?;
		return stream_stdout(&mut response).await;
	}

	let probe = send(source.head()).await?;
	let (headers, body_response) = if matches!(
		probe.status(),
		reqwest::StatusCode::METHOD_NOT_ALLOWED | reqwest::StatusCode::NOT_IMPLEMENTED
	) {
		let get_resp = checked(send(source.request()).await?).await?;
		(get_resp.headers().clone(), Some(get_resp))
	} else {
		let head_resp = checked(probe).await?;
		(head_resp.headers().clone(), None)
	};
	tracing::debug!("download metadata received");

	let cmd = down.label;
	rule(flags, cmd);
	let disposition = headers
		.get(reqwest::header::CONTENT_DISPOSITION)
		.and_then(|v| v.to_str().ok())
		.unwrap_or("");
	let filename = resolve_filename(&source, disposition);
	let destination = match down.output.as_deref() {
		None | Some(".") => PathBuf::from(&filename),
		Some(path) => {
			let candidate = PathBuf::from(path);
			if tokio::fs::metadata(&candidate).await.is_ok_and(|m| m.is_dir()) {
				candidate.join(&filename)
			} else {
				candidate
			}
		}
	};
	let name =
		destination.file_name().and_then(|s| s.to_str()).context("invalid output filename")?;
	core::clean_path(name, false)?;
	if tokio::fs::try_exists(&destination).await? {
		bail!("destination {} already exists", destination.display());
	}

	let partial = partial_path(&destination, name);
	let (exists, offset) = match tokio::fs::symlink_metadata(&partial).await {
		Ok(metadata) if metadata.file_type().is_file() => (true, metadata.len()),
		Ok(_) => bail!("partial download is not a regular file: {}", partial.display()),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, 0),
		Err(error) => return Err(error.into()),
	};

	let size = headers
		.get(reqwest::header::CONTENT_LENGTH)
		.and_then(|v| v.to_str().ok())
		.and_then(|v| v.parse::<u64>().ok())
		.or_else(|| body_response.as_ref().and_then(reqwest::Response::content_length));
	if let Ok(url) = reqwest::Url::parse(&source.url)
		&& let Some(host) = url.host_str()
	{
		row(flags, cmd, "from", host, Color::Dim);
	}
	let display_dest = destination.display().to_string();
	row(flags, cmd, "file", &display_dest, Color::Text);
	if let Some(size) = size {
		row(flags, cmd, "size", &bytes(size), Color::Text);
	}
	if size.is_some_and(|total| offset > total) {
		bail!("partial download is larger than the server file: {}", partial.display());
	}

	let etag = headers.get(reqwest::header::ETAG).and_then(|v| v.to_str().ok()).map(str::to_owned);
	let ranges =
		headers.get(reqwest::header::ACCEPT_RANGES).and_then(|v| v.to_str().ok()) == Some("bytes");
	let expected_xxh3 = etag.as_deref().and_then(flora_xxh3_etag);

	if let (Some(size), Some(etag)) = (size, etag.as_deref().filter(|_| ranges))
		&& (size > RANGE_SIZE || (offset > 0 && expected_xxh3.is_some()))
	{
		drop(body_response);
		if offset > 0 {
			row(flags, cmd, "resume", &bytes(offset), Color::Dim);
		}
		blank(flags);
		let mut file = open_partial(&partial, exists).await?;
		let count = size.saturating_sub(offset).div_ceil(RANGE_SIZE);
		let requests = stream::iter((0..count).map(|number| {
			let start = offset + number * RANGE_SIZE;
			let end = start.saturating_add(RANGE_SIZE).min(size) - 1;
			fetch_piece(&source, start, end, size, etag)
		}))
		.buffered(down.parallel);
		tokio::pin!(requests);

		let mut bar =
			ProgressBar::new(flags, cmd, "download", Some(size), ProgressUnit::Bytes, verbose_log);
		let mut completed = offset;
		if completed > 0 && completed == size {
			bar.update(completed);
		}
		while let Some(piece) = requests.next().await {
			let piece = piece?;
			file.write_all(&piece).await?;
			completed += piece.len() as u64;
			bar.update(completed);
		}
		drop(bar);
		file.sync_all().await?;
		drop(file);

		if let Some(expected_hash) = expected_xxh3
			&& !verify_file_xxh3(&partial, expected_hash).await?
		{
			let _ = tokio::fs::remove_file(&partial).await;
			bail!(
				"downloaded file failed XXH3-128 verification (corrupt or stale partial file removed)"
			);
		}

		finish(&partial, &destination).await?;
		download_done(flags, cmd, &display_dest, size.saturating_sub(offset), started);
		return Ok(());
	}

	if exists {
		tokio::fs::remove_file(&partial).await?;
	}
	let response = match body_response {
		Some(resp) => resp,
		None => checked(send(source.request()).await?).await?,
	};
	blank(flags);
	let saved =
		save_stream(flags, cmd, verbose_log, response, &partial, size, expected_xxh3).await?;
	finish(&partial, &destination).await?;
	download_done(flags, cmd, &display_dest, saved, started);
	Ok(())
}

async fn stream_stdout(response: &mut reqwest::Response) -> Result<()> {
	let mut out = tokio::io::stdout();
	while let Some(chunk) = next_chunk(response).await? {
		out.write_all(&chunk).await?;
	}
	out.flush().await?;
	Ok(())
}

fn resolve_filename(source: &Source, disposition: &str) -> String {
	let encoded = disposition
		.split("filename*=UTF-8''")
		.nth(1)
		.and_then(|s| s.split(';').next())
		.and_then(|s| percent_encoding::percent_decode_str(s).decode_utf8().ok())
		.map(std::borrow::Cow::into_owned);
	let from_header = encoded
		.or_else(|| {
			disposition
				.split("filename=\"")
				.nth(1)
				.and_then(|s| s.split('"').next())
				.map(str::to_owned)
		})
		.filter(|s| !s.is_empty());
	let candidate = from_header
		.or_else(|| source.path.as_deref().and_then(|p| p.rsplit('/').next()).map(str::to_owned))
		.unwrap_or_else(|| "download".to_owned());
	let base = candidate.rsplit(['/', '\\']).next().unwrap_or("download");
	if base.is_empty() { "download".to_owned() } else { base.to_owned() }
}

fn flora_xxh3_etag(etag: &str) -> Option<&str> {
	let trimmed = etag.strip_prefix('"')?.strip_suffix('"')?;
	core::hex::is_hex_128(trimmed).then_some(trimmed)
}

async fn verify_file_xxh3(path: &Path, expected: &str) -> Result<bool> {
	Ok(core::hex::hash_file_async(path).await?.eq_ignore_ascii_case(expected))
}

fn download_done(flags: OutputFlags, cmd: &str, destination: &str, saved: u64, started: Instant) {
	done(flags, cmd, "saved", destination);
	let elapsed = started.elapsed();
	row(flags, cmd, "elapsed", &render::elapsed(elapsed), Color::Dim);
	if saved > 0 && elapsed.as_secs_f64() > 0.0 {
		let rate = (saved as f64 / elapsed.as_secs_f64()) as u64;
		row(flags, cmd, "rate", &format!("{}/s", bytes(rate)), Color::Dim);
	}
	tracing::debug!(saved, "download completed");
}

fn partial_path(destination: &Path, name: &str) -> PathBuf {
	const SUFFIX: &str = ".partial";
	if name.len() + SUFFIX.len() <= 255 {
		return destination.with_file_name(format!("{name}{SUFFIX}"));
	}
	let suffix =
		format!("-{}{SUFFIX}", core::hex::u128(xxhash_rust::xxh3::xxh3_128(name.as_bytes())));
	let mut end = 255 - suffix.len();
	while !name.is_char_boundary(end) {
		end -= 1;
	}
	destination.with_file_name(format!("{}{suffix}", &name[..end]))
}

async fn open_partial(path: &Path, exists: bool) -> Result<tokio::fs::File> {
	let mut options = tokio::fs::OpenOptions::new();
	options.append(true);
	if !exists {
		options.create_new(true);
	}
	Ok(options.open(path).await?)
}

async fn fetch_piece(
	source: &Source,
	start: u64,
	end: u64,
	size: u64,
	etag: &str,
) -> Result<Vec<u8>> {
	let mut response = source.range(start, end, size, etag).await?;
	let expected = usize::try_from(end - start + 1)?;
	let mut piece = Vec::with_capacity(expected);
	while let Some(chunk) = next_chunk(&mut response).await? {
		if chunk.len() > expected - piece.len() {
			bail!("server returned too much range data");
		}
		piece.extend_from_slice(&chunk);
	}
	if piece.len() != expected {
		bail!("server returned an incomplete range");
	}
	Ok(piece)
}

async fn save_stream(
	flags: OutputFlags,
	cmd: &str,
	verbose_log: bool,
	mut response: reqwest::Response,
	partial: &Path,
	size: Option<u64>,
	expected_xxh3: Option<&str>,
) -> Result<u64> {
	let mut file = open_partial(partial, false).await?;
	let mut bar = ProgressBar::new(flags, cmd, "download", size, ProgressUnit::Bytes, verbose_log);
	let mut hash = Xxh3::new();
	let mut saved = 0u64;
	while let Some(chunk) = next_chunk(&mut response).await? {
		saved = saved.checked_add(chunk.len() as u64).context("download too large")?;
		if size.is_some_and(|total| saved > total) {
			bail!("server returned too much data");
		}
		hash.update(&chunk);
		file.write_all(&chunk).await?;
		bar.update(saved);
	}
	drop(bar);
	if size.is_some_and(|total| saved != total) {
		bail!("server returned an incomplete download");
	}
	file.sync_all().await?;
	drop(file);
	if let Some(expected) = expected_xxh3
		&& !core::hex::u128(hash.digest128()).eq_ignore_ascii_case(expected)
	{
		let _ = tokio::fs::remove_file(partial).await;
		bail!("downloaded file failed XXH3-128 verification");
	}
	Ok(saved)
}

async fn finish(partial: &Path, destination: &Path) -> Result<()> {
	let from = partial.to_path_buf();
	let to = destination.to_path_buf();
	tokio::task::spawn_blocking(move || core::rename::no_replace(&from, &to))
		.await?
		.with_context(|| format!("cannot save {} without overwriting it", destination.display()))
}
