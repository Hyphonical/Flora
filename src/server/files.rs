//! File listing, pastes, share views, moves, range downloads, and retention maintenance.

use super::{
	App, Result, auth, bad, conflict, db,
	events::{self, TreeOp},
	forbidden, missing, profile, trash, web,
};
use crate::core::{self, Entry, FileRecord, TokenKind, Upload, User};
use axum::{
	Json,
	body::Body,
	extract::{Path, Query, State},
	http::{HeaderMap, HeaderValue, Method, StatusCode, header},
	response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const MAX_PASTE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileQuery {
	#[serde(default)]
	path: String,
	owner: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerQuery {
	owner: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DownloadQuery {
	path: String,
	owner: Option<String>,
	#[serde(default, deserialize_with = "core::query_flag")]
	download: bool,
	#[serde(default, deserialize_with = "core::query_flag")]
	preview: bool,
}

#[derive(Default, Deserialize)]
pub(super) struct ShareQuery {
	#[serde(default, deserialize_with = "core::query_flag")]
	raw: bool,
	#[serde(default, deserialize_with = "core::query_flag")]
	download: bool,
	#[serde(default, deserialize_with = "core::query_flag")]
	preview: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MoveRequest {
	from: String,
	to: String,
	owner: Option<String>,
}

#[derive(Clone, Copy)]
struct DownloadOpts {
	shared: bool,
	attachment: bool,
	track: bool,
}

pub(super) async fn owner(
	app: &App,
	principal: &auth::Principal,
	requested: Option<&str>,
) -> Result<String> {
	let Some(requested) = requested else {
		return Ok(principal.user.id.clone());
	};
	if requested == principal.user.id || requested.eq_ignore_ascii_case(&principal.user.name) {
		return Ok(principal.user.id.clone());
	}
	if !principal.admin() {
		return Err(forbidden());
	}
	if let Some(u) = app.db.get_async::<User>(&db::user(requested)).await? {
		return Ok(u.id);
	}
	if let Some(u) = app.db.find_user_by_name_async(requested).await? {
		return Ok(u.id);
	}
	Err(missing())
}

pub(super) fn allowed(p: &auth::Principal, owner: &str, path: &str, write: bool) -> bool {
	if owner != p.user.id {
		return p.admin();
	}
	if write { p.manage() } else { p.permits(path, false) }
}

pub(super) async fn scoped_records(
	app: &App,
	owner: &str,
	prefix: &str,
) -> Result<Vec<FileRecord>> {
	let store = app.db.clone();
	let owner = owner.to_owned();
	let prefix = prefix.to_owned();
	tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<FileRecord>> {
		let mut records: Vec<FileRecord> = store.list(&db::files_under(&owner, &prefix))?;
		if !prefix.is_empty()
			&& let Some(exact) = store.get::<FileRecord>(&db::file(&owner, &prefix))?
		{
			records.push(exact);
		}
		Ok(records)
	})
	.await?
	.map_err(Into::into)
}

fn wants_html_ui(method: &Method, headers: &HeaderMap, query: &ShareQuery) -> bool {
	if *method != Method::GET || query.raw || query.download || query.preview {
		return false;
	}
	if headers.contains_key(header::RANGE) {
		return false;
	}
	headers
		.get(header::ACCEPT)
		.and_then(|v| v.to_str().ok())
		.is_some_and(|accept| accept.contains("text/html"))
}

pub(super) async fn shared_file(
	State(app): State<App>,
	method: Method,
	Path(token): Path<String>,
	Query(query): Query<ShareQuery>,
	headers: HeaderMap,
) -> Result<Response> {
	if wants_html_ui(&method, &headers, &query) {
		return Ok(web::serve_index());
	}
	let p = auth::authenticate_token(&app, &token).await?;
	if p.token.kind != TokenKind::File && p.token.kind != TokenKind::Folder {
		return Err(forbidden());
	}
	let path = p.token.scope.clone();
	shared_response(app, method, p, headers, path, query).await
}

pub(super) async fn shared_path(
	State(app): State<App>,
	method: Method,
	Path((token, path)): Path<(String, String)>,
	Query(query): Query<ShareQuery>,
	headers: HeaderMap,
) -> Result<Response> {
	if wants_html_ui(&method, &headers, &query) {
		return Ok(web::serve_index());
	}
	let p = auth::authenticate_token(&app, &token).await?;
	if p.token.kind != TokenKind::Folder {
		return Err(forbidden());
	}
	let path = if p.token.scope.is_empty() { path } else { format!("{}/{}", p.token.scope, path) };
	shared_response(app, method, p, headers, path, query).await
}

async fn shared_response(
	app: App,
	method: Method,
	principal: auth::Principal,
	headers: HeaderMap,
	path: String,
	query: ShareQuery,
) -> Result<Response> {
	let allow_root = principal.token.kind == TokenKind::Folder;
	let path = core::clean_path(&path, allow_root).map_err(bad)?;
	if !principal.permits(&path, false) {
		return Err(forbidden());
	}
	let target =
		core::safe_join_async(&app.storage.join(&principal.user.id), &path).await.map_err(bad)?;
	let metadata = tokio::fs::metadata(target).await.map_err(|error| {
		if error.kind() == std::io::ErrorKind::NotFound { missing() } else { error.into() }
	})?;
	if metadata.is_dir() {
		if principal.token.kind != TokenKind::Folder {
			return Err(forbidden());
		}
		Ok(list_for(&app, &principal, &principal.user.id, &path).await?.into_response())
	} else {
		if path.is_empty() {
			return Err(forbidden());
		}
		let opts = DownloadOpts { shared: true, attachment: query.download, track: !query.preview };
		download_for(&app, &method, &principal, &headers, &principal.user.id, &path, opts).await
	}
}

pub(super) async fn share_info_root(
	State(app): State<App>,
	Path(token): Path<String>,
) -> Result<Json<serde_json::Value>> {
	share_info_inner(&app, &token, None).await
}

pub(super) async fn share_info_path(
	State(app): State<App>,
	Path((token, subpath)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
	share_info_inner(&app, &token, Some(subpath)).await
}

async fn share_info_inner(
	app: &App,
	token: &str,
	subpath: Option<String>,
) -> Result<Json<serde_json::Value>> {
	let p = auth::authenticate_token(app, token).await?;
	if p.token.kind != TokenKind::File && p.token.kind != TokenKind::Folder {
		return Err(forbidden());
	}
	if subpath.is_some() && p.token.kind != TokenKind::Folder {
		return Err(forbidden());
	}
	let rel_sub = match subpath {
		Some(s) => core::clean_path(&s, true).map_err(bad)?,
		None => String::new(),
	};
	let full_path = match (p.token.scope.is_empty(), rel_sub.is_empty()) {
		(true, _) => rel_sub.clone(),
		(false, true) => p.token.scope.clone(),
		(false, false) => format!("{}/{rel_sub}", p.token.scope),
	};
	let allow_root = p.token.kind == TokenKind::Folder;
	let full_path = core::clean_path(&full_path, allow_root).map_err(bad)?;
	if !p.permits(&full_path, false) {
		return Err(forbidden());
	}
	let root_name = if p.token.scope.is_empty() {
		p.user.name.clone()
	} else {
		p.token.scope.rsplit('/').next().unwrap_or(&p.token.scope).to_owned()
	};
	let target =
		core::safe_join_async(&app.storage.join(&p.user.id), &full_path).await.map_err(bad)?;
	let metadata = tokio::fs::metadata(target).await.map_err(|error| {
		if error.kind() == std::io::ErrorKind::NotFound { missing() } else { error.into() }
	})?;
	if metadata.is_dir() {
		if p.token.kind != TokenKind::Folder {
			return Err(forbidden());
		}
		let Json(mut entries) = list_for(app, &p, &p.user.id, &full_path).await?;
		if !p.token.scope.is_empty() {
			let scope_prefix = format!("{}/", p.token.scope);
			for item in &mut entries {
				if let Some(stripped) = item.path.strip_prefix(&scope_prefix) {
					item.path = stripped.to_owned();
				}
			}
		}
		Ok(Json(serde_json::json!({
			"kind": p.token.kind,
			"root_name": root_name,
			"subpath": rel_sub,
			"directory": true,
			"entry": null,
			"entries": entries,
		})))
	} else {
		let record: FileRecord =
			app.db.get_async(&db::file(&p.user.id, &full_path)).await?.ok_or_else(missing)?;
		let name = record.path.rsplit('/').next().unwrap_or(&record.path).to_owned();
		let rel_path = if p.token.scope.is_empty() || p.token.kind == TokenKind::File {
			name.clone()
		} else {
			let scope_prefix = format!("{}/", p.token.scope);
			record.path.strip_prefix(&scope_prefix).unwrap_or(&record.path).to_owned()
		};
		let entry = Entry {
			name,
			path: rel_path,
			directory: false,
			size: record.size,
			modified: record.modified,
			hash: Some(record.hash),
			item_count: None,
			shared_token: None,
		};
		Ok(Json(serde_json::json!({
			"kind": p.token.kind,
			"root_name": root_name,
			"subpath": rel_sub,
			"directory": false,
			"entry": entry,
			"entries": [],
		})))
	}
}

pub(super) async fn usage(app: &App, owner: &str) -> Result<u64> {
	let live = app.db.sum_file_sizes_async(owner).await?;
	let personal = profile::load(app, owner).await?;
	let trashed = personal
		.trash
		.values()
		.filter(|entry| entry.state != profile::TrashState::Purging)
		.fold(0u64, |sum, entry| sum.saturating_add(entry.size));
	Ok(live.saturating_add(trashed))
}

pub(super) async fn list(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<FileQuery>,
) -> Result<Json<Vec<Entry>>> {
	let principal = auth::authenticate(&app, &headers).await?;
	let path = core::clean_path(&query.path, true).map_err(bad)?;
	let owner = owner(&app, &principal, query.owner.as_deref()).await?;
	list_for(&app, &principal, &owner, &path).await
}

#[derive(Default)]
struct FolderStats {
	size: u64,
	modified: u64,
	file_count: u64,
}

async fn list_for(
	app: &App,
	principal: &auth::Principal,
	owner: &str,
	path: &str,
) -> Result<Json<Vec<Entry>>> {
	if !allowed(principal, owner, path, false) {
		return Err(forbidden());
	}
	let prefix = if path.is_empty() { String::new() } else { format!("{path}/") };
	let directory = core::safe_join_async(&app.storage.join(owner), path).await.map_err(bad)?;
	let mut children = tokio::fs::read_dir(directory).await.map_err(|error| {
		if error.kind() == std::io::ErrorKind::NotFound { missing() } else { error.into() }
	})?;

	let mut disk_dirs = Vec::new();
	let mut disk_files = std::collections::HashSet::new();
	while let Some(child) = children.next_entry().await? {
		let kind = child.file_type().await?;
		if kind.is_symlink() {
			continue;
		}
		let name = child.file_name().into_string().map_err(|_| bad("entry name is not UTF-8"))?;
		if name.eq_ignore_ascii_case(".flora")
			|| name.eq_ignore_ascii_case(".flora.tmp")
			|| name.eq_ignore_ascii_case(".trash")
		{
			continue;
		}
		let full = format!("{prefix}{name}");
		if !allowed(principal, owner, &full, false) {
			continue;
		}
		if kind.is_dir() {
			disk_dirs.push((name, full));
		} else if kind.is_file() {
			disk_files.insert(name);
		}
	}

	let include_shares = principal.manage();
	let store = app.db.clone();
	let owner_owned = owner.to_owned();
	let path_owned = path.to_owned();
	let prefix_owned = prefix.clone();

	let (records, shares) = tokio::task::spawn_blocking(
		move || -> anyhow::Result<(Vec<FileRecord>, HashMap<String, String>)> {
			let records = store.list::<FileRecord>(&db::files_under(&owner_owned, &path_owned))?;
			let shares =
				if include_shares { store.share_map_sync(&owner_owned)? } else { HashMap::new() };
			Ok((records, shares))
		},
	)
	.await??;

	let mut entries = BTreeMap::<String, Entry>::new();
	let mut folder_stats = HashMap::<String, FolderStats>::new();

	for record in records {
		let Some(rel) = record.path.strip_prefix(&prefix_owned) else {
			continue;
		};
		if let Some((dir_name, _)) = rel.split_once('/') {
			let stats = folder_stats.entry(dir_name.to_owned()).or_default();
			stats.size = stats.size.saturating_add(record.size);
			stats.modified = stats.modified.max(record.modified);
			stats.file_count = stats.file_count.saturating_add(1);
		} else if disk_files.contains(rel) && allowed(principal, owner, &record.path, false) {
			let shared_token = shares.get(&record.path).cloned();
			entries.insert(
				rel.to_owned(),
				Entry {
					name: rel.to_owned(),
					path: record.path,
					directory: false,
					size: record.size,
					modified: record.modified,
					hash: Some(record.hash),
					item_count: None,
					shared_token,
				},
			);
		}
	}

	for (name, full) in disk_dirs {
		let stats = folder_stats.remove(&name).unwrap_or_default();
		let shared_token = shares.get(&full).cloned();
		entries.insert(
			name.clone(),
			Entry {
				name,
				path: full,
				directory: true,
				size: stats.size,
				modified: stats.modified,
				hash: None,
				item_count: Some(stats.file_count),
				shared_token,
			},
		);
	}

	Ok(Json(entries.into_values().collect()))
}

fn is_text_path(path: &str) -> bool {
	let filename = path.rsplit('/').next().unwrap_or(path);
	let lower = filename.to_ascii_lowercase();
	if matches!(
		lower.as_str(),
		"dockerfile"
			| "makefile"
			| "justfile"
			| "license"
			| "readme"
			| ".gitignore"
			| ".gitattributes"
			| ".editorconfig"
			| ".dockerignore"
			| ".env"
	) {
		return true;
	}
	let Some((_, ext)) = lower.rsplit_once('.') else {
		return false;
	};
	matches!(
		ext,
		"txt"
			| "md"
			| "markdown"
			| "rs"
			| "ts"
			| "tsx"
			| "js"
			| "jsx"
			| "mjs"
			| "cjs"
			| "svelte"
			| "vue"
			| "html"
			| "htm"
			| "css"
			| "scss"
			| "json"
			| "jsonc"
			| "jsonl"
			| "toml"
			| "yaml"
			| "yml"
			| "ini"
			| "conf"
			| "cfg"
			| "env"
			| "sh"
			| "bash"
			| "zsh"
			| "fish"
			| "ps1"
			| "bat"
			| "cmd"
			| "py"
			| "go"
			| "c"
			| "h"
			| "cpp"
			| "hpp"
			| "cs"
			| "java"
			| "kt"
			| "swift"
			| "rb"
			| "php"
			| "lua"
			| "sql"
			| "graphql"
			| "xml"
			| "csv"
			| "tsv"
			| "log"
			| "diff"
			| "patch"
			| "nix"
			| "zig"
	)
}

pub(super) async fn pastes(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<OwnerQuery>,
) -> Result<Json<Vec<Entry>>> {
	let principal = auth::authenticate(&app, &headers).await?;
	let owner = owner(&app, &principal, query.owner.as_deref()).await?;
	if !allowed(&principal, &owner, "", false) {
		return Err(forbidden());
	}
	let include_shares = principal.manage();
	let store = app.db.clone();
	let owner_owned = owner.clone();
	let (records, shares) = tokio::task::spawn_blocking(
		move || -> anyhow::Result<(Vec<FileRecord>, HashMap<String, String>)> {
			let records = store.list::<FileRecord>(&db::files(&owner_owned))?;
			let shares =
				if include_shares { store.share_map_sync(&owner_owned)? } else { HashMap::new() };
			Ok((records, shares))
		},
	)
	.await??;

	let mut out: Vec<Entry> = records
		.into_iter()
		.filter(|r| {
			r.size <= MAX_PASTE_BYTES
				&& is_text_path(&r.path)
				&& allowed(&principal, &owner, &r.path, false)
		})
		.map(|r| {
			let name = r.path.rsplit('/').next().unwrap_or(&r.path).to_owned();
			let shared_token = shares.get(&r.path).cloned();
			Entry {
				name,
				path: r.path,
				directory: false,
				size: r.size,
				modified: r.modified,
				hash: Some(r.hash),
				item_count: None,
				shared_token,
			}
		})
		.collect();
	out.sort_by_key(|a| std::cmp::Reverse(a.modified));
	Ok(Json(out))
}

pub(super) async fn remove(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<FileQuery>,
) -> Result<StatusCode> {
	let p = auth::authenticate(&app, &headers).await?;
	let path = core::clean_path(&query.path, false).map_err(bad)?;
	let owner = owner(&app, &p, query.owner.as_deref()).await?;
	if !allowed(&p, &owner, &path, true) {
		return Err(forbidden());
	}
	let _guard = app.user_locks.lock(&owner).await;
	let source = core::safe_join_async(&app.storage.join(&owner), &path).await.map_err(bad)?;
	if !tokio::fs::try_exists(source).await? {
		return Err(missing());
	}
	trash::delete(&app, &owner, &path).await?;
	app.events.emit_tree(&owner, TreeOp::Rm, vec![path]);
	Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn move_file(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<MoveRequest>,
) -> Result<StatusCode> {
	let p = auth::authenticate(&app, &headers).await?;
	let from = core::clean_path(&req.from, false).map_err(bad)?;
	let to = core::clean_path(&req.to, false).map_err(bad)?;
	let owner = owner(&app, &p, req.owner.as_deref()).await?;
	if !allowed(&p, &owner, &from, true) || !allowed(&p, &owner, &to, true) {
		return Err(forbidden());
	}
	if from == to || core::scoped(&from, &to) {
		return Err(bad("destination is inside source"));
	}
	let _guard = app.user_locks.lock(&owner).await;
	let active_uploads: Vec<Upload> = app.db.list_async("upload/").await?;
	if active_uploads.iter().any(|u| {
		u.owner == owner
			&& (core::scoped(&from, &u.path)
				|| core::scoped(&u.path, &from)
				|| core::scoped(&to, &u.path)
				|| core::scoped(&u.path, &to))
	}) {
		return Err(conflict("an active upload is using the source or destination path"));
	}
	let selected = scoped_records(&app, &owner, &from).await?;
	if !scoped_records(&app, &owner, &to).await?.is_empty() {
		return Err(conflict("destination already exists"));
	}
	let mut ancestor = to.as_str();
	while let Some((parent, _)) = ancestor.rsplit_once('/') {
		if app.db.get_async::<FileRecord>(&db::file(&owner, parent)).await?.is_some() {
			return Err(conflict("a parent of destination is a file"));
		}
		ancestor = parent;
	}
	let root = app.storage.join(&owner);
	let source = core::safe_join_async(&root, &from).await.map_err(bad)?;
	let dest = core::safe_join_async(&root, &to).await.map_err(bad)?;
	if !tokio::fs::try_exists(&source).await? {
		return Err(missing());
	}
	if tokio::fs::try_exists(&dest).await? {
		return Err(conflict("destination already exists"));
	}
	if let Some(parent) = dest.parent() {
		tokio::fs::create_dir_all(parent).await?;
	}
	tokio::task::spawn_blocking(move || core::rename::no_replace(&source, &dest)).await??;
	let mut deletes = Vec::with_capacity(selected.len());
	let mut puts = Vec::with_capacity(selected.len());
	for mut record in selected {
		deletes.push(db::file(&owner, &record.path));
		record.path = format!("{to}{}", &record.path[from.len()..]);
		puts.push((db::file(&owner, &record.path), record));
	}
	app.db.write_batch_async(&puts, &deletes).await?;
	app.events.emit_tree(&owner, TreeOp::Mv, vec![from, to]);
	Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateFolder {
	path: String,
	owner: Option<String>,
}

pub(super) async fn create_folder(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<CreateFolder>,
) -> Result<StatusCode> {
	let p = auth::authenticate(&app, &headers).await?;
	let path = core::clean_path(&req.path, false).map_err(bad)?;
	let owner = owner(&app, &p, req.owner.as_deref()).await?;
	if !allowed(&p, &owner, &path, true) {
		return Err(forbidden());
	}
	let _guard = app.user_locks.lock(&owner).await;
	let root = app.storage.join(&owner);
	let target = core::safe_join_async(&root, &path).await.map_err(bad)?;
	if tokio::fs::try_exists(&target).await? {
		return Err(conflict("destination already exists"));
	}
	let mut parent = target.parent();
	while let Some(dir) = parent {
		if dir == root {
			break;
		}
		if let Ok(meta) = tokio::fs::metadata(dir).await
			&& !meta.is_dir()
		{
			return Err(conflict("a parent path is a file"));
		}
		parent = dir.parent();
	}
	tokio::fs::create_dir_all(target).await?;
	app.events.emit_tree(&owner, TreeOp::Mkdir, vec![path]);
	Ok(StatusCode::CREATED)
}

pub(super) async fn download(
	State(app): State<App>,
	method: Method,
	headers: HeaderMap,
	Query(query): Query<DownloadQuery>,
) -> Result<Response> {
	let p = auth::authenticate(&app, &headers).await?;
	let path = core::clean_path(&query.path, false).map_err(bad)?;
	let owner = owner(&app, &p, query.owner.as_deref()).await?;
	let opts = DownloadOpts { shared: false, attachment: query.download, track: !query.preview };
	download_for(&app, &method, &p, &headers, &owner, &path, opts).await
}

async fn download_for(
	app: &App,
	method: &Method,
	p: &auth::Principal,
	headers: &HeaderMap,
	owner: &str,
	path: &str,
	opts: DownloadOpts,
) -> Result<Response> {
	if !allowed(p, owner, path, false) {
		return Err(forbidden());
	}
	let record: FileRecord = app.db.get_async(&db::file(owner, path)).await?.ok_or_else(missing)?;
	let mut start = 0u64;
	let mut end = record.size.saturating_sub(1);
	let mut partial = false;
	let etag = format!("\"{}\"", record.hash);
	let range_matches =
		headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()).is_none_or(|v| v == etag);
	if let Some(range) = headers.get(header::RANGE).filter(|_| range_matches) {
		let text = range.to_str().map_err(|_| bad("invalid range"))?;
		let range = text.strip_prefix("bytes=").ok_or_else(|| bad("invalid range"))?;
		let (a, b) = range.split_once('-').ok_or_else(|| bad("invalid range"))?;
		if record.size == 0 || b.contains(',') {
			return Err(super::Error(
				StatusCode::RANGE_NOT_SATISFIABLE,
				anyhow::anyhow!("range not satisfiable"),
			));
		}
		if a.is_empty() {
			let suffix: u64 = b.parse().map_err(|_| bad("invalid range"))?;
			if suffix == 0 {
				return Err(bad("invalid range"));
			}
			start = record.size.saturating_sub(suffix);
		} else {
			start = a.parse().map_err(|_| bad("invalid range"))?;
			if !b.is_empty() {
				end = b.parse::<u64>().map_err(|_| bad("invalid range"))?.min(end);
			}
		}
		if start > end || start >= record.size {
			return Err(super::Error(
				StatusCode::RANGE_NOT_SATISFIABLE,
				anyhow::anyhow!("range not satisfiable"),
			));
		}
		partial = true;
	}
	let length = if record.size == 0 { 0 } else { end - start + 1 };
	let file_path = core::safe_join_async(&app.storage.join(owner), path).await.map_err(bad)?;
	let body = if *method == Method::HEAD {
		if !tokio::fs::try_exists(&file_path).await? {
			return Err(missing());
		}
		Body::empty()
	} else {
		let mut file = tokio::fs::File::open(file_path).await.map_err(|error| {
			if error.kind() == std::io::ErrorKind::NotFound { missing() } else { error.into() }
		})?;
		file.seek(std::io::SeekFrom::Start(start)).await?;
		let reader_stream = ReaderStream::new(file.take(length));
		if opts.track && length > 0 {
			let source = events::detect_source(headers, opts.shared);
			app.events.wrap_download_stream(
				owner,
				path,
				source,
				record.size,
				length,
				headers,
				reader_stream,
			)
		} else {
			Body::from_stream(reader_stream)
		}
	};
	let mut response = Response::builder()
		.status(if partial { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
		.header(header::ACCEPT_RANGES, "bytes")
		.header(header::CONTENT_LENGTH, length.to_string())
		.header(header::ETAG, etag)
		.header(header::CONTENT_TYPE, mime_guess::from_path(path).first_or_octet_stream().as_ref());
	if partial {
		response =
			response.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{}", record.size));
	}
	let name = path.rsplit('/').next().unwrap_or("download");
	let ascii: String = name
		.chars()
		.map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' { c } else { '_' })
		.collect();
	let encoded = percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC);
	let disp_kind = if opts.attachment { "attachment" } else { "inline" };
	if let Ok(value) = HeaderValue::from_str(&format!(
		"{disp_kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"
	)) {
		response = response.header(header::CONTENT_DISPOSITION, value);
	}
	response.body(body).map_err(Into::into)
}

pub(super) async fn maintenance(app: &App) -> anyhow::Result<()> {
	app.db.prune_prefix_keep_newest_async("activity/", 5_000).await?;
	let now = core::now();
	let uploads: Vec<Upload> = app.db.list_async("upload/").await?;
	for upload in uploads {
		if !upload.copying
			&& app.buffer_retention.is_some_and(|age| now.saturating_sub(upload.updated) >= age)
		{
			let locks = app.transfer_locks.get(&upload.id);
			let Ok(_upload_guard) = locks.gate.try_write() else {
				continue;
			};
			let Some(current) = app.db.get_async::<Upload>(&db::upload(&upload.id)).await? else {
				app.transfer_locks.remove(&upload.id);
				continue;
			};
			if current.copying
				|| !app
					.buffer_retention
					.is_some_and(|age| now.saturating_sub(current.updated) >= age)
			{
				continue;
			}
			app.db.delete_async(&db::upload(&upload.id)).await?;
			super::uploads::remove_upload_artifacts(app, &upload.owner, &upload.id).await?;
			app.transfer_locks.remove(&upload.id);
			app.events.upload_cancelled(&upload.owner, &upload.id);
		}
	}
	let users: Vec<User> = app.db.list_async("user/").await?;
	for user in users {
		let _guard = app.user_locks.lock(&user.id).await;
		trash::expire(app, &user, now).await.map_err(|error| error.1)?;
	}
	Ok(())
}
