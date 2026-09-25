use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoggedRequest {
    pub method: String,
    pub path: String,
    pub headers: std::collections::BTreeMap<String, String>,
    pub body: Value,
}

#[derive(Clone, Default)]
pub struct FakeOpenRouter {
    requests: Arc<Mutex<Vec<LoggedRequest>>>,
}

impl FakeOpenRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/v1/models", get(models))
            .route("/api/v1/credits", get(credits))
            .route("/api/v1/key", get(key_info))
            .route("/api/v1/chat/completions", post(chat_completions))
            .route("/__requests", get(requests))
            .route("/__reset", post(reset))
            .with_state(self.clone())
    }

    pub fn requests_snapshot(&self) -> Vec<LoggedRequest> {
        self.requests.lock().expect("request log poisoned").clone()
    }

    pub fn reset_requests(&self) {
        self.requests.lock().expect("request log poisoned").clear();
    }

    fn record(&self, method: &str, path: &str, headers: &HeaderMap, body: Value) {
        let captured_headers = headers
            .iter()
            .filter_map(|(name, value)| {
                if name == axum::http::header::AUTHORIZATION {
                    return None;
                }
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_owned()))
            })
            .collect();
        self.requests
            .lock()
            .expect("request log poisoned")
            .push(LoggedRequest {
                method: method.into(),
                path: path.into(),
                headers: captured_headers,
                body,
            });
    }
}

pub fn app() -> Router {
    FakeOpenRouter::new().router()
}

pub async fn serve(addr: SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "fake OpenRouter listening");
    axum::serve(listener, app()).await
}

async fn models(State(state): State<FakeOpenRouter>, headers: HeaderMap) -> Json<Value> {
    state.record("GET", "/api/v1/models", &headers, Value::Null);
    Json(json!({"data": [
        model("test/text", "Fake Text", 32768, "0.000001", "0.000002", false),
        model("test/vision", "Fake Vision", 65536, "0.000003", "0.000006", true),
        model("test/title", "Fake Title", 8192, "0.0000001", "0.0000002", false)
    ]}))
}

fn model(
    id: &str,
    name: &str,
    context_length: u64,
    prompt: &str,
    completion: &str,
    vision: bool,
) -> Value {
    let mut input_modalities = vec!["text"];
    if vision {
        input_modalities.push("image");
    }
    json!({
        "id": id,
        "name": name,
        "context_length": context_length,
        "pricing": {"prompt": prompt, "completion": completion, "request": "0", "image": "0"},
        "architecture": {"modality": if vision {"text+image->text"} else {"text->text"}, "input_modalities": input_modalities, "output_modalities": ["text"], "tokenizer": "fake", "instruct_type": null},
        "top_provider": {"context_length": context_length, "max_completion_tokens": 4096, "is_moderated": false},
        "per_request_limits": null
    })
}

async fn credits(State(state): State<FakeOpenRouter>, headers: HeaderMap) -> Response {
    state.record("GET", "/api/v1/credits", &headers, Value::Null);
    if bearer(&headers).as_deref() == Some("bad-key") {
        return provider_error(StatusCode::UNAUTHORIZED, "Invalid API key", 401);
    }
    Json(json!({"data": {"total_credits": 42.0, "total_usage": 0.1234}})).into_response()
}

async fn key_info(State(state): State<FakeOpenRouter>, headers: HeaderMap) -> Response {
    state.record("GET", "/api/v1/key", &headers, Value::Null);
    if bearer(&headers).as_deref() == Some("bad-key") {
        return provider_error(StatusCode::UNAUTHORIZED, "Invalid API key", 401);
    }
    Json(json!({"data": {
        "label": "Fake OpenRouter key",
        "limit": null,
        "limit_remaining": null,
        "limit_reset": null,
        "usage": 0.1234,
        "is_free_tier": false
    }}))
    .into_response()
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_owned)
}

fn provider_error(status: StatusCode, message: &str, code: u16) -> Response {
    (
        status,
        Json(json!({"error": {"message": message, "code": code}})),
    )
        .into_response()
}

async fn requests(State(state): State<FakeOpenRouter>) -> Json<Vec<LoggedRequest>> {
    Json(state.requests_snapshot())
}

async fn reset(State(state): State<FakeOpenRouter>) -> StatusCode {
    state.reset_requests();
    StatusCode::NO_CONTENT
}

async fn chat_completions(
    State(state): State<FakeOpenRouter>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.record("POST", "/api/v1/chat/completions", &headers, body.clone());

    let user_text = last_user_text(&body);
    if body.get("stream").and_then(Value::as_bool) != Some(true) {
        return Json(title_completion(&body, &user_text)).into_response();
    }

    let scenario = Scenario::from_text(&user_text);
    let chunks = scenario_chunks(&body, &user_text, scenario);
    let output = stream::iter(chunks)
        .then(|(delay, event)| async move {
            if !delay.is_zero() {
                sleep(delay).await;
            }
            event
        })
        .map(Ok::<_, Infallible>);
    Sse::new(output).into_response()
}

#[derive(Clone, Copy)]
enum Scenario {
    Echo,
    Slow,
    Error,
    Rich,
    Think,
}
impl Scenario {
    fn from_text(text: &str) -> Self {
        if text.contains("[[slow]]") {
            Self::Slow
        } else if text.contains("[[error]]") {
            Self::Error
        } else if text.contains("[[rich]]") {
            Self::Rich
        } else if text.contains("[[think]]") {
            Self::Think
        } else {
            Self::Echo
        }
    }
}

fn scenario_chunks(body: &Value, user_text: &str, scenario: Scenario) -> Vec<(Duration, Event)> {
    if matches!(scenario, Scenario::Error) {
        return vec![
            (
                Duration::from_millis(60),
                sse_data(
                    json!({"id":"fake-completion","object":"chat.completion.chunk","created":now_secs(),"model":model_name(body),"choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}),
                ),
            ),
            (
                Duration::from_millis(60),
                sse_data(
                    json!({"error":{"message":"Rate limit exceeded","code":429,"metadata":{"provider_name":"Fake"}}}),
                ),
            ),
        ];
    }

    let reply = match scenario {
        Scenario::Rich => "| Name | Value |\n| --- | ---: |\n| Sprinter | 1 |\n\n```rust\nfn main() { println!(\"hello\"); }\n```\n\n$E = mc^2$\n\n```mermaid\ngraph TD; A-->B;\n```".to_owned(),
        _ => format!("You said: {}", user_text.replace("[[slow]]", "").replace("[[think]]", "").replace("[[rich]]", "").trim()),
    };
    let parts = if matches!(scenario, Scenario::Slow) {
        split_reply_exact(&reply, 20)
    } else {
        split_reply(&reply, 4)
    };
    let interval = if matches!(scenario, Scenario::Slow) {
        Duration::from_millis(500)
    } else {
        Duration::from_millis(75)
    };
    let initial = if matches!(scenario, Scenario::Think) {
        Duration::from_secs(2)
    } else {
        Duration::ZERO
    };
    let mut out = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let delay = if index == 0 { initial } else { interval };
        out.push((delay, sse_data(json!({
            "id":"fake-completion", "object":"chat.completion.chunk", "created":now_secs(), "model":model_name(body),
            "choices":[{"index":0,"delta":{"content":part},"finish_reason":null}]
        }))));
    }
    out.push((interval, sse_data(json!({
        "id":"fake-completion", "object":"chat.completion.chunk", "created":now_secs(), "model":model_name(body),
        "choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":12,"completion_tokens":parts.len() as u64,"total_tokens":12 + parts.len() as u64,"cost":0.0003}
    }))));
    out.push((Duration::ZERO, Event::default().data("[DONE]")));
    out
}

fn split_reply(reply: &str, max_chunks: usize) -> Vec<String> {
    let chars: Vec<char> = reply.chars().collect();
    let chunk_size = chars.len().div_ceil(max_chunks).max(1);
    chars
        .chunks(chunk_size)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn split_reply_exact(reply: &str, count: usize) -> Vec<String> {
    let chars: Vec<char> = reply.chars().collect();
    (0..count)
        .map(|index| {
            let start = index * chars.len() / count;
            let end = (index + 1) * chars.len() / count;
            chars[start..end].iter().collect()
        })
        .collect()
}

fn sse_data(value: Value) -> Event {
    Event::default().data(value.to_string())
}
fn model_name(body: &Value) -> &str {
    body.get("model")
        .and_then(Value::as_str)
        .unwrap_or("test/text")
}
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn last_user_text(body: &Value) -> String {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return String::new();
    };
    let Some(message) = messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return String::new();
    };
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn title_completion(body: &Value, text: &str) -> Value {
    let cleaned = text
        .replace("[[slow]]", "")
        .replace("[[error]]", "")
        .replace("[[rich]]", "")
        .replace("[[think]]", "")
        .trim()
        .to_owned();
    let title = cleaned.chars().take(60).collect::<String>();
    json!({
        "id":"fake-title", "object":"chat.completion", "created":now_secs(), "model":model_name(body),
        "choices":[{"index":0,"message":{"role":"assistant","content":title},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":8,"completion_tokens":title.len() as u64,"total_tokens":8 + title.len() as u64,"cost":0.00001}
    })
}

// A tiny conversion helper lets the service be embedded in tests without binding a port.
pub async fn response_body(response: Response) -> Vec<u8> {
    use axum::body::to_bytes;
    to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_or_default()
        .to_vec()
}
