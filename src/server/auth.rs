//! Setup, user administration, and password and token authentication.

use super::{App, Error, Result, bad, conflict, db, forbidden, missing, profile};
use crate::core::{self, Token, TokenKind, User, hex};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::{
	Json,
	extract::{Path, State},
	http::{HeaderMap, StatusCode},
};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

// Valid Argon2id hash used to equalize verification timing when a username does not exist.
const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRmb3JmbG9yYQ$nf65EOgLrQMR/uIPnA4rEsF5h7TKyQwu9U1bMCHGi/4";
const MAX_PASSWORD_BYTES: usize = 1024;

pub(super) fn random_secret() -> String {
	let mut bytes = [0u8; 32];
	rand::rng().fill(&mut bytes);
	hex::encode(bytes)
}

fn digest(secret: &str) -> String {
	hex::encode(Sha256::digest(secret.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
	if left.len() != right.len() {
		return false;
	}
	let mut diff = 0u8;
	for (&a, &b) in left.iter().zip(right.iter()) {
		diff |= std::hint::black_box(a ^ b);
	}
	std::hint::black_box(diff) == 0
}

fn password_hash_sync(password: &str) -> Result<String> {
	if password.len() < 8 {
		return Err(bad("password must contain at least eight bytes"));
	}
	if password.len() > MAX_PASSWORD_BYTES {
		return Err(bad("password must not exceed 1024 bytes"));
	}
	Argon2::default()
		.hash_password(password.as_bytes())
		.map(|h| h.to_string())
		.map_err(|e| Error(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e.to_string())))
}

async fn password_hash(password: String) -> Result<String> {
	if password.len() < 8 {
		return Err(bad("password must contain at least eight bytes"));
	}
	if password.len() > MAX_PASSWORD_BYTES {
		return Err(bad("password must not exceed 1024 bytes"));
	}
	tokio::task::spawn_blocking(move || password_hash_sync(&password)).await?
}

async fn verify_password(password: String, stored_hash: Option<String>) -> Result<bool> {
	tokio::task::spawn_blocking(move || {
		let candidate =
			stored_hash.as_deref().filter(|s| !s.is_empty()).unwrap_or(DUMMY_PASSWORD_HASH);
		let Ok(parsed) = PasswordHash::new(candidate) else {
			return false;
		};
		let too_long = password.len() > MAX_PASSWORD_BYTES;
		let input = &password.as_bytes()[..password.len().min(MAX_PASSWORD_BYTES)];
		let verified = Argon2::default().verify_password(input, &parsed).is_ok();
		!too_long && verified && stored_hash.is_some_and(|h| !h.is_empty())
	})
	.await
	.map_err(Into::into)
}
fn name_valid(name: &str) -> bool {
	!name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}

#[derive(Serialize)]
pub(super) struct UserView {
	id: String,
	name: String,
	admin: bool,
	quota: Option<u64>,
	retention: Option<String>,
}

impl From<User> for UserView {
	fn from(u: User) -> Self {
		Self { id: u.id, name: u.name, admin: u.admin, quota: u.quota, retention: u.retention }
	}
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Setup {
	code: String,
	name: String,
	password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Login {
	name: String,
	password: String,
}
#[derive(Serialize)]
struct LoginResponse {
	token: String,
	user: UserView,
}

async fn create_account(
	app: &App,
	name: String,
	password: String,
	admin: bool,
	quota: Option<u64>,
) -> Result<User> {
	if !name_valid(&name) {
		return Err(bad("invalid name"));
	}
	if app.db.find_user_by_name_async(&name).await?.is_some() {
		return Err(conflict("user name already exists"));
	}
	let hashed = password_hash(password).await?;
	let user = User {
		id: Uuid::new_v4().to_string(),
		name,
		password_hash: hashed,
		admin,
		quota,
		retention: None,
	};
	let dir = app.storage.join(&user.id);
	tokio::fs::create_dir_all(&dir).await?;
	profile::save(app, &user.id, &profile::Profile::new(user.name.clone())).await?;
	app.db.put_user_async(None, &user).await?;
	Ok(user)
}

fn session_cookie(app: &App, secret: &str, clear: bool) -> String {
	let secure =
		app.public_url.as_deref().is_some_and(|u| u.starts_with("https://")) || app.behind_proxy;
	let security = if secure { "; Secure" } else { "" };
	if clear {
		format!("flora_session=; HttpOnly{security}; SameSite=Strict; Path=/; Max-Age=0")
	} else {
		format!("flora_session={secret}; HttpOnly{security}; SameSite=Strict; Path=/")
	}
}

pub(super) async fn bootstrap(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
	let setup_required = app.setup_code.lock().await.is_some();
	let user = match authenticate(&app, &headers).await {
		Ok(p) if p.manage() => Some(UserView::from(p.user)),
		_ => None,
	};
	Ok(Json(serde_json::json!({
		"setup_required": setup_required,
		"user": user,
		"server": {
			"version": env!("CARGO_PKG_VERSION"),
			"chunk_size": app.chunk_size,
			"buffer_size": app.buffer_size,
			"trash_retention": app.trash_retention,
			"public_url": app.public_url,
		}
	})))
}

pub(super) async fn setup(
	State(app): State<App>,
	Json(request): Json<Setup>,
) -> Result<impl axum::response::IntoResponse> {
	let mut code = app.setup_code.lock().await;
	let expected = code.as_ref().ok_or_else(forbidden)?;
	let expected_digest = Sha256::digest(expected.as_bytes());
	let provided_digest = Sha256::digest(request.code.as_bytes());
	if !constant_time_eq(&expected_digest, &provided_digest) {
		return Err(forbidden());
	}
	let _guard = app.accounts.lock().await;
	let user = create_account(&app, request.name, request.password, true, None).await?;
	*code = None;
	drop(code);
	let (secret, _) = issue(&app, &user, String::new(), TokenKind::Account, true, None).await?;
	let cookie = session_cookie(&app, &secret, false);
	Ok((
		[(axum::http::header::SET_COOKIE, cookie)],
		Json(LoginResponse { token: secret, user: user.into() }),
	))
}

async fn issue(
	app: &App,
	user: &User,
	scope: String,
	kind: TokenKind,
	session: bool,
	label: Option<String>,
) -> Result<(String, Token)> {
	let secret = random_secret();
	let stored_secret = (kind != TokenKind::Account).then(|| secret.clone());
	let record = Token {
		hash: digest(&secret),
		owner: user.id.clone(),
		scope,
		kind,
		session,
		created: core::now(),
		label,
		secret: stored_secret,
	};
	app.db.put_token_async(&record).await?;
	Ok((secret, record))
}

pub(super) async fn login(
	State(app): State<App>,
	Json(request): Json<Login>,
) -> Result<impl axum::response::IntoResponse> {
	let matched = app.db.find_user_by_name_async(&request.name).await?;
	let stored_hash = matched.as_ref().map(|u| u.password_hash.clone());
	if !verify_password(request.password, stored_hash).await? {
		return Err(Error(StatusCode::UNAUTHORIZED, anyhow::anyhow!("invalid credentials")));
	}
	let user = matched
		.ok_or_else(|| Error(StatusCode::UNAUTHORIZED, anyhow::anyhow!("invalid credentials")))?;
	let (secret, _) = issue(&app, &user, String::new(), TokenKind::Account, true, None).await?;
	let cookie = session_cookie(&app, &secret, false);
	Ok((
		[(axum::http::header::SET_COOKIE, cookie)],
		Json(LoginResponse { token: secret, user: user.into() }),
	))
}

pub(super) async fn logout(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<impl axum::response::IntoResponse> {
	if let Ok(p) = authenticate(&app, &headers).await
		&& p.token.session
	{
		let _ = app.db.delete_token_async(&p.token.owner, &p.token.hash).await;
	}
	let cookie = session_cookie(&app, "", true);
	Ok((StatusCode::NO_CONTENT, [(axum::http::header::SET_COOKIE, cookie)]))
}

#[derive(Clone)]
pub(super) struct Principal {
	pub user: User,
	pub token: Token,
}

impl Principal {
	fn in_scope(&self, path: &str) -> bool {
		match self.token.kind {
			TokenKind::Account => true,
			TokenKind::File => path == self.token.scope,
			TokenKind::Folder => core::scoped(&self.token.scope, path),
		}
	}

	pub(super) fn permits(&self, path: &str, upload: bool) -> bool {
		if upload { self.token.kind == TokenKind::Account } else { self.in_scope(path) }
	}

	pub(super) fn manage(&self) -> bool {
		self.token.kind == TokenKind::Account
	}

	pub(super) fn admin(&self) -> bool {
		self.user.admin && self.token.kind == TokenKind::Account
	}
}

pub(super) async fn authenticate(app: &App, headers: &HeaderMap) -> Result<Principal> {
	let bearer = headers
		.get(axum::http::header::AUTHORIZATION)
		.and_then(|v| v.to_str().ok())
		.and_then(|s| s.strip_prefix("Bearer "));
	let cookie =
		headers.get(axum::http::header::COOKIE).and_then(|v| v.to_str().ok()).and_then(|s| {
			s.split(';').map(str::trim).find_map(|part| part.strip_prefix("flora_session="))
		});
	let secret = bearer.or(cookie).ok_or_else(|| {
		Error(StatusCode::UNAUTHORIZED, anyhow::anyhow!("authentication required"))
	})?;
	authenticate_token(app, secret).await
}

pub(super) async fn authenticate_token(app: &App, secret: &str) -> Result<Principal> {
	let store = app.db.clone();
	let hash = digest(secret);
	tokio::task::spawn_blocking(move || -> Result<Principal> {
		let token: Token = store
			.get(&db::token(&hash))?
			.ok_or_else(|| Error(StatusCode::UNAUTHORIZED, anyhow::anyhow!("invalid token")))?;
		let user = store.get(&db::user(&token.owner))?.ok_or_else(forbidden)?;
		Ok(Principal { user, token })
	})
	.await?
}

pub(super) async fn me(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
	let p = authenticate(&app, &headers).await?;
	if !p.manage() {
		return Err(forbidden());
	}
	Ok(Json(
		serde_json::json!({"user": UserView::from(p.user), "scope": p.token.scope, "kind": p.token.kind}),
	))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NewToken {
	owner: Option<String>,
	scope: Option<String>,
	kind: Option<TokenKind>,
	label: Option<String>,
}

pub(super) async fn create_token(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<NewToken>,
) -> Result<Json<serde_json::Value>> {
	let p = authenticate(&app, &headers).await?;
	if !p.manage() {
		return Err(forbidden());
	}
	let owner = super::files::owner(&app, &p, req.owner.as_deref()).await?;
	if owner != p.user.id && !p.admin() {
		return Err(forbidden());
	}
	let user: User = app.db.get_async(&db::user(&owner)).await?.ok_or_else(missing)?;
	let scope = core::clean_path(req.scope.as_deref().unwrap_or(""), true).map_err(bad)?;
	let kind = match (scope.is_empty(), req.kind) {
		(true, None | Some(TokenKind::Account)) => TokenKind::Account,
		(_, Some(TokenKind::Folder)) => TokenKind::Folder,
		(false, Some(TokenKind::File)) => TokenKind::File,
		(false, None) => {
			if app.db.get_async::<core::FileRecord>(&db::file(&owner, &scope)).await?.is_some() {
				TokenKind::File
			} else {
				TokenKind::Folder
			}
		}
		_ => return Err(bad("scope kind must be account, file or folder and match the scope")),
	};
	if kind != TokenKind::Account {
		let target = core::safe_join_async(&app.storage.join(&owner), &scope).await.map_err(bad)?;
		let metadata = tokio::fs::metadata(target).await.map_err(|_| missing())?;
		if (kind == TokenKind::File && !metadata.is_file())
			|| (kind == TokenKind::Folder && !metadata.is_dir())
		{
			return Err(bad("scope does not match token kind"));
		}
	}
	let label = req.label.map(|l| l.trim().to_owned()).filter(|l| !l.is_empty());
	if let Some(l) = &label
		&& (l.len() > 128 || l.chars().any(char::is_control))
	{
		return Err(bad("invalid token label"));
	}
	let (token, record) = issue(&app, &user, scope, kind, false, label).await?;
	let url = if kind == TokenKind::Account {
		None
	} else {
		app.public_url.as_ref().map(|base| format!("{}/t/{token}", base.trim_end_matches('/')))
	};
	Ok(Json(serde_json::json!({
		"token": token,
		"hash": record.hash,
		"owner": record.owner,
		"kind": record.kind,
		"scope": record.scope,
		"created": record.created,
		"label": record.label,
		"url": url,
	})))
}

pub(super) async fn list_tokens(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<Json<Vec<Token>>> {
	let p = authenticate(&app, &headers).await?;
	if !p.manage() {
		return Err(forbidden());
	}
	let tokens = if p.admin() {
		app.db.list_async("token/").await?
	} else {
		app.db.list_user_tokens_async(&p.user.id).await?
	};
	Ok(Json(tokens))
}

pub(super) async fn revoke_token(
	State(app): State<App>,
	Path(hash): Path<String>,
	headers: HeaderMap,
) -> Result<StatusCode> {
	let p = authenticate(&app, &headers).await?;
	if !p.manage() {
		return Err(forbidden());
	}
	let token: Token = app.db.get_async(&db::token(&hash)).await?.ok_or_else(missing)?;
	if token.owner != p.user.id && !p.admin() {
		return Err(forbidden());
	}
	app.db.delete_token_async(&token.owner, &hash).await?;
	Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NewUser {
	name: String,
	password: String,
	#[serde(default)]
	admin: bool,
	quota: Option<u64>,
}

pub(super) async fn list_users(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<Json<Vec<UserView>>> {
	let p = authenticate(&app, &headers).await?;
	if !p.admin() {
		return Err(forbidden());
	}
	let users: Vec<User> = app.db.list_async("user/").await?;
	Ok(Json(users.into_iter().map(UserView::from).collect()))
}

pub(super) async fn create_user(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<NewUser>,
) -> Result<Json<UserView>> {
	let p = authenticate(&app, &headers).await?;
	if !p.admin() {
		return Err(forbidden());
	}
	let _guard = app.accounts.lock().await;
	Ok(Json(create_account(&app, req.name, req.password, req.admin, req.quota).await?.into()))
}

fn nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
	D: serde::Deserializer<'de>,
	T: Deserialize<'de>,
{
	Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateUser {
	name: Option<String>,
	password: Option<String>,
	admin: Option<bool>,
	#[serde(default, deserialize_with = "nullable")]
	quota: Option<Option<u64>>,
	#[serde(default, deserialize_with = "nullable")]
	retention: Option<Option<String>>,
}

pub(super) async fn update_user(
	State(app): State<App>,
	Path(id): Path<String>,
	headers: HeaderMap,
	Json(req): Json<UpdateUser>,
) -> Result<Json<UserView>> {
	let p = authenticate(&app, &headers).await?;
	if !p.admin() && (p.user.id != id || req.admin.is_some() || req.quota.is_some()) {
		return Err(forbidden());
	}
	if !p.manage() {
		return Err(forbidden());
	}
	let new_password_hash = match req.password {
		Some(password) => Some(password_hash(password).await?),
		None => None,
	};
	let _guard = app.accounts.lock().await;
	let _user_guard = app.user_locks.lock(&id).await;
	let mut user: User = app.db.get_async(&db::user(&id)).await?.ok_or_else(missing)?;
	let old_name = user.name.clone();
	if let Some(name) = req.name {
		if !name_valid(&name) {
			return Err(bad("invalid name"));
		}
		if let Some(existing) = app.db.find_user_by_name_async(&name).await?
			&& existing.id != id
		{
			return Err(conflict("user name already exists"));
		}
		user.name = name;
	}
	if let Some(hashed) = new_password_hash {
		user.password_hash = hashed;
	}
	if let Some(admin) = req.admin {
		if user.admin && !admin {
			let users: Vec<User> = app.db.list_async("user/").await?;
			if !users.iter().any(|u| u.id != user.id && u.admin) {
				return Err(conflict("cannot remove the last administrator"));
			}
		}
		user.admin = admin;
	}
	if let Some(quota) = req.quota {
		user.quota = quota;
	}
	if let Some(retention) = req.retention {
		if let Some(value) = &retention {
			core::duration(value).map_err(bad)?;
		}
		user.retention = retention;
	}
	let mut personal = profile::load(&app, &id).await?;
	personal.name = user.name.clone();
	personal.settings.retention = user.retention.clone();
	profile::save(&app, &id, &personal).await?;
	app.db.put_user_async(Some(&old_name), &user).await?;
	Ok(Json(user.into()))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn password_hashes_verify_and_reject_wrong_passwords() {
		let hash = password_hash_sync("valid password").ok().expect("hash password");
		let parsed = PasswordHash::new(&hash).expect("parse password hash");
		assert!(Argon2::default().verify_password(b"valid password", &parsed).is_ok());
		assert!(Argon2::default().verify_password(b"wrong password", &parsed).is_err());
		assert!(password_hash_sync("short").is_err());
	}

	#[test]
	fn existing_argon2id_hashes_remain_usable() {
		let hash =
			"$argon2id$v=19$m=256,t=2,p=1$c29tZXNhbHQ$nf65EOgLrQMR/uIPnA4rEsF5h7TKyQwu9U1bMCHGi/4";
		let parsed = PasswordHash::new(hash).expect("parse existing hash");
		assert!(Argon2::default().verify_password(b"password", &parsed).is_ok());
		assert!(Argon2::default().verify_password(b"wrong password", &parsed).is_err());
	}
}
