use crate::api_types::{AuthResponse, SessionInfo};
use crate::{config::Config, state::AppState};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{ConnectInfo, Extension, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ipnet::IpNet;
use rand::{RngCore, rngs::OsRng};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{info, warn};
use uuid::Uuid;

const SESSION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const REFRESH_AFTER_MS: i64 = 60 * 60 * 1000;
const FAILURE_WINDOW_MS: i64 = 60 * 1000;
const MAX_IP_FAILURES: usize = 5;
const MAX_GLOBAL_FAILURES: usize = 30;
const MAX_BACKOFF_SECONDS: u64 = 15 * 60;

pub struct AuthContext {
    cookie_secure: bool,
    trusted_proxies: Vec<IpNet>,
    limiter: Mutex<LoginLimiter>,
}

impl AuthContext {
    pub fn new(config: &Config) -> Self {
        Self {
            cookie_secure: !config.insecure_cookies,
            trusted_proxies: config.trusted_proxies.clone(),
            limiter: Mutex::new(LoginLimiter::default()),
        }
    }
}

#[derive(Clone, Debug)]
struct AuthSession {
    id: String,
}

#[derive(Default)]
struct LoginLimiter {
    by_ip: HashMap<IpAddr, IpFailures>,
    global: VecDeque<i64>,
}

#[derive(Default)]
struct IpFailures {
    failures: VecDeque<i64>,
    blocked_until: i64,
    backoff_level: u32,
}

impl LoginLimiter {
    fn check(&mut self, ip: IpAddr, now: i64) -> Result<(), u64> {
        prune(&mut self.global, now);
        if self.global.len() >= MAX_GLOBAL_FAILURES {
            let retry_ms = self.global.front().copied().unwrap_or(now) + FAILURE_WINDOW_MS - now;
            return Err(retry_seconds(retry_ms));
        }

        let Some(bucket) = self.by_ip.get_mut(&ip) else {
            return Ok(());
        };
        prune(&mut bucket.failures, now);
        if bucket.blocked_until > now {
            return Err(retry_seconds(bucket.blocked_until - now));
        }
        if bucket.failures.len() >= MAX_IP_FAILURES {
            let window_retry =
                bucket.failures.front().copied().unwrap_or(now) + FAILURE_WINDOW_MS - now;
            let backoff = 60_u64.saturating_mul(2_u64.saturating_pow(bucket.backoff_level.min(4)));
            let delay_ms = (backoff.min(MAX_BACKOFF_SECONDS) as i64) * 1000;
            bucket.blocked_until = now + window_retry.max(delay_ms);
            bucket.backoff_level = bucket.backoff_level.saturating_add(1);
            return Err(retry_seconds(bucket.blocked_until - now));
        }
        Ok(())
    }

    fn failed(&mut self, ip: IpAddr, now: i64) -> usize {
        prune(&mut self.global, now);
        self.global.push_back(now);
        let bucket = self.by_ip.entry(ip).or_default();
        prune(&mut bucket.failures, now);
        bucket.failures.push_back(now);
        bucket.failures.len()
    }

    fn succeeded(&mut self, ip: IpAddr) {
        self.by_ip.remove(&ip);
    }
}

fn prune(entries: &mut VecDeque<i64>, now: i64) {
    while entries
        .front()
        .is_some_and(|time| now - *time >= FAILURE_WINDOW_MS)
    {
        entries.pop_front();
    }
}

fn retry_seconds(milliseconds: i64) -> u64 {
    ((milliseconds.max(1) as u64) + 999) / 1000
}

pub async fn initialize_password(
    pool: &SqlitePool,
    password: &SecretString,
) -> Result<(), Box<dyn Error>> {
    if password.expose_secret().is_empty() {
        return Err("SPRINTER_PASSWORD must not be empty".into());
    }

    let stored =
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'password_hash'")
            .fetch_optional(pool)
            .await?;
    if let Some(stored) = stored {
        let hash = decode_setting_string(&stored);
        if verify_password(password.expose_secret(), &hash) {
            return Ok(());
        }

        let new_hash = hash_password(password.expose_secret())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let now = now_ms();
        let mut tx = pool.begin().await?;
        let sessions_revoked = sqlx::query("DELETE FROM sessions")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let key_unreadable = sqlx::query(
            "UPDATE settings SET value = json_set(value, '$.readable', json('false')), updated_at = ? \
             WHERE key = 'openrouter_api_key' AND json_valid(value)",
        ).bind(now).execute(&mut *tx).await?.rows_affected() > 0;
        save_password_hash(&mut tx, &new_hash, now).await?;
        tx.commit().await?;
        warn!(
            sessions_revoked,
            api_key_unreadable = key_unreadable,
            "password rotated"
        );
    } else {
        let hash = hash_password(password.expose_secret())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let mut tx = pool.begin().await?;
        save_password_hash(&mut tx, &hash, now_ms()).await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn save_password_hash(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    hash: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    let value = serde_json::to_string(hash).expect("serializing a string cannot fail");
    sqlx::query(
        "INSERT INTO settings(key, value, updated_at) VALUES('password_hash', ?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(value)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn decode_setting_string(value: &str) -> String {
    serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_owned())
}

fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

pub fn router(state: AppState) -> Router<AppState> {
    let public = Router::new()
        .route("/api/auth/login", post(login))
        .route_layer(middleware::from_fn(require_json));
    let protected = Router::new()
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/sessions", get(list_sessions).delete(revoke_all))
        .route("/api/auth/sessions/{id}", delete(revoke_session))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_session,
        ));
    public.merge(protected)
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    password: String,
}

async fn login(State(state): State<AppState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let headers = parts.headers;
    let request_id = request_id(&headers);
    let input = match to_bytes(body, 1024 * 1024)
        .await
        .ok()
        .and_then(|body| serde_json::from_slice::<LoginRequest>(&body).ok())
    {
        Some(input) => input,
        None => {
            return api_error(
                &request_id,
                StatusCode::BAD_REQUEST,
                "invalid_json",
                "Request body must be valid JSON",
                None,
            );
        }
    };
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    let ip = resolve_client_ip(
        peer,
        headers.get("x-forwarded-for"),
        &state.auth.trusted_proxies,
    );
    let now = now_ms();

    let check = state
        .auth
        .limiter
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .check(ip, now);
    if let Err(retry_after) = check {
        warn!(ip = %ip, retry_after, "rate limited");
        return api_error(
            &request_id,
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many login attempts. Try again later.",
            Some(retry_after),
        );
    }

    let stored = match sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key = 'password_hash'",
    )
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(value)) => decode_setting_string(&value),
        Ok(None) => {
            return api_error(
                &request_id,
                StatusCode::SERVICE_UNAVAILABLE,
                "not_configured",
                "Server authentication is not configured",
                None,
            );
        }
        Err(error) => {
            tracing::error!(error = %error, "could not read password hash");
            return api_error(
                &request_id,
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "An internal error occurred",
                None,
            );
        }
    };

    if !verify_password(&input.password, &stored) {
        let attempts = state
            .auth
            .limiter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failed(ip, now);
        warn!(ip = %ip, attempts_in_window = attempts, "login failed");
        return api_error(
            &request_id,
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "The password is incorrect",
            None,
        );
    }

    state
        .auth
        .limiter
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .succeeded(ip);
    let mut token_bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let token = URL_SAFE_NO_PAD.encode(token_bytes);
    let token_hash = hash_token(&token);
    let id = Uuid::now_v7().to_string();
    let user_agent = header_text(&headers, header::USER_AGENT);
    let expires = now + SESSION_MS;
    let inserted = sqlx::query(
        "INSERT INTO sessions(id, token_hash, user_agent, ip, created_at, last_seen_at, expires_at) VALUES(?, ?, ?, ?, ?, ?, ?)",
    ).bind(&id).bind(token_hash).bind(&user_agent).bind(ip.to_string()).bind(now).bind(now).bind(expires)
        .execute(&state.pool).await;
    if let Err(error) = inserted {
        tracing::error!(error = %error, "could not create session");
        return api_error(
            &request_id,
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred",
            None,
        );
    }

    info!(session = %id, ip = %ip, ua = user_agent.as_deref().unwrap_or(""), "login ok");
    let mut response = Json(AuthResponse { ok: true }).into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, cookie_header(&state, &token, false));
    response
}

async fn require_json(request: Request, next: Next) -> Response {
    if request.method() != Method::POST
        && request.method() != Method::PATCH
        && request.method() != Method::PUT
        && request.method() != Method::DELETE
    {
        return next.run(request).await;
    }
    let valid = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
    if valid {
        return next.run(request).await;
    }
    api_error(
        &request_id(request.headers()),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "content_type_required",
        "Mutating requests must use application/json",
        None,
    )
}

pub async fn require_session(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.method() == Method::POST
        || request.method() == Method::PATCH
        || request.method() == Method::PUT
        || request.method() == Method::DELETE
    {
        let json = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
        let custom = request
            .headers()
            .get("x-sprinter")
            .and_then(|h| h.to_str().ok())
            == Some("1");
        if !json && !custom {
            return api_error(
                &request_id(request.headers()),
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "content_type_required",
                "Mutating requests must use application/json or X-Sprinter: 1",
                None,
            );
        }
    }
    let cookie_name = cookie_name(&state);
    let token = request
        .headers()
        .get(header::COOKIE)
        .and_then(|header| header.to_str().ok())
        .and_then(|value| cookie_value(value, cookie_name));
    let Some(token) = token else {
        return api_error(
            &request_id(request.headers()),
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Sign in to continue",
            None,
        );
    };
    let token_hash = hash_token(token);
    let row = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT id, expires_at, last_seen_at FROM sessions WHERE token_hash = ?",
    )
    .bind(token_hash)
    .fetch_optional(&state.pool)
    .await;
    let (session_id, expires, last_seen) = match row {
        Ok(Some(row)) if row.1 > now_ms() => row,
        Ok(Some((id, _, _))) => {
            let _ = sqlx::query("DELETE FROM sessions WHERE id = ?")
                .bind(id)
                .execute(&state.pool)
                .await;
            return api_error(
                &request_id(request.headers()),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Your session has expired",
                None,
            );
        }
        Ok(None) => {
            return api_error(
                &request_id(request.headers()),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Sign in to continue",
                None,
            );
        }
        Err(error) => {
            tracing::error!(error = %error, "could not validate session");
            return api_error(
                &request_id(request.headers()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "An internal error occurred",
                None,
            );
        }
    };
    let now = now_ms();
    if now - last_seen >= REFRESH_AFTER_MS {
        if let Err(error) =
            sqlx::query("UPDATE sessions SET last_seen_at = ?, expires_at = ? WHERE id = ?")
                .bind(now)
                .bind(now + SESSION_MS)
                .bind(&session_id)
                .execute(&state.pool)
                .await
        {
            tracing::error!(error = %error, session = %session_id, "could not refresh session");
            return api_error(
                &request_id(request.headers()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "An internal error occurred",
                None,
            );
        }
    } else if expires <= now {
        return api_error(
            &request_id(request.headers()),
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Your session has expired",
            None,
        );
    }
    request
        .extensions_mut()
        .insert(AuthSession { id: session_id });
    next.run(request).await
}

async fn logout(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(&session.id)
        .execute(&state.pool)
        .await
    {
        tracing::error!(error = %error, session = %session.id, "could not end session");
        return api_error(
            &request_id(&headers),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred",
            None,
        );
    }
    info!(session = %session.id, "logout");
    let mut response = Json(AuthResponse { ok: true }).into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, cookie_header(&state, "", true));
    response
}

async fn list_sessions(
    State(state): State<AppState>,
    Extension(current): Extension<AuthSession>,
    headers: HeaderMap,
) -> Response {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, i64, i64, i64)>(
        "SELECT id, user_agent, ip, created_at, last_seen_at, expires_at FROM sessions ORDER BY last_seen_at DESC",
    ).fetch_all(&state.pool).await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(error = %error, "could not list sessions");
            return api_error(
                &request_id(&headers),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "An internal error occurred",
                None,
            );
        }
    };
    let sessions: Vec<SessionInfo> = rows
        .into_iter()
        .map(
            |(id, user_agent, ip, created_at, last_seen_at, expires_at)| SessionInfo {
                current: id == current.id,
                id,
                user_agent,
                ip,
                created_at,
                last_seen_at,
                expires_at,
            },
        )
        .collect();
    Json(sessions).into_response()
}

async fn revoke_session(
    State(state): State<AppState>,
    Extension(current): Extension<AuthSession>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let result = sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(error = %error, "could not revoke session");
            return api_error(
                &request_id(&headers),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "An internal error occurred",
                None,
            );
        }
    };
    if result.rows_affected() == 0 {
        return api_error(
            &request_id(&headers),
            StatusCode::NOT_FOUND,
            "not_found",
            "Session not found",
            None,
        );
    }
    info!(session = %id, "session revoked");
    let mut response = Json(AuthResponse { ok: true }).into_response();
    if id == current.id {
        response
            .headers_mut()
            .append(header::SET_COOKIE, cookie_header(&state, "", true));
    }
    response
}

async fn revoke_all(
    State(state): State<AppState>,
    Extension(current): Extension<AuthSession>,
    headers: HeaderMap,
) -> Response {
    match sqlx::query("DELETE FROM sessions")
        .execute(&state.pool)
        .await
    {
        Ok(result) => {
            info!(session = %current.id, revoked = result.rows_affected(), "all sessions revoked")
        }
        Err(error) => {
            tracing::error!(error = %error, "could not revoke sessions");
            return api_error(
                &request_id(&headers),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "An internal error occurred",
                None,
            );
        }
    }
    let mut response = Json(AuthResponse { ok: true }).into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, cookie_header(&state, "", true));
    response
}

fn cookie_name(state: &AppState) -> &'static str {
    if state.auth.cookie_secure {
        "__Host-sprinter"
    } else {
        "sprinter"
    }
}

fn cookie_header(state: &AppState, token: &str, clear: bool) -> HeaderValue {
    let name = cookie_name(state);
    let mut value = format!("{name}={token}; Path=/; HttpOnly; SameSite=Strict");
    if state.auth.cookie_secure {
        value.push_str("; Secure");
    }
    if clear {
        value.push_str("; Max-Age=0");
    } else {
        value.push_str("; Max-Age=2592000");
    }
    HeaderValue::from_str(&value).expect("cookie contains only safe characters")
}

fn cookie_value<'a>(cookie: &'a str, name: &str) -> Option<&'a str> {
    cookie
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
        .filter(|value| !value.is_empty())
}

fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn resolve_client_ip(
    peer: IpAddr,
    forwarded_for: Option<&axum::http::HeaderValue>,
    trusted: &[IpNet],
) -> IpAddr {
    if !trusted.iter().any(|network| network.contains(&peer)) {
        return peer;
    }
    let Some(value) = forwarded_for.and_then(|header| header.to_str().ok()) else {
        return peer;
    };
    let mut chain = value
        .split(',')
        .filter_map(|value| value.trim().parse::<IpAddr>().ok())
        .collect::<Vec<_>>();
    chain.push(peer);
    for address in chain.into_iter().rev() {
        if trusted.iter().any(|network| network.contains(&address)) {
            continue;
        }
        return address;
    }
    peer
}

pub fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned()
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub fn api_error(
    request_id: &str,
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    retry_after: Option<u64>,
) -> Response {
    #[derive(Serialize)]
    struct ErrorEnvelope<'a> {
        error: ErrorDetail<'a>,
    }
    #[derive(Serialize)]
    struct ErrorDetail<'a> {
        code: &'a str,
        message: &'a str,
        request_id: &'a str,
    }
    let body = Json(ErrorEnvelope {
        error: ErrorDetail {
            code,
            message,
            request_id,
        },
    });
    let mut response = (status, body).into_response();
    if let Some(seconds) = retry_after {
        if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
    }
    response
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
