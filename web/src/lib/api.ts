// Typed fetch wrapper & endpoint client for Flora's backend API.

import type {
  ActivityRecord,
  AuthResponse,
  BootstrapResponse,
  ConflictMode,
  CreateTokenResponse,
  DashboardResponse,
  Entry,
  FileRecord,
  PurgeResponse,
  ShareInfo,
  TokenRecord,
  TransferSnapshot,
  TrashItem,
  Upload,
  UploadChunkInfo,
  UserView
} from './types';

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

async function parseError(res: Response): Promise<ApiError> {
  let message = `${res.status} ${res.statusText}`;
  try {
    const data = await res.json();
    if (data && typeof data.error === 'string' && data.error.trim()) {
      message = data.error;
    }
  } catch {
    // Ignore non-JSON error bodies
  }
  return new ApiError(res.status, message);
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (
    init.body !== undefined &&
    typeof init.body === 'string' &&
    !headers.has('Content-Type')
  ) {
    headers.set('Content-Type', 'application/json');
  }

  const res = await fetch(path, {
    ...init,
    headers,
    credentials: 'same-origin'
  });

  if (!res.ok) {
    throw await parseError(res);
  }

  if (res.status === 204) {
    return undefined as T;
  }

  const contentType = res.headers.get('content-type') || '';
  if (contentType.includes('application/json')) {
    return (await res.json()) as T;
  }
  return (await res.text()) as unknown as T;
}

function withQuery(base: string, params: Record<string, string | number | boolean | undefined>): string {
  const search = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== '') {
      search.set(k, String(v));
    }
  }
  const qs = search.toString();
  return qs ? `${base}?${qs}` : base;
}

function encodePathSegments(subpath: string): string {
  return subpath
    .split('/')
    .filter(Boolean)
    .map((seg) => encodeURIComponent(seg))
    .join('/');
}

export const api = {
  // Bootstrap & Auth
  bootstrap(): Promise<BootstrapResponse> {
    return request<BootstrapResponse>('/api/bootstrap');
  },

  setup(code: string, name: string, password: string): Promise<AuthResponse> {
    return request<AuthResponse>('/api/setup', {
      method: 'POST',
      body: JSON.stringify({ code, name, password })
    });
  },

  login(name: string, password: string): Promise<AuthResponse> {
    return request<AuthResponse>('/api/login', {
      method: 'POST',
      body: JSON.stringify({ name, password })
    });
  },

  logout(): Promise<void> {
    return request<void>('/api/logout', { method: 'POST' });
  },

  // Files & Folders
  listFiles(folder: string, owner?: string): Promise<Entry[]> {
    return request<Entry[]>(withQuery('/api/files', { path: folder || undefined, owner }));
  },

  createFolder(path: string, owner?: string): Promise<void> {
    const body: Record<string, string> = { path };
    if (owner) body.owner = owner;
    return request<void>('/api/files', {
      method: 'POST',
      body: JSON.stringify(body)
    });
  },

  moveFile(from: string, to: string, owner?: string): Promise<void> {
    const body: Record<string, string> = { from, to };
    if (owner) body.owner = owner;
    return request<void>('/api/files', {
      method: 'PATCH',
      body: JSON.stringify(body)
    });
  },

  deleteFile(path: string, owner?: string): Promise<void> {
    return request<void>(withQuery('/api/files', { path, owner }), {
      method: 'DELETE'
    });
  },

  search(q: string, owner?: string): Promise<FileRecord[]> {
    return request<FileRecord[]>(withQuery('/api/search', { q, owner }));
  },

  // Pastes & Text Content
  listPastes(owner?: string): Promise<Entry[]> {
    return request<Entry[]>(withQuery('/api/pastes', { owner }));
  },

  async fetchTextPreview(path: string, owner?: string): Promise<string> {
    const url = withQuery('/api/download', { path, preview: 1, owner });
    const res = await fetch(url, { credentials: 'same-origin' });
    if (!res.ok) {
      throw await parseError(res);
    }
    return res.text();
  },

  async putTextFile(
    path: string,
    content: string,
    conflict: ConflictMode = 'replace',
    owner?: string
  ): Promise<FileRecord> {
    const url = withQuery('/api/upload', { path, conflict, owner });
    const blob = new Blob([content], { type: 'text/plain; charset=utf-8' });
    const res = await fetch(url, {
      method: 'PUT',
      headers: { 'Content-Type': 'text/plain; charset=utf-8' },
      body: blob,
      credentials: 'same-origin'
    });
    if (!res.ok) {
      throw await parseError(res);
    }
    return (await res.json()) as FileRecord;
  },

  // Chunked Uploads
  reserveUpload(
    path: string,
    size: number,
    conflict: ConflictMode,
    owner?: string
  ): Promise<Upload> {
    const body: Record<string, unknown> = { path, size, conflict };
    if (owner) body.owner = owner;
    return request<Upload>('/api/uploads', {
      method: 'POST',
      body: JSON.stringify(body)
    });
  },

  async uploadChunk(
    uploadId: string,
    index: number,
    blob: Blob,
    signal?: AbortSignal
  ): Promise<UploadChunkInfo> {
    const res = await fetch(`/api/uploads/${encodeURIComponent(uploadId)}/${index}`, {
      method: 'PUT',
      body: blob,
      credentials: 'same-origin',
      signal
    });
    if (!res.ok) {
      throw await parseError(res);
    }
    return (await res.json()) as UploadChunkInfo;
  },

  completeUpload(uploadId: string, size: number, signal?: AbortSignal): Promise<FileRecord> {
    return request<FileRecord>(`/api/uploads/${encodeURIComponent(uploadId)}/complete`, {
      method: 'POST',
      body: JSON.stringify({ size }),
      signal
    });
  },

  cancelUpload(uploadId: string): Promise<void> {
    return request<void>(`/api/uploads/${encodeURIComponent(uploadId)}`, {
      method: 'DELETE'
    });
  },

  // Transfers
  listTransfers(opts?: { all?: boolean; owner?: string }): Promise<TransferSnapshot[]> {
    return request<TransferSnapshot[]>(
      withQuery('/api/transfers', {
        all: opts?.all ? 1 : undefined,
        owner: opts?.owner
      })
    );
  },

  // Tokens & Share Links
  listTokens(): Promise<TokenRecord[]> {
    return request<TokenRecord[]>('/api/tokens');
  },

  createToken(params: {
    scope?: string;
    kind?: 'account' | 'file' | 'folder';
    label?: string;
    owner?: string;
  }): Promise<CreateTokenResponse> {
    const body: Record<string, unknown> = {};
    if (params.scope !== undefined) body.scope = params.scope;
    if (params.kind !== undefined) body.kind = params.kind;
    if (params.label !== undefined) body.label = params.label;
    if (params.owner !== undefined) body.owner = params.owner;
    return request<CreateTokenResponse>('/api/tokens', {
      method: 'POST',
      body: JSON.stringify(body)
    });
  },

  revokeToken(hash: string): Promise<void> {
    return request<void>(`/api/tokens/${encodeURIComponent(hash)}`, {
      method: 'DELETE'
    });
  },

  // Trash & Purge
  listTrash(owner?: string): Promise<TrashItem[]> {
    return request<TrashItem[]>(withQuery('/api/trash', { owner }));
  },

  restoreTrash(id: string, owner?: string): Promise<void> {
    return request<void>(
      withQuery(`/api/trash/${encodeURIComponent(id)}/restore`, { owner }),
      { method: 'POST' }
    );
  },

  deleteTrashItem(id: string, owner?: string): Promise<void> {
    return request<void>(withQuery(`/api/trash/${encodeURIComponent(id)}`, { owner }), {
      method: 'DELETE'
    });
  },

  purge(params: { confirm: boolean; owner?: string; all?: boolean }): Promise<PurgeResponse> {
    const body: Record<string, unknown> = { confirm: params.confirm };
    if (params.owner) body.owner = params.owner;
    if (params.all) body.all = params.all;
    return request<PurgeResponse>('/api/purge', {
      method: 'POST',
      body: JSON.stringify(body)
    });
  },

  // Dashboard, Users & Activity
  dashboard(): Promise<DashboardResponse> {
    return request<DashboardResponse>('/api/dashboard');
  },

  listUsers(): Promise<UserView[]> {
    return request<UserView[]>('/api/users');
  },

  createUser(params: {
    name: string;
    password: string;
    admin?: boolean;
    quota?: number | null;
  }): Promise<UserView> {
    return request<UserView>('/api/users', {
      method: 'POST',
      body: JSON.stringify(params)
    });
  },

  updateUser(
    id: string,
    params: {
      name?: string;
      password?: string;
      admin?: boolean;
      quota?: number | null;
      retention?: string | null;
    }
  ): Promise<UserView> {
    return request<UserView>(`/api/users/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(params)
    });
  },

  activity(limit = 200): Promise<ActivityRecord[]> {
    return request<ActivityRecord[]>(withQuery('/api/activity', { limit }));
  },

  // Public Share
  getShareInfo(token: string, subpath = ''): Promise<ShareInfo> {
    const cleanSub = encodePathSegments(subpath);
    const url = cleanSub
      ? `/api/share/${encodeURIComponent(token)}/${cleanSub}`
      : `/api/share/${encodeURIComponent(token)}`;
    return request<ShareInfo>(url);
  },

  async fetchShareText(token: string, subpath = ''): Promise<string> {
    const cleanSub = encodePathSegments(subpath);
    const base = cleanSub
      ? `/t/${encodeURIComponent(token)}/${cleanSub}`
      : `/t/${encodeURIComponent(token)}`;
    const res = await fetch(`${base}?raw=1&preview=1`);
    if (!res.ok) {
      throw await parseError(res);
    }
    return res.text();
  },

  // URL Builders
  downloadUrl(path: string, owner?: string): string {
    return withQuery('/api/download', { path, download: 1, owner });
  },

  previewUrl(path: string, owner?: string): string {
    return withQuery('/api/download', { path, preview: 1, owner });
  },

  thumbnailUrl(path: string, owner?: string): string {
    return withQuery('/api/thumbnail', { path, owner });
  },

  shareDownloadUrl(token: string, subpath = ''): string {
    const cleanSub = encodePathSegments(subpath);
    const base = cleanSub
      ? `/t/${encodeURIComponent(token)}/${cleanSub}`
      : `/t/${encodeURIComponent(token)}`;
    return `${base}?download=1`;
  },

  sharePreviewUrl(token: string, subpath = ''): string {
    const cleanSub = encodePathSegments(subpath);
    const base = cleanSub
      ? `/t/${encodeURIComponent(token)}/${cleanSub}`
      : `/t/${encodeURIComponent(token)}`;
    return `${base}?raw=1&preview=1`;
  }
};
