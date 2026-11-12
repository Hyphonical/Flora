//! Live transfer tracking and Server-Sent Events (`/api/events`, `/api/transfers`).

use super::{App, Result, auth, forbidden};
use crate::core::{self, Entry, TransferSource, Upload};
use axum::{
	Json,
	body::Body,
	extract::{Query, State},
	http::{HeaderMap, header},
	response::Response,
};
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
	collections::HashMap,
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};
use tokio::sync::broadcast;
use uuid::Uuid;

const BROADCAST_CAPACITY: usize = 512;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(20);
const DOWNLOAD_IDLE_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum TransferDirection {
	Upload,
	Download,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum TransferPhase {
	Transferring,
	Verifying,
	Storing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum TreeOp {
	Mkdir,
	Mv,
	Rm,
	Restore,
	Purge,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TransferSnapshot {
	pub id: String,
	pub owner: String,
	pub direction: TransferDirection,
	pub source: TransferSource,
	pub path: String,
	pub total_bytes: Option<u64>,
	pub transferred_bytes: u64,
	pub speed_bps: u64,
	pub phase: TransferPhase,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub chunk_size: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub chunks_done: Option<usize>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub chunks_total: Option<usize>,
	pub started_at: u64,
	pub updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub(super) enum EventPayload {
	#[serde(rename = "transfer:started")]
	TransferStarted { transfer: TransferSnapshot },
	#[serde(rename = "transfer:progress")]
	TransferProgress {
		id: String,
		owner: String,
		direction: TransferDirection,
		phase: TransferPhase,
		transferred_bytes: u64,
		total_bytes: Option<u64>,
		speed_bps: u64,
		#[serde(skip_serializing_if = "Option::is_none")]
		chunk_index: Option<usize>,
		#[serde(skip_serializing_if = "Option::is_none")]
		chunks_done: Option<usize>,
		#[serde(skip_serializing_if = "Option::is_none")]
		chunks_total: Option<usize>,
	},
	#[serde(rename = "transfer:completed")]
	TransferCompleted {
		id: String,
		owner: String,
		direction: TransferDirection,
		path: String,
		size: u64,
		#[serde(skip_serializing_if = "Option::is_none")]
		entry: Option<Entry>,
	},
	#[serde(rename = "transfer:cancelled")]
	TransferCancelled { id: String, owner: String, direction: TransferDirection },
	#[serde(rename = "tree:changed")]
	TreeChanged { owner: String, op: TreeOp, paths: Vec<String> },
}

#[derive(Clone, Debug)]
struct ScopedEvent {
	owner: String,
	payload: EventPayload,
}

struct LiveUpload {
	started_instant: Instant,
	last_emit: Instant,
	committed_bytes: u64,
	session_bytes: u64,
	inflight_chunks: HashMap<usize, u64>,
	phase: TransferPhase,
}

struct LiveDownload {
	id: String,
	owner: String,
	path: String,
	source: TransferSource,
	total_bytes: u64,
	base_offset: u64,
	expected_session_bytes: u64,
	session_bytes: u64,
	active_streams: usize,
	generation: u64,
	started_at: u64,
	updated_at: u64,
	started_instant: Instant,
	last_emit: Instant,
}

pub(crate) struct Hub {
	tx: broadcast::Sender<ScopedEvent>,
	uploads: Mutex<HashMap<String, LiveUpload>>,
	downloads: Mutex<HashMap<String, LiveDownload>>,
}

impl Default for Hub {
	fn default() -> Self {
		let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
		Self { tx, uploads: Mutex::new(HashMap::new()), downloads: Mutex::new(HashMap::new()) }
	}
}

fn calc_speed(session_bytes: u64, elapsed: Duration) -> u64 {
	if session_bytes == 0 {
		return 0;
	}
	let secs = elapsed.as_secs_f64().max(0.05);
	(session_bytes as f64 / secs) as u64
}

fn completed_upload_stats(upload: &Upload) -> (u64, usize, Option<usize>) {
	let mut bytes = 0u64;
	let mut done = 0usize;
	for slot in upload.chunks.iter().flatten() {
		if !slot.hash.is_empty() {
			bytes = bytes.saturating_add(slot.size);
			done += 1;
		}
	}
	let total =
		upload.size.map(|sz| if sz == 0 { 0 } else { sz.div_ceil(upload.chunk_size) as usize });
	(bytes, done, total)
}

pub(super) fn detect_source(headers: &HeaderMap, shared: bool) -> TransferSource {
	let is_cli = headers
		.get("x-flora-source")
		.and_then(|v| v.to_str().ok())
		.is_some_and(|s| s.eq_ignore_ascii_case("cli"))
		|| headers
			.get(header::USER_AGENT)
			.and_then(|v| v.to_str().ok())
			.is_some_and(|ua| ua.starts_with("Flora/"));
	if is_cli {
		TransferSource::Cli
	} else if shared {
		TransferSource::Share
	} else {
		TransferSource::Web
	}
}

impl Hub {
	fn send(&self, owner: &str, payload: EventPayload) {
		let _ = self.tx.send(ScopedEvent { owner: owner.to_owned(), payload });
	}

	pub(super) fn emit_tree(&self, owner: &str, op: TreeOp, paths: Vec<String>) {
		self.send(owner, EventPayload::TreeChanged { owner: owner.to_owned(), op, paths });
	}

	pub(super) fn upload_started(&self, upload: &Upload, chunked: bool) {
		let (committed_bytes, chunks_done, chunks_total) = completed_upload_stats(upload);
		let now_inst = Instant::now();
		let phase =
			if upload.copying { TransferPhase::Storing } else { TransferPhase::Transferring };
		if let Ok(mut guard) = self.uploads.lock() {
			guard.entry(upload.id.clone()).or_insert_with(|| LiveUpload {
				started_instant: now_inst,
				last_emit: now_inst.checked_sub(PROGRESS_INTERVAL).unwrap_or(now_inst),
				committed_bytes,
				session_bytes: 0,
				inflight_chunks: HashMap::new(),
				phase,
			});
		}
		let snapshot = TransferSnapshot {
			id: upload.id.clone(),
			owner: upload.owner.clone(),
			direction: TransferDirection::Upload,
			source: upload.source,
			path: upload.path.clone(),
			total_bytes: upload.size,
			transferred_bytes: committed_bytes,
			speed_bps: 0,
			phase,
			chunk_size: chunked.then_some(upload.chunk_size),
			chunks_done: chunked.then_some(chunks_done),
			chunks_total: if chunked { chunks_total } else { None },
			started_at: if upload.created > 0 { upload.created } else { upload.updated },
			updated_at: upload.updated,
		};
		self.send(&upload.owner, EventPayload::TransferStarted { transfer: snapshot });
	}

	pub(super) fn upload_chunk_inflight(
		&self,
		upload: &Upload,
		chunk_index: usize,
		chunk_bytes_so_far: u64,
		delta_bytes: u64,
	) {
		let Ok(mut guard) = self.uploads.lock() else {
			return;
		};
		let now = Instant::now();
		let entry = guard.entry(upload.id.clone()).or_insert_with(|| {
			let (committed, _, _) = completed_upload_stats(upload);
			LiveUpload {
				started_instant: now,
				last_emit: now.checked_sub(PROGRESS_INTERVAL).unwrap_or(now),
				committed_bytes: committed,
				session_bytes: 0,
				inflight_chunks: HashMap::new(),
				phase: TransferPhase::Transferring,
			}
		});
		entry.session_bytes = entry.session_bytes.saturating_add(delta_bytes);
		entry.inflight_chunks.insert(chunk_index, chunk_bytes_so_far);
		if now.duration_since(entry.last_emit) < PROGRESS_INTERVAL {
			return;
		}
		entry.last_emit = now;
		let inflight_sum: u64 = entry.inflight_chunks.values().copied().sum();
		let transferred = entry.committed_bytes.saturating_add(inflight_sum);
		let speed = calc_speed(entry.session_bytes, now.duration_since(entry.started_instant));
		let (_, chunks_done, chunks_total) = completed_upload_stats(upload);
		let payload = EventPayload::TransferProgress {
			id: upload.id.clone(),
			owner: upload.owner.clone(),
			direction: TransferDirection::Upload,
			phase: entry.phase,
			transferred_bytes: upload.size.map_or(transferred, |sz| transferred.min(sz)),
			total_bytes: upload.size,
			speed_bps: speed,
			chunk_index: None,
			chunks_done: Some(chunks_done),
			chunks_total,
		};
		drop(guard);
		self.send(&upload.owner, payload);
	}

	pub(super) fn upload_chunk_done(&self, upload: &Upload, chunk_index: usize) {
		let (committed_bytes, chunks_done, chunks_total) = completed_upload_stats(upload);
		let (transferred_bytes, speed_bps) = if let Ok(mut guard) = self.uploads.lock() {
			let now = Instant::now();
			let entry = guard.entry(upload.id.clone()).or_insert_with(|| LiveUpload {
				started_instant: now,
				last_emit: now,
				committed_bytes,
				session_bytes: 0,
				inflight_chunks: HashMap::new(),
				phase: TransferPhase::Transferring,
			});
			entry.inflight_chunks.remove(&chunk_index);
			entry.committed_bytes = committed_bytes;
			entry.last_emit = now;
			let inflight_sum: u64 = entry.inflight_chunks.values().copied().sum();
			let total_now = committed_bytes.saturating_add(inflight_sum);
			let speed = calc_speed(entry.session_bytes, now.duration_since(entry.started_instant));
			(upload.size.map_or(total_now, |sz| total_now.min(sz)), speed)
		} else {
			(committed_bytes, 0)
		};
		self.send(
			&upload.owner,
			EventPayload::TransferProgress {
				id: upload.id.clone(),
				owner: upload.owner.clone(),
				direction: TransferDirection::Upload,
				phase: TransferPhase::Transferring,
				transferred_bytes,
				total_bytes: upload.size,
				speed_bps,
				chunk_index: Some(chunk_index),
				chunks_done: Some(chunks_done),
				chunks_total,
			},
		);
	}

	pub(super) fn upload_stream_progress(
		&self,
		upload: &Upload,
		transferred_bytes: u64,
		delta_bytes: u64,
	) {
		let Ok(mut guard) = self.uploads.lock() else {
			return;
		};
		let now = Instant::now();
		let entry = guard.entry(upload.id.clone()).or_insert_with(|| LiveUpload {
			started_instant: now,
			last_emit: now.checked_sub(PROGRESS_INTERVAL).unwrap_or(now),
			committed_bytes: 0,
			session_bytes: 0,
			inflight_chunks: HashMap::new(),
			phase: TransferPhase::Transferring,
		});
		entry.committed_bytes = transferred_bytes;
		entry.session_bytes = entry.session_bytes.saturating_add(delta_bytes);
		if now.duration_since(entry.last_emit) < PROGRESS_INTERVAL {
			return;
		}
		entry.last_emit = now;
		let speed = calc_speed(entry.session_bytes, now.duration_since(entry.started_instant));
		let payload = EventPayload::TransferProgress {
			id: upload.id.clone(),
			owner: upload.owner.clone(),
			direction: TransferDirection::Upload,
			phase: entry.phase,
			transferred_bytes,
			total_bytes: upload.size,
			speed_bps: speed,
			chunk_index: None,
			chunks_done: None,
			chunks_total: None,
		};
		drop(guard);
		self.send(&upload.owner, payload);
	}

	pub(super) fn upload_phase(&self, upload: &Upload, phase: TransferPhase) {
		let (committed_bytes, chunks_done, chunks_total) = completed_upload_stats(upload);
		let transferred = upload.size.unwrap_or(committed_bytes).max(committed_bytes);
		let speed = if let Ok(mut guard) = self.uploads.lock() {
			if let Some(entry) = guard.get_mut(&upload.id) {
				entry.phase = phase;
				entry.committed_bytes = transferred;
				calc_speed(entry.session_bytes, entry.started_instant.elapsed())
			} else {
				0
			}
		} else {
			0
		};
		self.send(
			&upload.owner,
			EventPayload::TransferProgress {
				id: upload.id.clone(),
				owner: upload.owner.clone(),
				direction: TransferDirection::Upload,
				phase,
				transferred_bytes: transferred,
				total_bytes: upload.size,
				speed_bps: speed,
				chunk_index: None,
				chunks_done: (!upload.chunks.is_empty()).then_some(chunks_done),
				chunks_total: if upload.chunks.is_empty() { None } else { chunks_total },
			},
		);
	}

	pub(super) fn upload_completed(&self, upload: &Upload, record: &core::FileRecord) {
		if let Ok(mut guard) = self.uploads.lock() {
			guard.remove(&upload.id);
		}
		let name = record.path.rsplit('/').next().unwrap_or(&record.path).to_owned();
		let entry = Entry {
			name,
			path: record.path.clone(),
			directory: false,
			size: record.size,
			modified: record.modified,
			hash: Some(record.hash.clone()),
			item_count: None,
			shared_token: None,
		};
		self.send(
			&upload.owner,
			EventPayload::TransferCompleted {
				id: upload.id.clone(),
				owner: upload.owner.clone(),
				direction: TransferDirection::Upload,
				path: record.path.clone(),
				size: record.size,
				entry: Some(entry),
			},
		);
	}

	pub(super) fn upload_cancelled(&self, owner: &str, id: &str) {
		if let Ok(mut guard) = self.uploads.lock() {
			guard.remove(id);
		}
		self.send(
			owner,
			EventPayload::TransferCancelled {
				id: id.to_owned(),
				owner: owner.to_owned(),
				direction: TransferDirection::Upload,
			},
		);
	}

	#[allow(clippy::too_many_arguments)]
	pub(super) fn wrap_download_stream<S>(
		self: &Arc<Self>,
		owner: &str,
		path: &str,
		source: TransferSource,
		file_size: u64,
		range_len: u64,
		headers: &HeaderMap,
		stream: S,
	) -> Body
	where
		S: Stream<Item = std::io::Result<bytes::Bytes>> + Send + 'static,
	{
		let client_tid = headers
			.get("x-transfer-id")
			.and_then(|v| v.to_str().ok())
			.filter(|id| Uuid::parse_str(id).is_ok());
		let declared_total = headers
			.get("x-transfer-total")
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.parse::<u64>().ok())
			.filter(|&n| n > 0 && n <= file_size);
		let declared_offset = headers
			.get("x-transfer-offset")
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.parse::<u64>().ok())
			.unwrap_or(0)
			.min(file_size);

		let id = client_tid.map_or_else(|| Uuid::new_v4().to_string(), str::to_owned);
		let key = format!("{owner}:{id}");
		let now_sec = core::now();
		let now_inst = Instant::now();

		let mut started_snapshot = None;
		if let Ok(mut guard) = self.downloads.lock() {
			if let Some(existing) = guard.get_mut(&key) {
				existing.active_streams += 1;
				existing.generation = existing.generation.wrapping_add(1);
				existing.updated_at = now_sec;
				if declared_total.is_none() {
					existing.expected_session_bytes =
						existing.expected_session_bytes.saturating_add(range_len).min(file_size);
				}
			} else {
				let expected_session = declared_total.unwrap_or(range_len);
				let dl = LiveDownload {
					id: id.clone(),
					owner: owner.to_owned(),
					path: path.to_owned(),
					source,
					total_bytes: file_size,
					base_offset: declared_offset,
					expected_session_bytes: expected_session,
					session_bytes: 0,
					active_streams: 1,
					generation: 1,
					started_at: now_sec,
					updated_at: now_sec,
					started_instant: now_inst,
					last_emit: now_inst.checked_sub(PROGRESS_INTERVAL).unwrap_or(now_inst),
				};
				started_snapshot = Some(TransferSnapshot {
					id: dl.id.clone(),
					owner: dl.owner.clone(),
					direction: TransferDirection::Download,
					source: dl.source,
					path: dl.path.clone(),
					total_bytes: Some(dl.total_bytes),
					transferred_bytes: dl.base_offset,
					speed_bps: 0,
					phase: TransferPhase::Transferring,
					chunk_size: None,
					chunks_done: None,
					chunks_total: None,
					started_at: now_sec,
					updated_at: now_sec,
				});
				guard.insert(key.clone(), dl);
			}
		}
		if let Some(transfer) = started_snapshot {
			self.send(owner, EventPayload::TransferStarted { transfer });
		}

		let guard_handle =
			DownloadStreamGuard { hub: Arc::clone(self), key, grouped: client_tid.is_some() };
		let pinned: std::pin::Pin<Box<dyn Stream<Item = std::io::Result<bytes::Bytes>> + Send>> =
			Box::pin(stream);
		let wrapped = futures_util::stream::unfold(
			(pinned, guard_handle),
			|(mut inner, tracker)| async move {
				let item = inner.next().await?;
				if let Ok(chunk) = &item {
					tracker.record_bytes(chunk.len() as u64);
				}
				Some((item, (inner, tracker)))
			},
		);
		Body::from_stream(wrapped)
	}

	async fn snapshots(
		&self,
		app: &App,
		owner_filter: Option<&str>,
	) -> Result<Vec<TransferSnapshot>> {
		let persisted: Vec<Upload> = app.db.list_async("upload/").await?;
		let mut out = Vec::new();
		for upload in persisted {
			if owner_filter.is_some_and(|wanted| upload.owner != wanted) {
				continue;
			}
			let (committed, chunks_done, chunks_total) = completed_upload_stats(&upload);
			let (transferred_bytes, speed_bps, phase) = if let Ok(guard) = self.uploads.lock()
				&& let Some(live) = guard.get(&upload.id)
			{
				let inflight: u64 = live.inflight_chunks.values().copied().sum();
				let raw = live.committed_bytes.max(committed).saturating_add(inflight);
				let clamped = upload.size.map_or(raw, |sz| raw.min(sz));
				let speed = calc_speed(live.session_bytes, live.started_instant.elapsed());
				(clamped, speed, live.phase)
			} else {
				let phase = if upload.copying {
					TransferPhase::Storing
				} else {
					TransferPhase::Transferring
				};
				(committed, 0, phase)
			};
			let chunked = !upload.chunks.is_empty() || upload.size.is_some();
			out.push(TransferSnapshot {
				id: upload.id,
				owner: upload.owner,
				direction: TransferDirection::Upload,
				source: upload.source,
				path: upload.path,
				total_bytes: upload.size,
				transferred_bytes,
				speed_bps,
				phase,
				chunk_size: chunked.then_some(upload.chunk_size),
				chunks_done: chunked.then_some(chunks_done),
				chunks_total: if chunked { chunks_total } else { None },
				started_at: if upload.created > 0 { upload.created } else { upload.updated },
				updated_at: upload.updated,
			});
		}

		if let Ok(guard) = self.downloads.lock() {
			for dl in guard.values() {
				if owner_filter.is_some_and(|wanted| dl.owner != wanted) {
					continue;
				}
				let transferred =
					dl.base_offset.saturating_add(dl.session_bytes).min(dl.total_bytes);
				let speed = calc_speed(dl.session_bytes, dl.started_instant.elapsed());
				out.push(TransferSnapshot {
					id: dl.id.clone(),
					owner: dl.owner.clone(),
					direction: TransferDirection::Download,
					source: dl.source,
					path: dl.path.clone(),
					total_bytes: Some(dl.total_bytes),
					transferred_bytes: transferred,
					speed_bps: speed,
					phase: TransferPhase::Transferring,
					chunk_size: None,
					chunks_done: None,
					chunks_total: None,
					started_at: dl.started_at,
					updated_at: dl.updated_at,
				});
			}
		}

		out.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
		Ok(out)
	}
}

struct DownloadStreamGuard {
	hub: Arc<Hub>,
	key: String,
	grouped: bool,
}

impl DownloadStreamGuard {
	fn record_bytes(&self, len: u64) {
		if len == 0 {
			return;
		}
		let Ok(mut guard) = self.hub.downloads.lock() else {
			return;
		};
		let Some(dl) = guard.get_mut(&self.key) else {
			return;
		};
		dl.session_bytes = dl.session_bytes.saturating_add(len);
		dl.updated_at = core::now();
		let now = Instant::now();
		if now.duration_since(dl.last_emit) < PROGRESS_INTERVAL {
			return;
		}
		dl.last_emit = now;
		let transferred = dl.base_offset.saturating_add(dl.session_bytes).min(dl.total_bytes);
		let speed = calc_speed(dl.session_bytes, now.duration_since(dl.started_instant));
		let owner = dl.owner.clone();
		let payload = EventPayload::TransferProgress {
			id: dl.id.clone(),
			owner: owner.clone(),
			direction: TransferDirection::Download,
			phase: TransferPhase::Transferring,
			transferred_bytes: transferred,
			total_bytes: Some(dl.total_bytes),
			speed_bps: speed,
			chunk_index: None,
			chunks_done: None,
			chunks_total: None,
		};
		drop(guard);
		self.hub.send(&owner, payload);
	}
}

impl Drop for DownloadStreamGuard {
	fn drop(&mut self) {
		let mut immediate_event = None;
		let mut deferred_check = None;

		if let Ok(mut guard) = self.hub.downloads.lock()
			&& let Some(dl) = guard.get_mut(&self.key)
		{
			dl.active_streams = dl.active_streams.saturating_sub(1);
			if dl.active_streams == 0 {
				if dl.session_bytes >= dl.expected_session_bytes {
					if let Some(done) = guard.remove(&self.key) {
						immediate_event = Some((
							done.owner.clone(),
							EventPayload::TransferCompleted {
								id: done.id,
								owner: done.owner,
								direction: TransferDirection::Download,
								path: done.path,
								size: done.total_bytes,
								entry: None,
							},
						));
					}
				} else if self.grouped {
					deferred_check = Some(dl.generation);
				} else if let Some(cancelled) = guard.remove(&self.key) {
					immediate_event = Some((
						cancelled.owner.clone(),
						EventPayload::TransferCancelled {
							id: cancelled.id,
							owner: cancelled.owner,
							direction: TransferDirection::Download,
						},
					));
				}
			}
		}

		if let Some((owner, payload)) = immediate_event {
			self.hub.send(&owner, payload);
		} else if let Some(expected_gen) = deferred_check {
			let hub = Arc::clone(&self.hub);
			let key = self.key.clone();
			tokio::spawn(async move {
				tokio::time::sleep(DOWNLOAD_IDLE_GRACE).await;
				let mut to_emit = None;
				if let Ok(mut guard) = hub.downloads.lock()
					&& let Some(dl) = guard.get(&key)
					&& dl.active_streams == 0
					&& dl.generation == expected_gen
					&& let Some(removed) = guard.remove(&key)
				{
					let payload = if removed.session_bytes >= removed.expected_session_bytes {
						EventPayload::TransferCompleted {
							id: removed.id,
							owner: removed.owner.clone(),
							direction: TransferDirection::Download,
							path: removed.path,
							size: removed.total_bytes,
							entry: None,
						}
					} else {
						EventPayload::TransferCancelled {
							id: removed.id,
							owner: removed.owner.clone(),
							direction: TransferDirection::Download,
						}
					};
					to_emit = Some((removed.owner, payload));
				}
				if let Some((owner, payload)) = to_emit {
					hub.send(&owner, payload);
				}
			});
		}
	}
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScopeQuery {
	#[serde(default, deserialize_with = "core::query_flag")]
	all: bool,
	owner: Option<String>,
}

pub(super) async fn list_transfers(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<ScopeQuery>,
) -> Result<Json<Vec<TransferSnapshot>>> {
	let p = auth::authenticate(&app, &headers).await?;
	if !p.manage() {
		return Err(forbidden());
	}
	if query.all {
		if !p.admin() {
			return Err(forbidden());
		}
		return Ok(Json(app.events.snapshots(&app, None).await?));
	}
	let owner = super::files::owner(&app, &p, query.owner.as_deref()).await?;
	Ok(Json(app.events.snapshots(&app, Some(&owner)).await?))
}

enum StreamPhase {
	Init,
	Active,
}

pub(super) async fn sse(
	State(app): State<App>,
	headers: HeaderMap,
	Query(query): Query<ScopeQuery>,
) -> Result<Response> {
	let p = auth::authenticate(&app, &headers).await?;
	if !p.manage() {
		return Err(forbidden());
	}
	let owner_filter = if query.all {
		if !p.admin() {
			return Err(forbidden());
		}
		None
	} else {
		Some(super::files::owner(&app, &p, query.owner.as_deref()).await?)
	};

	let rx = app.events.tx.subscribe();
	let mut ping = tokio::time::interval(KEEP_ALIVE_INTERVAL);
	ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	let user_id = p.user.id.clone();

	let stream = futures_util::stream::unfold(
		(StreamPhase::Init, rx, ping, owner_filter, user_id),
		|(phase, mut rx, mut ping, filter, uid)| async move {
			if matches!(phase, StreamPhase::Init) {
				let init_msg = format!("event: ready\ndata: {{\"user_id\":{uid:?}}}\n\n");
				return Some((
					Ok::<bytes::Bytes, std::convert::Infallible>(bytes::Bytes::from(init_msg)),
					(StreamPhase::Active, rx, ping, filter, uid),
				));
			}
			loop {
				tokio::select! {
					_ = ping.tick() => {
						return Some((
							Ok(bytes::Bytes::from_static(b": keep-alive\n\n")),
							(StreamPhase::Active, rx, ping, filter, uid),
						));
					}
					item = rx.recv() => {
						match item {
							Ok(scoped) => {
								if filter.as_deref().is_some_and(|wanted| scoped.owner != wanted) {
									continue;
								}
								if let Ok(json) = serde_json::to_string(&scoped.payload) {
									let frame = format!("data: {json}\n\n");
									return Some((
										Ok(bytes::Bytes::from(frame)),
										(StreamPhase::Active, rx, ping, filter, uid),
									));
								}
							}
							Err(broadcast::error::RecvError::Lagged(missed)) => {
								let frame = format!(
									"data: {{\"type\":\"resync\",\"missed\":{missed}}}\n\n"
								);
								return Some((
									Ok(bytes::Bytes::from(frame)),
									(StreamPhase::Active, rx, ping, filter, uid),
								));
							}
							Err(broadcast::error::RecvError::Closed) => return None,
						}
					}
				}
			}
		},
	);

	Ok(Response::builder()
		.header(header::CONTENT_TYPE, "text/event-stream")
		.header(header::CACHE_CONTROL, "no-cache, no-transform")
		.header("x-accel-buffering", "no")
		.body(Body::from_stream(stream))?)
}
