use crate::{
    auth, chats, client_log, exports, generation, messages, search, settings, state::AppState,
    uploads, usage,
};
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderName, HeaderValue, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use std::time::Instant;
use tower_http::{compression::CompressionLayer, limit::RequestBodyLimitLayer};
use ulid::Ulid;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist/"]
struct Assets;

pub fn router(state: AppState) -> Router {
    // JSON APIs keep a small request cap. Raw uploads have their own streamed,
    // per-kind limits and therefore must sit outside this layer.
    let limited_api = Router::new()
        .merge(auth::router(state.clone()))
        .merge(settings::router(state.clone()))
        .merge(chats::router(state.clone()))
        .merge(exports::router(state.clone()))
        .merge(messages::router(state.clone()))
        .merge(generation::router(state.clone()))
        .merge(search::router(state.clone()))
        .merge(client_log::router(state.clone()))
        .merge(usage::router(state.clone()))
        .layer(RequestBodyLimitLayer::new(1024 * 1024));
    Router::new()
        .route("/healthz", get(healthz))
        .merge(limited_api)
        .merge(uploads::router(state.clone()))
        .fallback(spa_fallback)
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn(request_id))
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> Response {
    match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"status":"ok"}"#,
        )
            .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"status":"unavailable"}"#,
        )
            .into_response(),
    }
}

async fn request_id(mut request: Request, next: Next) -> Response {
    let request_id = Ulid::new().to_string();
    let started = Instant::now();
    let method = request.method().clone();
    let route = route_template(request.uri().path());
    // Read the declared length only. In particular, do not consume or buffer uploads
    // or request streams just to measure them.
    let req_bytes = content_length(request.headers());
    let is_upload = route == "/api/uploads" || route == "/api/uploads/{id}";
    let is_sse_route = route == "/api/messages/{id}/stream";
    request.headers_mut().insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_str(&request_id).expect("ULID is a valid header value"),
    );
    let span = tracing::info_span!("request", rid = %request_id, method = %method, route, req_bytes = req_bytes);
    let mut response = next.run(request).instrument(span).await;
    let latency_ms = started.elapsed().as_millis();
    let resp_bytes = content_length(response.headers());
    let is_sse = is_sse_route
        || response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
    tracing::info!(
        method = %method,
        route,
        status = response.status().as_u16(),
        latency_ms,
        req_bytes = req_bytes,
        resp_bytes = resp_bytes,
        "completed"
    );
    if latency_ms > 2_000 && !is_sse && !is_upload {
        tracing::warn!(
            method = %method,
            route,
            status = response.status().as_u16(),
            latency_ms,
            req_bytes = req_bytes,
            resp_bytes = resp_bytes,
            "slow request"
        );
    }
    response.headers_mut().insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_str(&request_id).expect("validated request id"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; worker-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}

fn content_length(headers: &http::HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

/// Return a stable route label without putting user controlled IDs or arbitrary
/// fallback paths into the log output.
fn route_template(path: &str) -> &'static str {
    match path {
        "/healthz" => "/healthz",
        "/api/auth/login" => "/api/auth/login",
        "/api/auth/logout" => "/api/auth/logout",
        "/api/auth/sessions" => "/api/auth/sessions",
        "/api/settings" => "/api/settings",
        "/api/models" => "/api/models",
        "/api/chats" => "/api/chats",
        "/api/chats/new/messages" => "/api/chats/new/messages",
        "/api/uploads" => "/api/uploads",
        "/api/search" => "/api/search",
        "/api/client-log" => "/api/client-log",
        "/api/usage" => "/api/usage",
        _ => {
            let segments: Vec<_> = path.trim_matches('/').split('/').collect();
            match segments.as_slice() {
                ["api", "auth", "sessions", id] if !id.is_empty() => "/api/auth/sessions/{id}",
                ["api", "chats", id] if !id.is_empty() => "/api/chats/{id}",
                ["api", "chats", id, "switch"] if !id.is_empty() => "/api/chats/{id}/switch",
                ["api", "chats", id, "messages"] if !id.is_empty() => "/api/chats/{id}/messages",
                ["api", "chats", id, "export"] if !id.is_empty() => "/api/chats/{id}/export",
                ["api", "messages", id, "stream"] if !id.is_empty() => "/api/messages/{id}/stream",
                ["api", "messages", id, "cancel"] if !id.is_empty() => "/api/messages/{id}/cancel",
                ["api", "messages", id, "regenerate"] if !id.is_empty() => {
                    "/api/messages/{id}/regenerate"
                }
                ["api", "uploads", id] if !id.is_empty() => "/api/uploads/{id}",
                ["api", ..] => "/api/{path}",
                _ => "/{asset}",
            }
        }
    }
}

use tracing::Instrument;

async fn spa_fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let requested_asset = if path.is_empty() {
        None
    } else {
        Assets::get(path)
    };
    let is_index = requested_asset.is_none();
    let asset = requested_asset.or_else(|| Assets::get("index.html"));
    let Some(asset) = asset else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Frontend bundle is not built yet",
        )
            .into_response();
    };
    let content_type = mime_type(if is_index { "index.html" } else { path });
    let mut response = Response::new(Body::from(asset.data.into_owned()));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if path.is_empty() || path == "index.html" {
            "no-cache"
        } else {
            "public, max-age=31536000, immutable"
        }),
    );
    response
}

fn mime_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "json" | "webmanifest" => "application/json",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}
