//! Setup, user administration, and password and token authentication.

use super::{App, Error, Result, bad, conflict, db, forbidden, missing, profile};
use crate::core::{self, Token, User, hex};
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
const DUMMY_PASSWORD_HASH: &str =
	"$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRmb3JmbG9yYQ$nf65EOgLrQMR/uIPnA4rEsF5h7TKyQwu9U1bMCHGi/4";

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
		diff |= a ^ b;
	}
	diff == 0
}

fn password_hash_sync(password: &str) -> Result<String> {
	if password.len() < 8 {
		return Err(bad("password must contain at least eight bytes"));
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
	tokio::task::spawn_blocking(move || password_hash_sync(&password)).await?
}

async fn verify_password(password: String, stored_hash: Option<String>) -> Result<bool> {
	tokio::task::spawn_blocking(move || {
		let candidate = stored_hash
			.as_deref()
			.filter(|s| !s.is_empty())
			.unwrap_or(DUMMY_PASSWORD_HASH);
		let Ok(parsed) = PasswordHash::new(candidate) else {
			return false;
		};
		let verified = Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok();
		verified && stored_hash.is_some_and(|h| !h.is_empty())
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
pub(super) struct Setup {
	code: String,
	name: String,
	password: String,
}

#[derive(Deserialize)]
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
	let users: Vec<User> = app.db.list("user/")?;
	if users.iter().any(|u| u.name.eq_ignore_ascii_case(&name)) {
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
	app.db.put(&db::user(&user.id), &user)?;
	Ok(user)
}

pub(super) async fn setup(
	State(app): State<App>,
	Json(request): Json<Setup>,
) -> Result<Json<serde_json::Value>> {
	let mut code = app.setup_code.lock().await;
	let expected = code.as_ref().ok_or_else(forbidden)?;
	let expected_digest = Sha256::digest(expected.as_bytes());
	let provided_digest = Sha256::digest(request.code.as_bytes());
	if !constant_time_eq(&expected_digest, &provided_digest) {
		return Err(forbidden());
	}
	let _guard = app.mutation.lock().await;
	let user = create_account(&app, request.name, request.password, true, None).await?;
	*code = None;
	Ok(Json(serde_json::json!({"user": UserView::from(user)})))
}

fn issue(app: &App, user: &User, scope: String, kind: String, session: bool) -> Result<String> {
	let secret = random_secret();
	let record = Token { hash: digest(&secret), owner: user.id.clone(), scope, kind, session };
	app.db.put(&db::token(&record.hash), &record)?;
	Ok(secret)
}

pub(super) async fn login(
	State(app): State<App>,
	Json(request): Json<Login>,
) -> Result<impl axum::response::IntoResponse> {
	let users: Vec<User> = app.db.list("user/")?;
	let matched = users.into_iter().find(|u| u.name.eq_ignore_ascii_case(&request.name));
	let stored_hash = matched.as_ref().map(|u| u.password_hash.clone());
	if !verify_password(request.password, stored_hash).await? {
		return Err(Error(StatusCode::UNAUTHORIZED, anyhow::anyhow!("invalid credentials")));
	}
	let user = matched
		.ok_or_else(|| Error(StatusCode::UNAUTHORIZED, anyhow::anyhow!("invalid credentials")))?;
	let secret = issue(&app, &user, String::new(), "account".to_owned(), true)?;
	let secure = app.public_url.as_deref().is_some_and(|u| u.starts_with("https://"))
		|| app.behind_proxy;
	let cookie = if secure {
		format!("flora_session={secret}; HttpOnly; Secure; SameSite=Strict; Path=/")
	} else {
		format!("flora_session={secret}; HttpOnly; SameSite=Strict; Path=/")
	};
	Ok((
		[(axum::http::header::SET_COOKIE, cookie)],
		Json(LoginResponse { token: secret, user: user.into() }),
	))
}

#[derive(Clone)]
pub(super) struct Principal {
	pub user: User,
	pub token: Token,
}

impl Principal {
	fn in_scope(&self, path: &str) -> bool {
		match self.token.kind.as_str() {
			"account" => true,
			"file" => path == self.token.scope,
			"folder" => core::scoped(&self.token.scope, path),
			_ => false,
		}
	}

	pub(super) fn permits(&self, path: &str, upload: bool) -> bool {
		if upload { self.token.kind == "account" } else { self.in_scope(path) }
	}

	pub(super) fn manage(&self, _path: &str) -> bool {
		self.token.kind == "account"
	}

	pub(super) fn admin(&self) -> bool {
		self.user.admin && self.token.kind == "account"
	}
}

pub(super) fn authenticate(app: &App, headers: &HeaderMap) -> Result<Principal> {
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
	authenticate_token(app, secret)
}

pub(super) fn authenticate_token(app: &App, secret: &str) -> Result<Principal> {
	let token: Token = app
		.db
		.get(&db::token(&digest(secret)))?
		.ok_or_else(|| Error(StatusCode::UNAUTHORIZED, anyhow::anyhow!("invalid token")))?;
	let user = app.db.get(&db::user(&token.owner))?.ok_or_else(forbidden)?;
	Ok(Principal { user, token })
}

pub(super) async fn me(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
	let p = authenticate(&app, &headers)?;
	if !p.manage("") {
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
	kind: Option<String>,
}

pub(super) async fn create_token(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<NewToken>,
) -> Result<Json<serde_json::Value>> {
	let p = authenticate(&app, &headers)?;
	if !p.manage("") {
		return Err(forbidden());
	}
	let owner = super::files::owner(&app, &p, req.owner.as_deref())?;
	if owner != p.user.id && !p.admin() {
		return Err(forbidden());
	}
	let user: User = app.db.get(&db::user(&owner))?.ok_or_else(missing)?;
	let scope = core::clean_path(req.scope.as_deref().unwrap_or(""), true)
		.map_err(|e| bad(e.to_string()))?;
	let kind = match (scope.is_empty(), req.kind.as_deref()) {
		(true, None | Some("account")) => "account",
		(_, Some("folder")) => "folder",
		(false, Some("file")) => "file",
		(false, None) => {
			if app.db.get::<core::FileRecord>(&db::file(&owner, &scope))?.is_some() {
				"file"
			} else {
				"folder"
			}
		}
		_ => return Err(bad("scope kind must be account, file or folder and match the scope")),
	};
	if kind != "account" {
		let target = core::safe_join_async(&app.storage.join(&owner), &scope)
			.await
			.map_err(|e| bad(e.to_string()))?;
		let metadata = tokio::fs::metadata(target).await.map_err(|_| missing())?;
		if (kind == "file" && !metadata.is_file()) || (kind == "folder" && !metadata.is_dir()) {
			return Err(bad("scope does not match token kind"));
		}
	}
	let token = issue(&app, &user, scope, kind.to_owned(), false)?;
	let url = if kind == "account" {
		None
	} else {
		app.public_url.as_ref().map(|base| format!("{base}/t/{token}"))
	};
	Ok(Json(serde_json::json!({"token": token, "url": url})))
}

pub(super) async fn list_tokens(
	State(app): State<App>,
	headers: HeaderMap,
) -> Result<Json<Vec<Token>>> {
	let p = authenticate(&app, &headers)?;
	if !p.manage("") {
		return Err(forbidden());
	}
	let tokens: Vec<Token> = app.db.list("token/")?;
	Ok(Json(tokens.into_iter().filter(|t| t.owner == p.user.id || p.admin()).collect()))
}

pub(super) async fn revoke_token(
	State(app): State<App>,
	Path(hash): Path<String>,
	headers: HeaderMap,
) -> Result<StatusCode> {
	let p = authenticate(&app, &headers)?;
	if !p.manage("") {
		return Err(forbidden());
	}
	let token: Token = app.db.get(&db::token(&hash))?.ok_or_else(missing)?;
	if token.owner != p.user.id && !p.admin() {
		return Err(forbidden());
	}
	app.db.delete(&db::token(&hash))?;
	Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
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
	let p = authenticate(&app, &headers)?;
	if !p.admin() {
		return Err(forbidden());
	}
	let users: Vec<User> = app.db.list("user/")?;
	Ok(Json(users.into_iter().map(UserView::from).collect()))
}

pub(super) async fn create_user(
	State(app): State<App>,
	headers: HeaderMap,
	Json(req): Json<NewUser>,
) -> Result<Json<UserView>> {
	let p = authenticate(&app, &headers)?;
	if !p.admin() {
		return Err(forbidden());
	}
	let _guard = app.mutation.lock().await;
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
	let p = authenticate(&app, &headers)?;
	if !p.admin() && (p.user.id != id || req.admin.is_some() || req.quota.is_some()) {
		return Err(forbidden());
	}
	if !p.manage("") {
		return Err(forbidden());
	}
	let new_password_hash = match req.password {
		Some(password) => Some(password_hash(password).await?),
		None => None,
	};
	let _guard = app.mutation.lock().await;
	let mut user: User = app.db.get(&db::user(&id))?.ok_or_else(missing)?;
	if let Some(name) = req.name {
		if !name_valid(&name) {
			return Err(bad("invalid name"));
		}
		let users: Vec<User> = app.db.list("user/")?;
		if users.iter().any(|u| u.id != id && u.name.eq_ignore_ascii_case(&name)) {
			return Err(conflict("user name already exists"));
		}
		user.name = name;
	}
	if let Some(hashed) = new_password_hash {
		user.password_hash = hashed;
	}
	if let Some(admin) = req.admin {
		if user.admin && !admin {
			let users: Vec<User> = app.db.list("user/")?;
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
			core::duration(value).map_err(|e| bad(e.to_string()))?;
		}
		user.retention = retention;
	}
	let mut personal = profile::load(&app, &id).await?;
	personal.name = user.name.clone();
	personal.settings.retention = user.retention.clone();
	profile::save(&app, &id, &personal).await?;
	app.db.put(&db::user(&id), &user)?;
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
		let hash = "$argon2id$v=19$m=256,t=2,p=1$c29tZXNhbHQ$nf65EOgLrQMR/uIPnA4rEsF5h7TKyQwu9U1bMCHGi/4";
		let parsed = PasswordHash::new(hash).expect("parse existing hash");
		assert!(Argon2::default().verify_password(b"password", &parsed).is_ok());
		assert!(Argon2::default().verify_password(b"wrong password", &parsed).is_err());
	}
}
