// Exact backend wire types & frontend domain models for Flora Web UI.

export type Scene = 'loading' | 'workspace' | 'share' | 'login' | 'setup';
export type Tab = 'files' | 'pastes' | 'tokens' | 'trash' | 'admin';
export type ViewMode = 'list' | 'grid';
export type ConflictMode = 'reject' | 'rename' | 'replace';
export type FileKind = 'folder' | 'image' | 'audio' | 'pdf' | 'archive' | 'text';

export interface ServerInfo {
  version: string;
  chunk_size: number;
  buffer_size: number;
  trash_retention: string;
  public_url: string | null;
}

export interface UserView {
  id: string;
  name: string;
  admin: boolean;
  quota: number | null;
  retention: string | null;
}

export interface BootstrapResponse {
  setup_required: boolean;
  user: UserView | null;
  server: ServerInfo;
}

export interface AuthResponse {
  token: string;
  user: UserView;
}

export interface Entry {
  name: string;
  path: string;
  directory: boolean;
  size: number;
  modified: number;
  hash: string | null;
  item_count?: number;
  shared_token?: string;
}

export interface FileRecord {
  owner: string;
  path: string;
  size: number;
  hash: string;
  modified: number;
}

export interface UploadChunkInfo {
  size: number;
  hash: string;
}

export interface Upload {
  id: string;
  owner: string;
  path: string;
  chunk_size: number;
  chunks: Array<UploadChunkInfo | null>;
  size: number | null;
  reserved: number;
}

export type TransferDirection = 'upload' | 'download';
export type TransferSource = 'web' | 'cli' | 'share';
export type TransferPhase = 'transferring' | 'verifying' | 'storing';

export interface TransferSnapshot {
  id: string;
  owner: string;
  direction: TransferDirection;
  source: TransferSource;
  path: string;
  total_bytes: number | null;
  transferred_bytes: number;
  speed_bps: number;
  phase: TransferPhase;
  chunk_size?: number;
  chunks_done?: number;
  chunks_total?: number;
  started_at: number;
  updated_at: number;
}

export type TreeOp = 'mkdir' | 'mv' | 'rm' | 'restore' | 'purge';

export type ServerSseEvent =
  | {
      type: 'transfer:started';
      transfer: TransferSnapshot;
    }
  | {
      type: 'transfer:progress';
      id: string;
      owner: string;
      direction: TransferDirection;
      phase: TransferPhase;
      transferred_bytes: number;
      total_bytes: number | null;
      speed_bps: number;
      chunk_index?: number;
      chunks_done?: number;
      chunks_total?: number;
    }
  | {
      type: 'transfer:completed';
      id: string;
      owner: string;
      direction: TransferDirection;
      path: string;
      size: number;
      entry?: Entry;
    }
  | {
      type: 'transfer:cancelled';
      id: string;
      owner: string;
      direction: TransferDirection;
    }
  | {
      type: 'tree:changed';
      owner: string;
      op: TreeOp;
      paths: string[];
    }
  | {
      type: 'resync';
      missed: number;
    };

export interface TokenRecord {
  hash: string;
  owner: string;
  scope: string;
  kind: 'account' | 'file' | 'folder';
  session: boolean;
  created: number;
  label?: string;
  secret?: string;
}

export interface CreateTokenResponse {
  token: string;
  hash: string;
  owner: string;
  kind: 'account' | 'file' | 'folder';
  scope: string;
  created: number;
  label?: string;
  url: string | null;
}

export interface TrashItem {
  id: string;
  owner: string;
  path: string;
  deleted: number;
  expires_at: number | null;
  size: number;
  directory: boolean;
}

export interface PurgeResponse {
  trash: number;
  uploads: number;
  purged: boolean;
}

export interface DashboardUserStat {
  id: string;
  name: string;
  quota: number | null;
  used: number;
  buffer_reserved: number;
}

export interface DashboardResponse {
  buffer_size: number;
  users: DashboardUserStat[];
}

export interface ActivityRecord {
  time: number;
  client: string;
  method: string;
  path: string;
  status: number;
}

export interface ShareInfo {
  kind: 'file' | 'folder';
  root_name: string;
  subpath: string;
  directory: boolean;
  entry: Entry | null;
  entries: Entry[];
}

// UI-enriched item types
export type ChunkVisualState = 'queued' | 'inflight' | 'done';

export interface TransferItem {
  id: string;
  owner: string;
  direction: TransferDirection;
  source: TransferSource;
  path: string;
  name: string;
  folder: string;
  total_bytes: number | null;
  transferred_bytes: number;
  speed_bps: number;
  phase: TransferPhase | 'complete';
  chunk_size?: number;
  chunks_done?: number;
  chunks_total?: number;
  chunks: ChunkVisualState[];
  started_at: number;
  updated_at: number;
}
