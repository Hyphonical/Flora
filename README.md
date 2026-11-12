# Flora

Self-hosted permanent file storage in a single Rust binary. `flora` runs both the HTTP server (`flora serve`) and the file transfer CLI (`flora upload`, `flora download`, `flora ls`, `flora mv`, `flora rm`, `flora cat`).

- **Single binary** — server, CLI, embedded Web UI, and embedded client installers in one executable.
- **Resumable chunked transfers** — parallel chunked uploads and ranged `.partial` downloads verified end-to-end with XXH3-128.
- **Plain files on disk** — per-user UUID storage directories with crash-safe trash and rebuildable Fjall metadata.
- **Scoped sharing** — revocable account tokens and read-only capability URLs (`/t/<token>`) for individual files or folders.

> **Status:** Local storage, HTTP API, CLI, and the Web UI (Svelte 5, embedded into the binary) are implemented. Remote storage backends (WebDAV, SFTP, FTP) are not included yet. Keep backups and place an HTTPS reverse proxy in front of internet-facing instances.

---

## Quick start

### 1. Start the server

With Cargo (build the web UI first so it is embedded into the binary):

```sh
bun install --cwd web && bun run --cwd web build
cargo build --locked --release
./target/release/flora serve
```

Or with Docker Compose (the image builds the web UI and the Rust binary):

```sh
docker compose up -d --build
```

When no administrator exists, `flora serve` logs a one-time `setup_code` on startup.

The server then serves the web UI at `http://127.0.0.1:8080/` — including the public
share view at `/t/<share_token>` — while the API stays under `/api`.

For frontend development, run the Vite dev server next to a local backend; it proxies
`/api`, `/t/`, and `/install.*` to `http://127.0.0.1:8080`:

```sh
flora serve          # terminal 1
bun run --cwd web dev # terminal 2 → http://127.0.0.1:5173
```

### 2. Create the first admin and an account token

```sh
export FLORA_SERVER="http://127.0.0.1:8080"

# Claim first-run setup using the printed setup_code
curl -sS -X POST "$FLORA_SERVER/api/setup" \
  -H "Content-Type: application/json" \
  -d '{"code":"<setup_code>","name":"alice","password":"correct-horse-battery"}'

# Sign in to receive a session/account token
curl -sS -X POST "$FLORA_SERVER/api/login" \
  -H "Content-Type: application/json" \
  -d '{"name":"alice","password":"correct-horse-battery"}'

# Optional: mint a dedicated non-session CLI token
curl -sS -X POST "$FLORA_SERVER/api/tokens" \
  -H "Authorization: Bearer <login_token>" \
  -H "Content-Type: application/json" \
  -d '{"kind":"account"}'
```

### 3. Sign in from the CLI and transfer files

```sh
flora login http://127.0.0.1:8080

flora upload report.pdf docs/
flora up photos/ --on-conflict rename
cat dump.sql | flora upload - backup/ --name dump.sql

flora ls docs/
flora download docs/report.pdf -o ./report.pdf
flora down https://files.example.com/t/<share_token>
```

---

## Installing the CLI

Install from Git (every Flora server forwards read-only Smart-HTTP Git requests to the upstream repository) or from a local checkout:

```sh
cargo install --git https://flora.hypho.dev --locked
# or locally:
cargo install --path . --locked
```

Every Flora server also serves one-command installers at `/install.sh` and `/install.ps1` that fetch the latest GitHub Release binary for the current OS and architecture (`linux-x86_64`, `linux-aarch64`, `macos-x86_64`, `macos-aarch64`, `windows-x86_64`, `windows-aarch64`):

```sh
# Linux / macOS
curl -fsSL https://flora.hypho.dev/install.sh | sh

# Windows (PowerShell)
irm https://flora.hypho.dev/install.ps1 | iex
```
---

## CLI reference

All human-facing progress and status output is written to `stderr`; file bytes (`flora cat`, `flora download -o -`) are written cleanly to `stdout`. When `stderr` is not a terminal, Flora switches from colored progress bars to plain UTC-timestamped log lines.

| Command | Alias | Usage | Description |
|---|---|---|---|
| `upload` | `up` | `flora upload <file\|dir\|-> [dest/]` | Upload a file, directory tree, or `stdin` (`--name` required for `-`). |
| `download` | `down` | `flora download <path\|url> [-o file\|-]` | Download an authenticated path or `/t/<token>` share URL. |
| `ls` | — | `flora ls [path]` | List files and directories. |
| `mv` | — | `flora mv <from> <to>` | Move or rename a file or directory. |
| `rm` | — | `flora rm <path>` | Move a file or directory to trash. |
| `cat` | — | `flora cat <path\|url>` | Stream remote file bytes to `stdout`. |
| `login` | — | `flora login <url>` | Verify an account token and save credentials to the user config dir. |
| `serve` | — | `flora serve` | Run the HTTP storage server. |

### Transfer options & behavior

- **Conflict policy (`--on-conflict`):** `reject` (default), `rename`, or `replace`.
- **Concurrency (`--parallel`):** concurrent chunk/range requests per transfer (`1..=64`, default `4`).
- **Directory uploads:** preserve relative paths, create empty folders, and skip files whose remote size and XXH3-128 checksum already match. Symbolic links are skipped/rejected rather than dereferenced.
- **Resumable uploads:** interrupted uploads stay in the server buffer until retention expires, reusing already-verified chunks on retry.
- **Resumable downloads:** in-progress files are written in order to `<name>.partial` alongside the destination, resumed via HTTP byte ranges with `ETag`/`If-Range` checks, verified against the server's XXH3-128 `ETag`, and atomically renamed without overwriting existing files.
- **Credentials:** `flora login` saves `<config_dir>/flora/credentials` (`0600` on Unix). `--server` / `FLORA_SERVER` and `--token` / `FLORA_TOKEN` override saved credentials; Flora refuses to send a saved token if `--server` points to a different host without an explicit token.

---

## Server configuration

Every flag has a matching `FLORA_<FLAG_NAME>` environment variable (for example `--buffer-size` ↔ `FLORA_BUFFER_SIZE`). Precedence is **CLI flag > environment variable > default**.

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--listen` | `FLORA_LISTEN` | `0.0.0.0:8080` | TCP bind address. |
| `--public-url` | `FLORA_PUBLIC_URL` | *unset* | Base `http(s)://` URL used for share links and `Secure` cookies. |
| `--data` | `FLORA_DATA` | `./data` | Fjall database and thumbnail cache. |
| `--buffer` | `FLORA_BUFFER` | `./buffer` | Staging directory for in-progress chunked uploads. |
| `--storage` | `FLORA_STORAGE` | `./storage` | Permanent per-user file storage. |
| `--buffer-size` | `FLORA_BUFFER_SIZE` | `3GB` | Total upload buffer capacity **and** maximum single-file size. |
| `--chunk-size` | `FLORA_CHUNK_SIZE` | `8MB` | Maximum chunk size for multi-part uploads. |
| `--disk-limit` | `FLORA_DISK_LIMIT` | *unset* | Ceiling on used filesystem space (`data`, `buffer`, `storage` must share one volume). |
| `--buffer-retention` | `FLORA_BUFFER_RETENTION` | `7d` | How long abandoned uploads stay buffered (`never` to disable expiry). |
| `--trash-retention` | `FLORA_TRASH_RETENTION` | `7d` | Default trash retention (`never` or duration; overridable per user). |
| `--upload-concurrency` | `FLORA_UPLOAD_CONCURRENCY` | `16` | Server-wide semaphore on concurrent chunk writes. |
| `--rate-limit` | `FLORA_RATE_LIMIT` | `600` | Per-client requests per minute (authenticated chunk/range data requests exempt). |
| `--login-rate-limit` | `FLORA_LOGIN_RATE_LIMIT` | `10` | Per-client `/api/login` and `/api/setup` attempts per minute. |
| `--behind-proxy` | `FLORA_BEHIND_PROXY` | `false` | Read `CF-Connecting-IP`, `X-Real-IP`, or `X-Forwarded-For` from trusted proxies and mark session cookies `Secure`. |
| `--trusted-proxies` | `FLORA_TRUSTED_PROXIES` | *loopback* | Comma-separated proxy IPs or CIDRs allowed to supply forwarded client IPs. |
| `--log` | `FLORA_LOG` | `info` | Diagnostic log level (`error`, `warn`, `info`, `debug`, `trace`). |

**Value formats:**
- **Sizes:** decimal (`500MB`, `3GB`), binary (`512MiB`, `3GiB`), raw bytes, or whole-number disk percentages (`10%`, resolved once at startup; `--chunk-size` accepts fixed byte units only).
- **Durations:** positive combinations of `w`, `d`, `h` with each unit used at most once (e.g. `2w1d12h`), or `never`.

---

## Storage layout & recovery

`data/`, `buffer/`, and `storage/` must be separate, non-nested directories:

```text
data/
├── fjall/                  users, roles, quotas, token hashes, file index, activity
└── thumbnails/<uuid>/      on-demand WebP thumbnail cache

buffer/
└── <uuid>/<upload-id>/     in-flight upload chunks (space reserved upfront)

storage/
└── <uuid>/                 one directory per user, named by UUID
    ├── .flora              JSON profile: username, personal retention, trash manifest
    ├── .trash/             deleted files and directories awaiting expiry or restore
    └── <files & folders>   stored verbatim on disk
```

- **Crash & index recovery:** On startup, Flora finishes interrupted `.flora` writes, recovers in-progress trash/upload states, and reconciles the Fjall file index against `storage/`. If Fjall is lost, user directories, live files, and trash entries rebuild from `storage/`, while passwords, admin flags, quotas, tokens, and activity history must be reconfigured.
- **Cross-platform filenames:** Paths use `/` separators and enforce portable rules across Linux, macOS, and Windows: max 255 UTF-8 bytes per segment, no control characters or `<>:"\|?*`, no trailing spaces or dots, no Windows device names (`CON`, `NUL`, `COM1`, `LPT1`, etc.), and `.flora` / `.flora.tmp` / `.trash` are reserved.
- **Reverse proxies:** Flora serves HTTP directly. Terminate TLS at a reverse proxy, and only enable `--behind-proxy` when the proxy strips or overwrites forwarded IP headers (`CF-Connecting-IP`, `X-Real-IP`, `X-Forwarded-For`) and untrusted clients cannot bypass it.

---

## HTTP API

Authenticated endpoints accept `Authorization: Bearer <account_token>` or the `flora_session` cookie issued by `POST /api/login` / `POST /api/setup`. Account and session tokens are stored strictly as SHA-256 digests; `file` and `folder` share tokens also retain their capability URL secret so links can be copied again from the UI.

| Method & Path | Auth | Purpose |
|---|---|---|
| `GET /api/bootstrap` | Public / Optional | Initial state (`setup_required`, optional signed-in `user`, and server limits). |
| `POST /api/setup` | Setup code | Create the initial admin (`{ "code", "name", "password" }`) and start a session. |
| `POST /api/login` | Public | Verify password (Argon2id), set `flora_session`, and return `{ "token", "user" }`. |
| `POST /api/logout` | Public / Account | Revoke current `flora_session` token and clear the session cookie. |
| `GET /api/me` | Account | Inspect current user and token scope. |
| `GET, POST /api/tokens` | Account | List tokens or create `"account"`, `"file"`, or `"folder"` tokens (`scope`, `label`, `owner`). |
| `DELETE /api/tokens/{hash}` | Account | Revoke a token by its hash. |
| `GET, POST /api/users` | Account / Admin | List users or create a user (`POST` requires admin). |
| `PATCH /api/users/{id}` | Account / Admin | Update password/retention (self or admin) or name/admin/quota (admin). |
| `POST /api/uploads` | Account | Reserve or resume a chunked upload (`path`, `size`, `conflict`, optional admin `owner`). |
| `GET, DELETE /api/uploads/{id}` | Account | Inspect received chunks or cancel an upload. |
| `PUT /api/uploads/{id}/{number}` | Account | Upload chunk `number` with `Content-Length`. |
| `POST /api/uploads/{id}/complete` | Account | Verify whole-file XXH3-128 (`{ "size", "hash" }`) and commit to `storage/`. |
| `PUT /api/upload?path=…` | Account | Single-request streaming upload (`conflict`, `owner`, optional `X-XXH3-128` header). |
| `GET /api/transfers` | Account | Active upload and download snapshots (`?owner=`, or `?all=1` for admins). |
| `GET /api/events` | Account | SSE stream (`transfer:started`, `transfer:progress`, `transfer:completed`, `transfer:cancelled`, `tree:changed`). |
| `GET, POST, PATCH, DELETE /api/files` | Account | Single-pass directory list with folder stats & share links (`?path=`), mkdir, move, or trash. |
| `GET /api/pastes` | Account | Flat list of text and code files (`<= 2 MiB`) sorted newest-first. |
| `GET /api/download?path=…` | Account / Scoped | Download a file (`Range`, XXH3-128 `ETag`, `?download=1` attachment, `?preview=1` untracked). |
| `GET /api/share/{token}[/{*path}]` | Capability URL | JSON metadata for public file or folder share views. |
| `GET /t/{token}`, `/t/{token}/{*path}` | Capability URL | Serves Web UI share view to browsers (`Accept: text/html`) or streams raw bytes (`curl`, CLI, `?raw=1`, `?download=1`). |
| `GET /api/trash` | Account | List trashed items with sizes, directory flags, and `expires_at`. |
| `POST /api/trash/{id}/restore` | Account | Restore a trashed file or folder to its original path. |
| `DELETE /api/trash/{id}` | Account | Permanently delete a single trashed item. |
| `POST /api/purge` | Account | Preview (`{"confirm": false}`) or execute (`{"confirm": true}`) upload & trash cleanup. |
| `GET /api/search?q=…` | Account | On-demand case-insensitive filename + UTF-8 content search (up to 16 MiB per file). |
| `GET /api/thumbnail?path=…` | Account | Generate and cache an image thumbnail (`JPEG`, `PNG`, `WebP`, `GIF`). |
| `GET /api/dashboard` | Account | Storage usage, quota, and reserved buffer space. |
| `GET /api/activity` | Admin | Recent mutating API events (`?limit=`, max `1000`). |
| `GET /install.sh`, `/install.ps1` | Public | Client install scripts fetching the latest GitHub Release build. |
| `GET /api/client/{target}` | Public | `302` redirect to the latest GitHub Release binary for `<os>-<arch>`. |
| `GET /info/refs`, `POST /git-upload-pack` | Public | Read-only Smart-HTTP Git proxy to GitHub (`cargo install --git`). |
### Standard HTTP clients (`curl`)

Scripts and external tools can upload and resume downloads without the `flora` CLI:

```sh
# Stream an upload in a single request
curl -f -H "Authorization: Bearer $FLORA_TOKEN" \
  --upload-file report.pdf \
  "$FLORA_SERVER/api/upload?path=docs/report.pdf&conflict=rename"

# Download (or resume with -C -)
curl -fL -H "Authorization: Bearer $FLORA_TOKEN" \
  -C - -o report.pdf \
  "$FLORA_SERVER/api/download?path=docs/report.pdf"
```

> Single-request uploads (`PUT /api/upload`) pass through reverse-proxy request-body limits (such as Cloudflare's) in one body; use `flora upload` for multi-request chunked transfers that stay under per-request proxy limits and verify whole-file XXH3-128 automatically.

---

## Development

```sh
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```
