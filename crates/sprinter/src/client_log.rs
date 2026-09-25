use crate::{auth, state::AppState};
use axum::{
    Router,
    body::Bytes,
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(60);
const MAX_PER_WINDOW: usize = 20;
const MAX_LOG_CHARS: usize = 500;

static RECENT_LOGS: OnceLock<Mutex<VecDeque<Instant>>> = OnceLock::new();

#[derive(Deserialize)]
struct ClientLogRequest {
    level: String,
    message: String,
    stack: Option<String>,
    route: String,
    app_version: String,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/client-log", post(log_handler))
        .route_layer(middleware::from_fn_with_state(state, auth::require_session))
}

async fn log_handler(headers: HeaderMap, body: Bytes) -> Result<StatusCode, ClientLogError> {
    if body.len() > 8 * 1024 {
        return Err(ClientLogError::too_large(&headers));
    }
    let request: ClientLogRequest =
        serde_json::from_slice(&body).map_err(|_| ClientLogError::bad_request(&headers))?;
    if !take_token() {
        return Err(ClientLogError::rate_limited(&headers));
    }
    if !matches!(request.level.as_str(), "error" | "warn" | "info")
        || request.message.trim().is_empty()
        || request.route.len() > 512
        || request.app_version.len() > 100
    {
        return Err(ClientLogError::bad_request(&headers));
    }
    let message = preview(&request.message, MAX_LOG_CHARS);
    let stack = request
        .stack
        .as_deref()
        .map(|stack| preview(stack, MAX_LOG_CHARS));
    tracing::warn!(
        level = %request.level,
        message = %message,
        stack = ?stack,
        route = %preview(&request.route, 200),
        app_version = %request.app_version,
        "client error"
    );
    Ok(StatusCode::NO_CONTENT)
}

fn take_token() -> bool {
    let recent = RECENT_LOGS.get_or_init(|| Mutex::new(VecDeque::new()));
    let now = Instant::now();
    let mut recent = recent.lock().expect("client log rate limiter poisoned");
    while recent
        .front()
        .is_some_and(|time| now.duration_since(*time) >= WINDOW)
    {
        recent.pop_front();
    }
    if recent.len() >= MAX_PER_WINDOW {
        return false;
    }
    recent.push_back(now);
    true
}

fn preview(value: &str, max_chars: usize) -> String {
    let value = value.replace(['\r', '\n'], " ");
    let mut chars = value.chars();
    let output = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{output}…")
    } else {
        output
    }
}

struct ClientLogError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: String,
}

impl ClientLogError {
    fn new(
        headers: &HeaderMap,
        status: StatusCode,
        code: &'static str,
        message: &'static str,
    ) -> Self {
        Self {
            status,
            code,
            message,
            request_id: auth::request_id(headers),
        }
    }
    fn bad_request(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::BAD_REQUEST,
            "invalid_client_log",
            "Client log fields are invalid",
        )
    }
    fn rate_limited(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many client logs",
        )
    }
    fn too_large(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::PAYLOAD_TOO_LARGE,
            "client_log_too_large",
            "Client log body exceeds 8 KB",
        )
    }
}

impl IntoResponse for ClientLogError {
    fn into_response(self) -> Response {
        let mut response =
            auth::api_error(&self.request_id, self.status, self.code, self.message, None);
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_LOG_CHARS, preview};

    #[test]
    fn client_log_previews_are_single_line_and_bounded() {
        let value = format!("{}\nrest", "a".repeat(MAX_LOG_CHARS + 10));
        let output = preview(&value, MAX_LOG_CHARS);
        assert_eq!(output.chars().count(), MAX_LOG_CHARS + 1);
        assert!(!output.contains('\n'));
    }
}
