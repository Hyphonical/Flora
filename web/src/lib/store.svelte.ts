// Production Svelte 5 runes store wired to Flora's backend API and SSE stream.

import { api, ApiError } from './api';
import { FloraSseClient } from './sse';
import type {
  ActivityRecord,
  ChunkVisualState,
  ConflictMode,
  DashboardUserStat,
  Entry,
  FileKind,
  FileRecord,
  Scene,
  ServerInfo,
  ServerSseEvent,
  ShareInfo,
  Tab,
  TokenRecord,
  TransferItem,
  TransferSnapshot,
  TrashItem,
  UserView,
  ViewMode
} from './types';

export type {
  ActivityRecord,
  ChunkVisualState,
  ConflictMode,
  Entry,
  FileKind,
  Scene,
  ShareInfo,
  Tab,
  TokenRecord,
  TransferItem,
  TrashItem,
  UserView,
  ViewMode
};

const TEXT_FILENAMES: Record<string, true> = {
  dockerfile: true,
  makefile: true,
  justfile: true,
  license: true,
  readme: true,
  '.gitignore': true,
  '.gitattributes': true,
  '.editorconfig': true,
  '.dockerignore': true,
  '.env': true
};

const TEXT_EXTENSIONS: Record<string, true> = {
  txt: true,
  md: true,
  markdown: true,
  rs: true,
  ts: true,
  tsx: true,
  js: true,
  jsx: true,
  mjs: true,
  cjs: true,
  svelte: true,
  vue: true,
  html: true,
  htm: true,
  css: true,
  scss: true,
  json: true,
  jsonc: true,
  jsonl: true,
  toml: true,
  yaml: true,
  yml: true,
  ini: true,
  conf: true,
  cfg: true,
  env: true,
  sh: true,
  bash: true,
  zsh: true,
  fish: true,
  ps1: true,
  bat: true,
  cmd: true,
  py: true,
  go: true,
  c: true,
  h: true,
  cpp: true,
  hpp: true,
  cs: true,
  java: true,
  kt: true,
  swift: true,
  rb: true,
  php: true,
  lua: true,
  sql: true,
  graphql: true,
  xml: true,
  csv: true,
  tsv: true,
  log: true,
  diff: true,
  patch: true,
  nix: true,
  zig: true
};

const IMAGE_EXTENSIONS: Record<string, true> = {
  jpg: true,
  jpeg: true,
  png: true,
  webp: true,
  gif: true,
  svg: true,
  bmp: true,
  avif: true
};
const AUDIO_EXTENSIONS: Record<string, true> = {
  mp3: true,
  wav: true,
  flac: true,
  ogg: true,
  m4a: true,
  aac: true,
  opus: true
};
const ARCHIVE_EXTENSIONS: Record<string, true> = {
  zip: true,
  tar: true,
  gz: true,
  tgz: true,
  zst: true,
  xz: true,
  bz2: true,
  '7z': true,
  rar: true
};
const THUMBNAIL_EXTENSIONS: Record<string, true> = {
  jpg: true,
  jpeg: true,
  png: true,
  webp: true,
  gif: true
};

export function detectFileKind(path: string, directory: boolean): FileKind {
  if (directory) return 'folder';
  const name = path.split('/').pop() ?? path;
  const lower = name.toLowerCase();
  if (TEXT_FILENAMES[lower]) return 'text';
  const dot = lower.lastIndexOf('.');
  const ext = dot > 0 ? lower.slice(dot + 1) : '';
  if (!ext) return 'text';
  if (IMAGE_EXTENSIONS[ext]) return 'image';
  if (AUDIO_EXTENSIONS[ext]) return 'audio';
  if (ext === 'pdf') return 'pdf';
  if (ARCHIVE_EXTENSIONS[ext]) return 'archive';
  if (TEXT_EXTENSIONS[ext]) return 'text';
  return 'archive';
}

export function supportsThumbnail(path: string, size: number): boolean {
  if (size <= 0 || size > 64 * 1024 * 1024) return false;
  const name = path.split('/').pop() ?? path;
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return false;
  return Boolean(THUMBNAIL_EXTENSIONS[name.slice(dot + 1).toLowerCase()]);
}

export function formatBytes(bytes: number | null | undefined): string {
  if (!bytes || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const k = 1000;
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), units.length - 1);
  const val = bytes / Math.pow(k, i);
  if (i === 0) return `${Math.round(val)} B`;
  return `${val >= 100 ? val.toFixed(0) : val >= 10 ? val.toFixed(1) : val.toFixed(2)} ${units[i]}`;
}

export function shortHash(hash: string | null | undefined): string {
  if (!hash || hash === '—') return '—';
  if (hash.length <= 14) return hash;
  return `${hash.slice(0, 8)}…${hash.slice(-4)}`;
}

export function formatTimestamp(unixSeconds: number): string {
  if (!unixSeconds || unixSeconds <= 0) return '—';
  const now = Math.floor(Date.now() / 1000);
  const diff = now - unixSeconds;
  if (diff >= -5 && diff < 45) return 'Just now';
  if (diff >= 45 && diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff >= 3600 && diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  const d = new Date(unixSeconds * 1000);
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' })
  });
}

export function formatExpiresIn(expiresAt: number | null): string {
  if (expiresAt === null) return 'never';
  const now = Math.floor(Date.now() / 1000);
  const rem = expiresAt - now;
  if (rem <= 0) return 'expired';
  const days = Math.floor(rem / 86400);
  const hours = Math.floor((rem % 86400) / 3600);
  if (days > 0) return `${days}d ${String(hours).padStart(2, '0')}h`;
  const mins = Math.max(1, Math.floor((rem % 3600) / 60));
  return hours > 0 ? `${hours}h ${mins}m` : `${mins}m`;
}

export interface FileItem {
  id: string;
  owner: string;
  name: string;
  folder: string;
  path: string;
  directory: boolean;
  bytes: number;
  itemCount?: number;
  hash: string;
  modified: string;
  modifiedTs: number;
  sharedToken?: string;
  kind: FileKind;
}

export interface UserSpace {
  id: string;
  name: string;
  admin: boolean;
  usedBytes: number;
  trashBytes: number;
  quotaBytes: number | null;
  bufferReservedBytes: number;
  retention: string;
}

function splitPath(path: string): { folder: string; name: string } {
  const clean = path.replace(/^\/+|\/+$/g, '');
  const slash = clean.lastIndexOf('/');
  if (slash === -1) return { folder: '', name: clean };
  return { folder: clean.slice(0, slash), name: clean.slice(slash + 1) };
}

function entryToFileItem(entry: Entry, owner: string): FileItem {
  const { folder, name } = splitPath(entry.path);
  return {
    id: entry.path,
    owner,
    name: entry.name || name,
    folder,
    path: entry.path,
    directory: entry.directory,
    bytes: entry.size,
    itemCount: entry.item_count,
    hash: entry.hash ?? '—',
    modified: formatTimestamp(entry.modified),
    modifiedTs: entry.modified,
    sharedToken: entry.shared_token,
    kind: detectFileKind(entry.path, entry.directory)
  };
}

function recordToFileItem(rec: FileRecord, sharedMap: Map<string, string>): FileItem {
  const { folder, name } = splitPath(rec.path);
  return {
    id: rec.path,
    owner: rec.owner,
    name,
    folder,
    path: rec.path,
    directory: false,
    bytes: rec.size,
    hash: rec.hash,
    modified: formatTimestamp(rec.modified),
    modifiedTs: rec.modified,
    sharedToken: sharedMap.get(rec.path),
    kind: detectFileKind(rec.path, false)
  };
}

function buildChunkVisuals(
  totalBytes: number | null,
  transferredBytes: number,
  chunkSize?: number,
  chunksDone?: number,
  chunksTotal?: number,
  phase: TransferItem['phase'] = 'transferring'
): ChunkVisualState[] {
  const count = Math.max(
    1,
    Math.min(
      24,
      chunksTotal && chunksTotal > 0
        ? chunksTotal
        : totalBytes && chunkSize && chunkSize > 0
          ? Math.ceil(totalBytes / chunkSize)
          : totalBytes && totalBytes > 0
            ? Math.min(16, Math.max(4, Math.ceil(totalBytes / 8_000_000)))
            : 8
    )
  );

  if (phase === 'complete' || phase === 'verifying' || phase === 'storing') {
    return Array.from({ length: count }, () => 'done');
  }

  let doneRatio = 0;
  if (chunksTotal && chunksTotal > 0 && chunksDone !== undefined) {
    doneRatio = chunksDone / chunksTotal;
  } else if (totalBytes && totalBytes > 0) {
    doneRatio = Math.min(1, transferredBytes / totalBytes);
  }

  const doneCells = Math.min(count, Math.floor(doneRatio * count));
  const hasInflight = doneCells < count && (transferredBytes > 0 || (chunksDone ?? 0) > 0);
  return Array.from({ length: count }, (_, idx) => {
    if (idx < doneCells) return 'done';
    if (hasInflight && idx === doneCells) return 'inflight';
    return 'queued';
  });
}

function snapshotToTransferItem(snap: TransferSnapshot): TransferItem {
  const { folder, name } = splitPath(snap.path);
  return {
    id: snap.id,
    owner: snap.owner,
    direction: snap.direction,
    source: snap.source,
    path: snap.path,
    name: name || snap.path || 'transfer',
    folder,
    total_bytes: snap.total_bytes,
    transferred_bytes: snap.transferred_bytes,
    speed_bps: snap.speed_bps,
    phase: snap.phase,
    chunk_size: snap.chunk_size,
    chunks_done: snap.chunks_done,
    chunks_total: snap.chunks_total,
    chunks: buildChunkVisuals(
      snap.total_bytes,
      snap.transferred_bytes,
      snap.chunk_size,
      snap.chunks_done,
      snap.chunks_total,
      snap.phase
    ),
    started_at: snap.started_at,
    updated_at: snap.updated_at
  };
}

export function matchShareRoute(pathname: string): { token: string; subpath: string } | null {
  const m = /^\/t\/([^/]+)(?:\/(.*))?$/.exec(pathname);
  if (!m) return null;
  const token = decodeURIComponent(m[1]);
  const rawSub = m[2] ?? '';
  const subpath = rawSub
    .split('/')
    .filter(Boolean)
    .map((s) => {
      try {
        return decodeURIComponent(s);
      } catch {
        return s;
      }
    })
    .join('/');
  return { token, subpath };
}

class FloraStore {
  // Scene & navigation
  scene = $state<Scene>('loading');
  tab = $state<Tab>('files');
  viewMode = $state<ViewMode>('list');
  currentFolder = $state<string>('');
  search = $state<string>('');
  conflictMode = $state<ConflictMode>('reject');

  // Server & Auth state
  server = $state<ServerInfo>({
    version: '0.1.0',
    chunk_size: 8_000_000,
    buffer_size: 3_000_000_000,
    trash_retention: '7d',
    public_url: null
  });
  authUser = $state<UserView | null>(null);
  selectedOwnerId = $state<string>('');

  // Shell layout & responsive panels
  leftWidth = $state<number>(240);
  rightWidth = $state<number>(332);
  leftCollapsed = $state<boolean>(false);
  rightCollapsed = $state<boolean>(false);
  mobileNavOpen = $state<boolean>(false);
  mobileInspectorOpen = $state<boolean>(false);

  // Workspace data collections
  rawUsers = $state<UserView[]>([]);
  dashboardStats = $state<DashboardUserStat[]>([]);
  dashboardBufferSize = $state<number>(3_000_000_000);
  files = $state<FileItem[]>([]);
  searchResults = $state<FileItem[] | null>(null);
  pastes = $state<FileItem[]>([]);
  tokens = $state<TokenRecord[]>([]);
  trash = $state<TrashItem[]>([]);
  activity = $state<ActivityRecord[]>([]);

  // Newly minted CLI token secret (shown once)
  mintedAccountToken = $state<{ token: string; label?: string } | null>(null);

  // Selection, text preview cache & inline editing
  selectedFilePath = $state<string | null>(null);
  editingFilePath = $state<string | null>(null);
  editingFileDraftItem = $state<FileItem | null>(null);
  activePastePath = $state<string | null>(null);
  newUnsavedPaste = $state<FileItem | null>(null);
  textCache = $state<Record<string, string>>({});
  loadingTextPaths = $state<Record<string, boolean>>({});

  // Transfers Dock & SSE state
  transfers = $state<TransferItem[]>([]);
  dockExpanded = $state<boolean>(true);
  dockVisible = $state<boolean>(false);
  sseConnected = $state<boolean>(false);
  ssePaused = $state<boolean>(false);

  // Public share view state
  shareToken = $state<string>('');
  shareSubpath = $state<string>('');
  shareInfo = $state<ShareInfo | null>(null);
  shareLoading = $state<boolean>(false);
  shareError = $state<string | null>(null);
  shareTextPreview = $state<string | null>(null);

  // Loading & toast feedback
  loadingFiles = $state<boolean>(false);
  toastMessage = $state<string | null>(null);

  private toastTimer: number | undefined = undefined;
  private searchTimer: number | undefined = undefined;
  private sse: FloraSseClient;
  private uploadAborts = new Map<string, AbortController>();

  constructor() {
    this.sse = new FloraSseClient({
      onReady: () => {
        this.sseConnected = true;
      },
      onDisconnected: () => {
        this.sseConnected = false;
      },
      onEvent: (event) => this.handleSseEvent(event)
    });

    if (typeof window !== 'undefined') {
      window.addEventListener('popstate', () => {
        const shareMatch = matchShareRoute(window.location.pathname);
        if (shareMatch) {
          void this.openPublicShare(shareMatch.token, shareMatch.subpath, false);
        }
      });
      void this.init();
    }
  }

  notify(msg: string) {
    this.toastMessage = msg;
    clearTimeout(this.toastTimer);
    this.toastTimer = window.setTimeout(() => {
      this.toastMessage = null;
    }, 2800);
  }

  notifyError(err: unknown, fallback = 'Operation failed') {
    const msg = err instanceof Error ? err.message : fallback;
    this.notify(msg);
  }

  // Computed properties
  get ownerParam(): string | undefined {
    if (!this.authUser || !this.selectedOwnerId) return undefined;
    return this.selectedOwnerId !== this.authUser.id ? this.selectedOwnerId : undefined;
  }

  get users(): UserSpace[] {
    const list = this.rawUsers.length > 0 ? this.rawUsers : this.authUser ? [this.authUser] : [];
    const statMap = new Map(this.dashboardStats.map((s) => [s.id, s]));
    return list.map((u) => {
      const stat = statMap.get(u.id);
      const isSelected = u.id === (this.selectedOwnerId || this.authUser?.id);
      const trashBytes = isSelected
        ? this.trash.reduce((sum, t) => sum + t.size, 0)
        : 0;
      return {
        id: u.id,
        name: u.name,
        admin: u.admin,
        usedBytes: stat?.used ?? 0,
        trashBytes,
        quotaBytes: u.quota ?? stat?.quota ?? null,
        bufferReservedBytes: stat?.buffer_reserved ?? 0,
        retention: u.retention ?? this.server.trash_retention
      };
    });
  }

  get currentUser(): UserSpace {
    const targetId = this.selectedOwnerId || this.authUser?.id || '';
    return (
      this.users.find((u) => u.id === targetId) ??
      this.users[0] ?? {
        id: targetId,
        name: this.authUser?.name ?? 'user',
        admin: this.authUser?.admin ?? false,
        usedBytes: 0,
        trashBytes: 0,
        quotaBytes: null,
        bufferReservedBytes: 0,
        retention: this.server.trash_retention
      }
    );
  }

  get isAdmin(): boolean {
    return Boolean(this.authUser?.admin);
  }

  get basePublicUrl(): string {
    if (this.server.public_url) {
      return this.server.public_url.replace(/\/+$/, '');
    }
    return typeof window !== 'undefined' ? window.location.origin : '';
  }

  shareUrlFor(secret: string): string {
    return `${this.basePublicUrl}/t/${secret}`;
  }

  downloadUrlFor(path: string): string {
    return api.downloadUrl(path, this.ownerParam);
  }

  previewUrlFor(path: string): string {
    return api.previewUrl(path, this.ownerParam);
  }

  thumbnailUrlFor(path: string): string {
    return api.thumbnailUrl(path, this.ownerParam);
  }

  get visibleFiles(): FileItem[] {
    if (this.search.trim() && this.searchResults !== null) {
      return this.searchResults;
    }
    return [...this.files].sort(
      (a, b) => Number(b.directory) - Number(a.directory) || a.name.localeCompare(b.name)
    );
  }

  get selectedFile(): FileItem | null {
    const list = this.visibleFiles;
    if (this.selectedFilePath) {
      const found = list.find((f) => f.path === this.selectedFilePath);
      if (found) return found;
    }
    if (this.editingFile) return this.editingFile;
    return list[0] ?? null;
  }

  get editingFile(): FileItem | null {
    if (!this.editingFilePath) return null;
    return (
      this.files.find((f) => f.path === this.editingFilePath) ??
      this.searchResults?.find((f) => f.path === this.editingFilePath) ??
      this.pastes.find((f) => f.path === this.editingFilePath) ??
      (this.editingFileDraftItem?.path === this.editingFilePath ? this.editingFileDraftItem : null)
    );
  }

  get pasteFiles(): FileItem[] {
    if (this.newUnsavedPaste) {
      const exists = this.pastes.some((p) => p.path === this.newUnsavedPaste!.path);
      return exists ? this.pastes : [this.newUnsavedPaste, ...this.pastes];
    }
    return this.pastes;
  }

  get activePaste(): FileItem | null {
    const list = this.pasteFiles;
    if (this.activePastePath) {
      const found = list.find((p) => p.path === this.activePastePath);
      if (found) return found;
    }
    return list[0] ?? null;
  }

  get ownerTokens(): TokenRecord[] {
    const ownerId = this.selectedOwnerId || this.authUser?.id;
    return this.tokens
      .filter((t) => !t.session && (!ownerId || t.owner === ownerId))
      .sort((a, b) => b.created - a.created);
  }

  get ownerTrash(): TrashItem[] {
    return [...this.trash].sort((a, b) => b.deleted - a.deleted);
  }

  get activeTransfers(): TransferItem[] {
    return this.transfers.filter((t) => t.phase !== 'complete');
  }

  get totalBufferReserved(): number {
    const dashReserved = this.dashboardStats.reduce((sum, u) => sum + u.buffer_reserved, 0);
    const activeUploadBytes = this.transfers
      .filter((t) => t.direction === 'upload' && t.phase !== 'complete')
      .reduce((sum, t) => sum + (t.total_bytes ?? t.transferred_bytes), 0);
    return Math.max(dashReserved, activeUploadBytes);
  }

  get aggregateSpeedMbps(): number {
    const bps = this.transfers
      .filter((t) => t.phase === 'transferring')
      .reduce((sum, t) => sum + t.speed_bps, 0);
    return bps / 1_000_000;
  }

  get breadcrumbs(): { name: string; path: string }[] {
    if (!this.currentFolder) return [];
    const parts = this.currentFolder.split('/').filter(Boolean);
    return parts.map((name, i) => ({
      name,
      path: parts.slice(0, i + 1).join('/')
    }));
  }

  // Initialization & Routing
  async init() {
    const shareMatch = matchShareRoute(window.location.pathname);
    if (shareMatch) {
      await this.openPublicShare(shareMatch.token, shareMatch.subpath, false);
      return;
    }

    try {
      const boot = await api.bootstrap();
      this.server = boot.server;
      this.dashboardBufferSize = boot.server.buffer_size;

      if (boot.setup_required) {
        this.scene = 'setup';
      } else if (!boot.user) {
        this.scene = 'login';
      } else {
        await this.enterWorkspace(boot.user);
      }
    } catch (err) {
      this.scene = 'login';
      this.notifyError(err, 'Could not connect to Flora server');
    }
  }

  private async enterWorkspace(user: UserView) {
    this.authUser = user;
    this.selectedOwnerId = user.id;
    this.currentFolder = '';
    this.search = '';
    this.searchResults = null;
    this.editingFilePath = null;
    this.editingFileDraftItem = null;
    this.scene = 'workspace';

    this.connectSseForCurrentScope();
    await this.hydrateWorkspace();
  }

  private connectSseForCurrentScope() {
    if (!this.authUser || this.ssePaused) return;
    if (this.authUser.admin) {
      this.sse.connect({ all: true });
    } else {
      this.sse.connect();
    }
  }

  toggleSseStream() {
    if (this.ssePaused) {
      this.ssePaused = false;
      this.connectSseForCurrentScope();
      void this.refreshTransfers();
    } else {
      this.ssePaused = true;
      this.sse.pause();
    }
  }

  async hydrateWorkspace() {
    const tasks: Promise<unknown>[] = [
      this.refreshFiles(),
      this.refreshPastes(),
      this.refreshTokens(),
      this.refreshTrash(),
      this.refreshDashboard(),
      this.refreshTransfers()
    ];
    if (this.authUser?.admin) {
      tasks.push(this.refreshUsers(), this.refreshActivity());
    } else if (this.authUser) {
      this.rawUsers = [this.authUser];
    }
    await Promise.allSettled(tasks);
  }

  // Auth actions
  async submitSetup(code: string, name: string, password: string) {
    const res = await api.setup(code.trim(), name.trim(), password);
    const boot = await api.bootstrap();
    this.server = boot.server;
    await this.enterWorkspace(res.user);
    this.notify('Administrator account created');
  }

  async submitLogin(name: string, password: string) {
    const res = await api.login(name.trim(), password);
    const boot = await api.bootstrap();
    this.server = boot.server;
    await this.enterWorkspace(res.user);
    this.notify(`Signed in as ${res.user.name}`);
  }

  async signOut() {
    try {
      await api.logout();
    } catch {
      // Ignore logout network error and still clear local state
    }
    for (const ctrl of this.uploadAborts.values()) {
      ctrl.abort();
    }
    this.uploadAborts.clear();
    this.sse.close();
    this.authUser = null;
    this.selectedOwnerId = '';
    this.files = [];
    this.pastes = [];
    this.tokens = [];
    this.trash = [];
    this.transfers = [];
    this.editingFilePath = null;
    this.editingFileDraftItem = null;
    this.mobileNavOpen = false;
    this.mobileInspectorOpen = false;
    this.scene = 'login';
    this.notify('Signed out');
  }

  // Data refreshers
  async refreshFiles() {
    this.loadingFiles = true;
    try {
      const entries = await api.listFiles(this.currentFolder, this.ownerParam);
      const ownerId = this.selectedOwnerId || this.authUser?.id || '';
      this.files = entries.map((e) => entryToFileItem(e, ownerId));
      if (this.selectedFilePath && !this.files.some((f) => f.path === this.selectedFilePath)) {
        this.selectedFilePath = this.files[0]?.path ?? null;
      } else if (!this.selectedFilePath && this.files.length > 0) {
        this.selectedFilePath = this.files[0].path;
      }
      const sel = this.selectedFile;
      if (sel && sel.kind === 'text' && !sel.directory) {
        void this.ensureTextLoaded(sel.path);
      }
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        await this.signOut();
        return;
      }
      this.notifyError(err, 'Failed to load files');
    } finally {
      this.loadingFiles = false;
    }
  }

  async refreshPastes() {
    try {
      const entries = await api.listPastes(this.ownerParam);
      const ownerId = this.selectedOwnerId || this.authUser?.id || '';
      this.pastes = entries.map((e) => entryToFileItem(e, ownerId));
      if (this.newUnsavedPaste && this.pastes.some((p) => p.path === this.newUnsavedPaste!.path)) {
        this.newUnsavedPaste = null;
      }
      const act = this.activePaste;
      if (act && act !== this.newUnsavedPaste) {
        this.activePastePath = act.path;
        if (this.tab === 'pastes') {
          void this.ensureTextLoaded(act.path);
        }
      }
    } catch {
      // Non-fatal
    }
  }

  async refreshTokens() {
    try {
      this.tokens = await api.listTokens();
    } catch {
      // Non-fatal
    }
  }

  async refreshTrash() {
    try {
      this.trash = await api.listTrash(this.ownerParam);
    } catch {
      // Non-fatal
    }
  }

  async refreshDashboard() {
    try {
      const dash = await api.dashboard();
      this.dashboardBufferSize = dash.buffer_size;
      this.dashboardStats = dash.users;
    } catch {
      // Non-fatal
    }
  }

  async refreshUsers() {
    if (!this.authUser?.admin) return;
    try {
      this.rawUsers = await api.listUsers();
    } catch {
      // Non-fatal
    }
  }

  async refreshActivity() {
    if (!this.authUser?.admin) return;
    try {
      this.activity = await api.activity(200);
    } catch {
      // Non-fatal
    }
  }

  async refreshTransfers() {
    if (!this.authUser) return;
    try {
      const snaps = await api.listTransfers(
        this.authUser.admin ? { all: true } : { owner: this.ownerParam }
      );
      const activeItems = snaps.map(snapshotToTransferItem);
      const completedExisting = this.transfers.filter(
        (t) => t.phase === 'complete' && !activeItems.some((a) => a.id === t.id)
      );
      this.transfers = [...activeItems, ...completedExisting];
      if (activeItems.length > 0) {
        this.dockVisible = true;
      }
    } catch {
      // Non-fatal
    }
  }

  // Navigation & Space Switching
  selectTab(next: Tab) {
    if (next === 'admin' && !this.isAdmin) return;
    this.tab = next;
    this.editingFilePath = null;
    this.editingFileDraftItem = null;
    this.mobileNavOpen = false;
    if (next === 'files') {
      void this.refreshFiles();
    } else if (next === 'pastes') {
      void this.refreshPastes();
    } else if (next === 'tokens') {
      void this.refreshTokens();
    } else if (next === 'trash') {
      void this.refreshTrash();
    } else if (next === 'admin') {
      void Promise.all([this.refreshDashboard(), this.refreshUsers(), this.refreshActivity()]);
    }
  }

  async openFolder(path: string) {
    this.currentFolder = path.replace(/^\/+|\/+$/g, '');
    this.search = '';
    this.searchResults = null;
    this.editingFilePath = null;
    this.editingFileDraftItem = null;
    this.selectedFilePath = null;
    await this.refreshFiles();
  }

  async switchSpace(ownerIdOrName: string) {
    const found =
      this.users.find((u) => u.id === ownerIdOrName || u.name === ownerIdOrName) ?? null;
    if (!found) return;
    this.selectedOwnerId = found.id;
    this.currentFolder = '';
    this.search = '';
    this.searchResults = null;
    this.selectedFilePath = null;
    this.editingFilePath = null;
    this.editingFileDraftItem = null;
    this.activePastePath = null;
    this.newUnsavedPaste = null;
    this.textCache = {};
    await Promise.allSettled([
      this.refreshFiles(),
      this.refreshPastes(),
      this.refreshTrash(),
      this.refreshTokens(),
      this.refreshDashboard()
    ]);
  }

  setSearch(query: string) {
    this.search = query;
    clearTimeout(this.searchTimer);
    const trimmed = query.trim();
    if (!trimmed) {
      this.searchResults = null;
      return;
    }
    this.searchTimer = window.setTimeout(async () => {
      try {
        const records = await api.search(trimmed, this.ownerParam);
        const ownerId = this.selectedOwnerId || this.authUser?.id || '';
        const shareMap = new Map<string, string>();
        for (const tok of this.tokens) {
          if (tok.owner === ownerId && tok.secret && (tok.kind === 'file' || tok.kind === 'folder')) {
            shareMap.set(tok.scope, tok.secret);
          }
        }
        if (this.search.trim() === trimmed) {
          this.searchResults = records.map((r) => recordToFileItem(r, shareMap));
          if (this.searchResults.length > 0) {
            this.selectedFilePath = this.searchResults[0].path;
          }
        }
      } catch (err) {
        this.notifyError(err, 'Search failed');
      }
    }, 180);
  }

  selectFile(item: FileItem) {
    this.selectedFilePath = item.path;
    if (item.kind === 'text' && !item.directory && item.bytes <= 2 * 1024 * 1024) {
      void this.ensureTextLoaded(item.path);
    }
  }

  async openEditor(item: FileItem) {
    this.selectedFilePath = item.path;
    this.editingFileDraftItem = item;
    this.editingFilePath = item.path;
    await this.ensureTextLoaded(item.path, true);
  }

  closeEditor() {
    this.editingFilePath = null;
    this.editingFileDraftItem = null;
  }

  // Text preview & editing
  private cacheKey(path: string): string {
    const owner = this.selectedOwnerId || this.authUser?.id || '';
    return `${owner}:${path}`;
  }

  getCachedText(path: string): string | undefined {
    return this.textCache[this.cacheKey(path)];
  }

  isTextLoading(path: string): boolean {
    return Boolean(this.loadingTextPaths[this.cacheKey(path)]);
  }

  async ensureTextLoaded(path: string, force = false): Promise<string> {
    const key = this.cacheKey(path);
    if (!force && key in this.textCache) {
      return this.textCache[key];
    }
    if (this.newUnsavedPaste && this.newUnsavedPaste.path === path && !(key in this.textCache)) {
      this.textCache[key] = '';
      return '';
    }
    this.loadingTextPaths[key] = true;
    try {
      const content = await api.fetchTextPreview(path, this.ownerParam);
      this.textCache[key] = content;
      return content;
    } catch {
      return this.textCache[key] ?? '';
    } finally {
      delete this.loadingTextPaths[key];
    }
  }

  // File & Paste mutations
  async createFolder(name: string) {
    const clean = name.trim().replace(/^[\\/]+|[\\/]+$/g, '');
    if (!clean) return;
    const path = this.currentFolder ? `${this.currentFolder}/${clean}` : clean;
    try {
      await api.createFolder(path, this.ownerParam);
      await this.refreshFiles();
      this.selectedFilePath = path;
      this.notify(`Created folder ${clean}`);
    } catch (err) {
      this.notifyError(err, 'Could not create folder');
    }
  }

  async renameFile(fromPath: string, nextPath: string) {
    const clean = nextPath.replace(/^\/+|\/+$/g, '').trim();
    if (!clean || clean === fromPath) return;
    try {
      await api.moveFile(fromPath, clean, this.ownerParam);
      const oldKey = this.cacheKey(fromPath);
      if (oldKey in this.textCache) {
        this.textCache[this.cacheKey(clean)] = this.textCache[oldKey];
        delete this.textCache[oldKey];
      }
      if (this.selectedFilePath === fromPath) this.selectedFilePath = clean;
      if (this.editingFilePath === fromPath) this.editingFilePath = clean;
      if (this.activePastePath === fromPath) this.activePastePath = clean;
      await Promise.all([this.refreshFiles(), this.refreshPastes()]);
      this.notify(`Moved to /${clean}`);
    } catch (err) {
      this.notifyError(err, 'Move / rename failed');
    }
  }

  async saveFileContent(originalPath: string, targetPath: string, content: string) {
    const cleanTarget = targetPath.replace(/^\/+|\/+$/g, '').trim() || originalPath;
    const isUnsavedNew = this.newUnsavedPaste?.path === originalPath;

    try {
      if (!isUnsavedNew && cleanTarget !== originalPath) {
        await api.moveFile(originalPath, cleanTarget, this.ownerParam);
      }
      await api.putTextFile(cleanTarget, content, 'replace', this.ownerParam);
      this.textCache[this.cacheKey(cleanTarget)] = content;
      if (cleanTarget !== originalPath) {
        delete this.textCache[this.cacheKey(originalPath)];
      }
      if (isUnsavedNew) {
        this.newUnsavedPaste = null;
      }
      if (this.editingFilePath === originalPath) {
        this.editingFilePath = cleanTarget;
      }
      if (this.selectedFilePath === originalPath) {
        this.selectedFilePath = cleanTarget;
      }
      if (this.activePastePath === originalPath) {
        this.activePastePath = cleanTarget;
      }
      await Promise.all([this.refreshFiles(), this.refreshPastes(), this.refreshDashboard()]);
      const savedName = cleanTarget.split('/').pop() ?? cleanTarget;
      this.notify(`Saved ${savedName}`);
    } catch (err) {
      this.notifyError(err, 'Failed to save file');
    }
  }

  createNewPaste() {
    const ownerId = this.selectedOwnerId || this.authUser?.id || '';
    const baseFolder = this.currentFolder || '';
    let idx = this.pastes.length + 1;
    let candidate = baseFolder ? `${baseFolder}/snippet-${idx}.md` : `snippet-${idx}.md`;
    while (this.pastes.some((p) => p.path === candidate)) {
      idx += 1;
      candidate = baseFolder ? `${baseFolder}/snippet-${idx}.md` : `snippet-${idx}.md`;
    }
    const { folder, name } = splitPath(candidate);
    const draft: FileItem = {
      id: candidate,
      owner: ownerId,
      name,
      folder,
      path: candidate,
      directory: false,
      bytes: 0,
      hash: '—',
      modified: 'Unsaved',
      modifiedTs: Math.floor(Date.now() / 1000),
      kind: 'text'
    };
    this.newUnsavedPaste = draft;
    this.textCache[this.cacheKey(candidate)] = '';
    this.activePastePath = candidate;
    this.tab = 'pastes';
    this.editingFilePath = null;
    this.editingFileDraftItem = null;
    this.mobileNavOpen = false;
  }

  triggerDownload(path: string) {
    const url = this.downloadUrlFor(path);
    const a = document.createElement('a');
    a.href = url;
    a.rel = 'noopener';
    document.body.appendChild(a);
    a.click();
    a.remove();
  }

  // Share Links & Tokens
  async toggleShareLink(item: FileItem) {
    if (this.newUnsavedPaste?.path === item.path) {
      await this.saveFileContent(
        item.path,
        item.path,
        this.getCachedText(item.path) ?? ''
      );
    }

    const ownerId = this.selectedOwnerId || this.authUser?.id || '';
    if (item.sharedToken) {
      // Find matching token hash to revoke
      if (this.tokens.length === 0) {
        await this.refreshTokens();
      }
      const match = this.tokens.find(
        (t) =>
          (t.secret && t.secret === item.sharedToken) ||
          (t.owner === ownerId && t.scope === item.path && (t.kind === 'file' || t.kind === 'folder'))
      );
      if (!match) {
        this.notify('Share token not found');
        return;
      }
      try {
        await api.revokeToken(match.hash);
        await Promise.all([this.refreshTokens(), this.refreshFiles(), this.refreshPastes()]);
        this.notify(`Revoked link for ${item.name}`);
      } catch (err) {
        this.notifyError(err, 'Failed to revoke share link');
      }
    } else {
      try {
        const created = await api.createToken({
          scope: item.path,
          kind: item.directory ? 'folder' : 'file',
          owner: this.ownerParam
        });
        const shareUrl = created.url ?? this.shareUrlFor(created.token);
        await navigator.clipboard?.writeText(shareUrl).catch(() => {});
        await Promise.all([this.refreshTokens(), this.refreshFiles(), this.refreshPastes()]);
        this.notify(`Created & copied share link for ${item.name}`);
      } catch (err) {
        this.notifyError(err, 'Failed to create share link');
      }
    }
  }

  async mintAccountToken(label: string) {
    const clean = label.trim() || undefined;
    try {
      const created = await api.createToken({
        kind: 'account',
        label: clean,
        owner: this.ownerParam
      });
      this.mintedAccountToken = { token: created.token, label: created.label };
      await this.refreshTokens();
      this.notify(`Created CLI token${created.label ? ` "${created.label}"` : ''}`);
    } catch (err) {
      this.notifyError(err, 'Failed to create token');
    }
  }

  async revokeToken(hash: string) {
    try {
      await api.revokeToken(hash);
      await Promise.all([this.refreshTokens(), this.refreshFiles(), this.refreshPastes()]);
      this.notify('Token revoked');
    } catch (err) {
      this.notifyError(err, 'Failed to revoke token');
    }
  }

  // Trash & Purge
  async moveToTrash(path: string) {
    if (this.newUnsavedPaste?.path === path) {
      this.newUnsavedPaste = null;
      this.activePastePath = this.pastes[0]?.path ?? null;
      this.notify('Discarded unsaved paste');
      return;
    }
    try {
      await api.deleteFile(path, this.ownerParam);
      if (this.editingFilePath === path) {
        this.editingFilePath = null;
        this.editingFileDraftItem = null;
      }
      this.mobileInspectorOpen = false;
      await Promise.all([
        this.refreshFiles(),
        this.refreshPastes(),
        this.refreshTrash(),
        this.refreshDashboard()
      ]);
      const name = path.split('/').pop() ?? path;
      this.notify(`Moved ${name} to trash`);
    } catch (err) {
      this.notifyError(err, 'Could not move to trash');
    }
  }

  async restoreFromTrash(id: string) {
    try {
      await api.restoreTrash(id, this.ownerParam);
      await Promise.all([
        this.refreshTrash(),
        this.refreshFiles(),
        this.refreshPastes(),
        this.refreshDashboard()
      ]);
      this.notify('Restored from trash');
    } catch (err) {
      this.notifyError(err, 'Restore failed');
    }
  }

  async deleteTrashItemPermanently(id: string) {
    try {
      await api.deleteTrashItem(id, this.ownerParam);
      await Promise.all([this.refreshTrash(), this.refreshDashboard()]);
      this.notify('Deleted permanently');
    } catch (err) {
      this.notifyError(err, 'Could not delete item');
    }
  }

  async emptyTrash() {
    try {
      const res = await api.purge({ confirm: true, owner: this.ownerParam });
      await Promise.all([this.refreshTrash(), this.refreshDashboard(), this.refreshTransfers()]);
      this.notify(`Purged ${res.trash} trash ${res.trash === 1 ? 'item' : 'items'}`);
    } catch (err) {
      this.notifyError(err, 'Failed to empty trash');
    }
  }

  async purgeAllAdmin() {
    try {
      const res = await api.purge({ confirm: true, all: true });
      await Promise.all([
        this.refreshTrash(),
        this.refreshDashboard(),
        this.refreshTransfers(),
        this.refreshActivity()
      ]);
      this.notify(`Purged ${res.trash} trash items & ${res.uploads} stale uploads`);
    } catch (err) {
      this.notifyError(err, 'Purge failed');
    }
  }

  async updateRetention(retention: string) {
    const targetId = this.selectedOwnerId || this.authUser?.id;
    if (!targetId) return;
    try {
      const updated = await api.updateUser(targetId, { retention });
      if (this.authUser?.id === updated.id) {
        this.authUser = updated;
      }
      this.rawUsers = this.rawUsers.map((u) => (u.id === updated.id ? updated : u));
      await this.refreshTrash();
      this.notify(`Retention set to ${retention}`);
    } catch (err) {
      this.notifyError(err, 'Failed to update retention');
    }
  }

  // Admin User Management
  async createUserAccount(params: {
    name: string;
    password: string;
    admin?: boolean;
    quotaBytes?: number | null;
  }) {
    try {
      const created = await api.createUser({
        name: params.name.trim(),
        password: params.password,
        admin: params.admin ?? false,
        quota: params.quotaBytes ?? null
      });
      await Promise.all([this.refreshUsers(), this.refreshDashboard(), this.refreshActivity()]);
      this.notify(`Created user ${created.name}`);
      return true;
    } catch (err) {
      this.notifyError(err, 'Failed to create user');
      return false;
    }
  }

  // Chunked Browser Uploads (Section 3.5)
  async uploadBrowserFiles(fileList: File[]) {
    for (const file of fileList) {
      void this.runChunkedUpload(file);
    }
  }

  private async runChunkedUpload(file: File) {
    const destPath = this.currentFolder ? `${this.currentFolder}/${file.name}` : file.name;
    let uploadId: string | null = null;
    const abortCtrl = new AbortController();

    try {
      const reservation = await api.reserveUpload(
        destPath,
        file.size,
        this.conflictMode,
        this.ownerParam
      );
      uploadId = reservation.id;
      this.uploadAborts.set(uploadId, abortCtrl);

      this.dockVisible = true;
      this.dockExpanded = true;

      // Ensure transfer appears immediately even before SSE arrives
      const chunkCount =
        file.size === 0 ? 0 : Math.ceil(file.size / reservation.chunk_size);
      const alreadyDone = reservation.chunks.filter((c) => c && c.hash).length;
      this.upsertTransfer({
        id: reservation.id,
        owner: reservation.owner,
        direction: 'upload',
        source: 'web',
        path: reservation.path,
        total_bytes: file.size,
        transferred_bytes: alreadyDone * reservation.chunk_size,
        speed_bps: 0,
        phase: 'transferring',
        chunk_size: reservation.chunk_size,
        chunks_done: alreadyDone,
        chunks_total: Math.max(1, chunkCount),
        started_at: Math.floor(Date.now() / 1000),
        updated_at: Math.floor(Date.now() / 1000)
      });

      if (chunkCount > 0) {
        const pendingIndices: number[] = [];
        for (let i = 0; i < chunkCount; i++) {
          if (!reservation.chunks[i]?.hash) {
            pendingIndices.push(i);
          }
        }

        const concurrency = Math.min(4, Math.max(1, pendingIndices.length));
        let cursor = 0;

        const worker = async () => {
          while (cursor < pendingIndices.length) {
            if (abortCtrl.signal.aborted) return;
            const idx = pendingIndices[cursor++];
            const start = idx * reservation.chunk_size;
            const end = Math.min(file.size, (idx + 1) * reservation.chunk_size);
            const slice = file.slice(start, end);
            await api.uploadChunk(reservation.id, idx, slice, abortCtrl.signal);
          }
        };

        await Promise.all(Array.from({ length: concurrency }, () => worker()));
      }

      if (abortCtrl.signal.aborted) return;

      await api.completeUpload(reservation.id, file.size, abortCtrl.signal);
      await Promise.all([this.refreshFiles(), this.refreshPastes(), this.refreshDashboard()]);
    } catch (err) {
      if (abortCtrl.signal.aborted) return;
      if (uploadId) {
        this.transfers = this.transfers.filter((t) => t.id !== uploadId);
      }
      this.notifyError(err, `Upload failed for ${file.name}`);
    } finally {
      if (uploadId) {
        this.uploadAborts.delete(uploadId);
      }
    }
  }

  async cancelOrDismissTransfer(id: string) {
    const item = this.transfers.find((t) => t.id === id);
    if (!item) return;

    if (item.phase === 'complete' || item.direction === 'download') {
      this.transfers = this.transfers.filter((t) => t.id !== id);
      return;
    }

    const abort = this.uploadAborts.get(id);
    if (abort) {
      abort.abort();
      this.uploadAborts.delete(id);
    }

    try {
      await api.cancelUpload(id);
    } catch {
      // May already be completed or cancelled
    }
    this.transfers = this.transfers.filter((t) => t.id !== id);
    void this.refreshDashboard();
  }

  clearCompletedTransfers() {
    this.transfers = this.transfers.filter((t) => t.phase !== 'complete');
  }

  // SSE Event Handling (Section 3.6)
  private upsertTransfer(snap: TransferSnapshot) {
    const item = snapshotToTransferItem(snap);
    const idx = this.transfers.findIndex((t) => t.id === snap.id);
    if (idx !== -1) {
      this.transfers[idx] = item;
    } else {
      this.transfers = [item, ...this.transfers];
    }
  }

  private handleSseEvent(event: ServerSseEvent) {
    if (event.type === 'transfer:started') {
      this.upsertTransfer(event.transfer);
      this.dockVisible = true;
      this.dockExpanded = true;
      return;
    }

    if (event.type === 'transfer:progress') {
      const idx = this.transfers.findIndex((t) => t.id === event.id);
      if (idx !== -1) {
        const cur = this.transfers[idx];
        const totalBytes = event.total_bytes ?? cur.total_bytes;
        const chunksDone = event.chunks_done ?? cur.chunks_done;
        const chunksTotal = event.chunks_total ?? cur.chunks_total;
        cur.phase = event.phase;
        cur.transferred_bytes = event.transferred_bytes;
        cur.total_bytes = totalBytes;
        cur.speed_bps = event.speed_bps;
        cur.chunks_done = chunksDone;
        cur.chunks_total = chunksTotal;
        cur.chunks = buildChunkVisuals(
          totalBytes,
          event.transferred_bytes,
          cur.chunk_size,
          chunksDone,
          chunksTotal,
          event.phase
        );
        if (
          event.phase === 'transferring' &&
          event.chunk_index !== undefined &&
          event.chunk_index < cur.chunks.length &&
          cur.chunks[event.chunk_index] === 'queued'
        ) {
          cur.chunks[event.chunk_index] = 'inflight';
        }
      } else {
        void this.refreshTransfers();
      }
      return;
    }

    if (event.type === 'transfer:completed') {
      const idx = this.transfers.findIndex((t) => t.id === event.id);
      if (idx !== -1) {
        const cur = this.transfers[idx];
        cur.phase = 'complete';
        cur.total_bytes = event.size;
        cur.transferred_bytes = event.size;
        cur.speed_bps = 0;
        cur.chunks = cur.chunks.map(() => 'done');
      }

      if (event.direction === 'upload') {
        const viewingOwner = this.selectedOwnerId || this.authUser?.id || '';
        if (event.owner === viewingOwner) {
          const { folder } = splitPath(event.path);
          if (event.entry && folder === this.currentFolder && !this.search.trim()) {
            const newItem = entryToFileItem(event.entry, event.owner);
            const existingIdx = this.files.findIndex((f) => f.path === newItem.path);
            if (existingIdx !== -1) {
              this.files[existingIdx] = newItem;
            } else {
              this.files = [newItem, ...this.files];
            }
          } else {
            void this.refreshFiles();
          }
          void this.refreshPastes();
        }
        void this.refreshDashboard();
        if (this.tab === 'admin') {
          void this.refreshActivity();
        }
      }
      return;
    }

    if (event.type === 'transfer:cancelled') {
      this.transfers = this.transfers.filter((t) => t.id !== event.id);
      void this.refreshDashboard();
      return;
    }

    if (event.type === 'tree:changed') {
      const viewingOwner = this.selectedOwnerId || this.authUser?.id || '';
      if (event.owner === viewingOwner) {
        if (this.tab === 'files') void this.refreshFiles();
        else if (this.tab === 'pastes') void this.refreshPastes();
        else if (this.tab === 'trash') void this.refreshTrash();
        void this.refreshTrash();
      }
      void this.refreshDashboard();
      return;
    }

    if (event.type === 'resync') {
      void this.refreshTransfers();
      if (this.tab === 'files') void this.refreshFiles();
      else if (this.tab === 'pastes') void this.refreshPastes();
      else if (this.tab === 'trash') void this.refreshTrash();
      else if (this.tab === 'admin') {
        void Promise.all([this.refreshDashboard(), this.refreshUsers(), this.refreshActivity()]);
      }
    }
  }

  // Public Share View (Section 3.10)
  async openPublicShare(token: string, subpath = '', pushHistory = true) {
    this.scene = 'share';
    this.shareToken = token;
    this.shareSubpath = subpath;
    this.shareLoading = true;
    this.shareError = null;
    this.shareTextPreview = null;

    if (pushHistory && typeof window !== 'undefined') {
      const encodedSub = subpath
        .split('/')
        .filter(Boolean)
        .map((s) => encodeURIComponent(s))
        .join('/');
      const targetUrl = encodedSub
        ? `/t/${encodeURIComponent(token)}/${encodedSub}`
        : `/t/${encodeURIComponent(token)}`;
      if (window.location.pathname !== targetUrl) {
        window.history.pushState({}, '', targetUrl);
      }
    }

    try {
      const info = await api.getShareInfo(token, subpath);
      this.shareInfo = info;
      if (!info.directory && info.entry) {
        const kind = detectFileKind(info.entry.path, false);
        if (kind === 'text' && info.entry.size <= 2 * 1024 * 1024) {
          this.shareTextPreview = await api.fetchShareText(token, subpath).catch(() => null);
        }
      }
    } catch (err) {
      this.shareInfo = null;
      this.shareError = err instanceof Error ? err.message : 'Shared link not found or expired';
    } finally {
      this.shareLoading = false;
    }
  }
}

export const store = new FloraStore();
