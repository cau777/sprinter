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
            .route("/api/v1/models/user", get(user_models))
            .route("/api/v1/endpoints/zdr", get(zdr_endpoints))
            .route("/api/v1/credits", get(credits))
            .route("/api/v1/key", get(key_info))
            .route("/api/v1/messages", post(messages))
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
        with_tools(model("test/text", "Fake Text", 32768, "0.000001", "0.000002", false, false)),
        model("test/vision", "Fake Vision", 65536, "0.000003", "0.000006", true, false),
        with_tools(model("test/file", "Fake PDF", 32768, "0.000001", "0.000002", false, true)),
        with_tools(model("test/partial", "Fake Partial Search", 32768, "0.000001", "0.000002", false, false)),
        model("test/title", "Fake Title", 8192, "0.0000001", "0.0000002", false, false)
    ]}))
}

async fn user_models(State(state): State<FakeOpenRouter>, headers: HeaderMap) -> Json<Value> {
    state.record("GET", "/api/v1/models/user", &headers, Value::Null);
    // Simulates an account policy that excludes test/title while retaining a
    // non-ZDR-capable model (test/vision) for Sprinter to remove.
    Json(json!({"data": [
        with_tools(model("test/text", "Fake Text", 32768, "0.000001", "0.000002", false, false)),
        model("test/vision", "Fake Vision", 65536, "0.000003", "0.000006", true, false),
        with_tools(model("test/file", "Fake PDF", 32768, "0.000001", "0.000002", false, true)),
        with_tools(model("test/partial", "Fake Partial Search", 32768, "0.000001", "0.000002", false, false))
    ]}))
}

async fn zdr_endpoints(State(state): State<FakeOpenRouter>, headers: HeaderMap) -> Json<Value> {
    state.record("GET", "/api/v1/endpoints/zdr", &headers, Value::Null);
    Json(json!({"data": [
        {"model_id":"test/text","provider_name":"Azure","tag":"azure/global","native_tools":{"openrouter:web_search":{"type":"web_search"},"openrouter:apply_patch":{"type":"apply_patch"}}},
        {"model_id":"test/partial","provider_name":"Search A","tag":"search-a","native_tools":{"openrouter:web_search":{"type":"google_search"}}},
        {"model_id":"test/partial","provider_name":"Search B","tag":"search-b","native_tools":{}},
        {"model_id":"test/file","provider_name":"Azure","tag":"azure/global","native_tools":{}}
    ]}))
}

fn with_tools(mut model: Value) -> Value {
    model["supported_parameters"] = json!(["tools"]);
    model
}

fn model(
    id: &str,
    name: &str,
    context_length: u64,
    prompt: &str,
    completion: &str,
    vision: bool,
    files: bool,
) -> Value {
    let mut input_modalities = vec!["text"];
    if vision {
        input_modalities.push("image");
    }
    if files {
        input_modalities.push("file");
    }
    json!({
        "id": id,
        "name": name,
        "created": now_secs(),
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

async fn messages(
    State(state): State<FakeOpenRouter>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.record("POST", "/api/v1/messages", &headers, body.clone());

    if let Some(response) = http_error(&body) {
        return response;
    }

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

fn http_error(body: &Value) -> Option<Response> {
    let status = body
        .get("model")
        .and_then(Value::as_str)?
        .strip_prefix("test/http-error-")?
        .parse::<u16>()
        .ok()?;
    let (kind, error_type, message) = match status {
        400 => (
            "invalid_request_error",
            "invalid_request",
            "This endpoint's maximum context length is 200000 tokens.",
        ),
        401 => ("authentication_error", "authentication", "Bad API key."),
        402 => ("payment_error", "payment_required", "No credits remain."),
        429 => (
            "rate_limit_error",
            "rate_limit_exceeded",
            "Rate limit exceeded.",
        ),
        503 => ("api_error", "server_error", "Provider unavailable."),
        _ => ("api_error", "unknown", "Provider rejected the request."),
    };
    Some(
        (
            StatusCode::from_u16(status).ok()?,
            Json(json!({"type":"error","error":{"type":kind,"error_type":error_type,"message":message},"request_id":"gen-fake"})),
        )
            .into_response(),
    )
}

#[derive(Clone, Copy)]
enum Scenario {
    Echo,
    Slow,
    Error,
    Rich,
    Think,
    Truncated,
    Search,
    ToolFallback,
    Bash,
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
        } else if text.contains("[[truncated]]") {
            Self::Truncated
        } else if text.contains("[[tool-fallback]]") {
            Self::ToolFallback
        } else if text.contains("[[tool-bash]]") {
            Self::Bash
        } else if text.contains("[[tool-search]]") {
            Self::Search
        } else {
            Self::Echo
        }
    }
}

fn scenario_chunks(body: &Value, user_text: &str, scenario: Scenario) -> Vec<(Duration, Event)> {
    if matches!(scenario, Scenario::Error) {
        return vec![
            (
                Duration::ZERO,
                named_data(
                    "message_start",
                    json!({"type":"message_start","message":{"id":"gen-fake","model":model_name(body),"provider":"Azure","usage":{"input_tokens":12,"output_tokens":0}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_start",
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                ),
            ),
            (
                Duration::from_millis(60),
                named_data(
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Partial reply"}}),
                ),
            ),
            (
                Duration::from_millis(60),
                named_data(
                    "error",
                    json!({"type":"error","error":{"type":"rate_limit_error","message":"Rate limit exceeded","error_type":"rate_limit_exceeded"},"request_id":"gen-fake"}),
                ),
            ),
        ];
    }

    if matches!(scenario, Scenario::Truncated) {
        return vec![
            (
                Duration::ZERO,
                named_data(
                    "message_start",
                    json!({"type":"message_start","message":{"id":"gen-fake","model":model_name(body),"provider":"Azure","usage":{"input_tokens":12,"output_tokens":0}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_start",
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Cut off"}}),
                ),
            ),
        ];
    }

    if matches!(scenario, Scenario::Search | Scenario::ToolFallback) {
        let fallback = matches!(scenario, Scenario::ToolFallback);
        let mut chunks = vec![
            (
                Duration::ZERO,
                named_data(
                    "message_start",
                    json!({"type":"message_start","message":{"id":"gen-fake","model":model_name(body),"provider":"Azure","usage":{"input_tokens":12,"output_tokens":0}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_start",
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"ws_fake_1","name":"openrouter:web_search","input":{}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":0}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_start",
                    json!({"type":"content_block_start","index":1,"content_block":{"type":"server_tool_use","id":"ws_fake_2","name":"openrouter:web_search","input":{}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":1}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_start",
                    json!({"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"Search result summary."}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":2,"delta":{"type":"citations_delta","citation":{"type":"web_search_result_location","url":"https://example.com/source","title":"Example source","cited_text":"","encrypted_index":""}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":2}),
                ),
            ),
        ];
        chunks.push((Duration::ZERO, named_data("message_delta", json!({
            "type":"message_delta","delta":{"stop_reason":"end_turn"},
            "usage":{"input_tokens":12,"output_tokens":4,"server_tool_use":{"web_search_requests":2,"web_fetch_requests":0},"tool_calls_requested":if fallback {1} else {0},"tool_calls_executed":if fallback {1} else {0},"cost":if fallback {0.012} else {0.005},"cost_details":{"upstream_inference_cost":0.005,"server_tool_cost":if fallback {0.007} else {0.0}}}
        }))));
        chunks.push((
            Duration::ZERO,
            named_data("message_stop", json!({"type":"message_stop"})),
        ));
        chunks.push((
            Duration::ZERO,
            Event::default().event("data").data("[DONE]"),
        ));
        return chunks;
    }

    if matches!(scenario, Scenario::Bash) {
        return vec![
            (
                Duration::ZERO,
                named_data(
                    "message_start",
                    json!({"type":"message_start","message":{"id":"gen-fake","model":model_name(body),"provider":"Azure","usage":{"input_tokens":12,"output_tokens":0}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_start",
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"toolu_fake","name":"openrouter:bash","input":{}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"printf tool-output\"}"}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":0}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_start",
                    json!({"type":"content_block_start","index":1,"content_block":{"type":"openrouter_bash_tool_result","tool_use_id":"toolu_fake","content":{"command":"printf tool-output","stdout":"tool-output","stderr":"","exitCode":0,"container_id":"container-fake"}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_start",
                    json!({"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"Command completed."}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":2}),
                ),
            ),
            (
                Duration::ZERO,
                named_data(
                    "message_delta",
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":12,"output_tokens":4,"cost":0.0034,"cost_details":{"upstream_inference_cost":0.0004,"server_tool_cost":0.003}}}),
                ),
            ),
            (
                Duration::ZERO,
                named_data("message_stop", json!({"type":"message_stop"})),
            ),
            (
                Duration::ZERO,
                Event::default().event("data").data("[DONE]"),
            ),
        ];
    }

    let reply = match scenario {
        Scenario::Rich => "| Name | Value |\n| --- | ---: |\n| Sprinter | 1 |\n\n```rust\nfn main() { println!(\"hello\"); }\n```\n\n$E = mc^2$\n\n```mermaid\ngraph TD; A-->B;\n```\n\n[Sprinter](https://example.com)".to_owned(),
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
    let mut out = vec![(
        Duration::ZERO,
        named_data(
            "message_start",
            json!({
                "type":"message_start",
                "message":{"id":"gen-fake","model":model_name(body),"provider":"Azure","content":[],"usage":{"input_tokens":12,"cache_read_input_tokens":5,"cache_creation_input_tokens":2,"output_tokens":0}}
            }),
        ),
    )];
    out.push((Duration::ZERO, named_data("ping", json!({"type":"ping"}))));
    if matches!(scenario, Scenario::Think) {
        out.extend([
            (Duration::ZERO, named_data("content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"internal thought"}}))),
            (Duration::ZERO, named_data("content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"internal thought"}}))),
            (Duration::ZERO, named_data("content_block_stop", json!({"type":"content_block_stop","index":0}))),
            (Duration::ZERO, named_data("content_block_start", json!({"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"opaque"}}))),
            (Duration::ZERO, named_data("content_block_stop", json!({"type":"content_block_stop","index":1}))),
        ]);
    }
    let unknown_index = if matches!(scenario, Scenario::Think) {
        2
    } else {
        0
    };
    out.extend([
        (Duration::ZERO, named_data("content_block_start", json!({"type":"content_block_start","index":unknown_index,"content_block":{"type":"future_unknown","data":{}}}))),
        (Duration::ZERO, named_data("content_block_delta", json!({"type":"content_block_delta","index":unknown_index,"delta":{"type":"text_delta","text":"ignored unknown block"}}))),
        (Duration::ZERO, named_data("content_block_stop", json!({"type":"content_block_stop","index":unknown_index}))),
    ]);
    let text_index = unknown_index + 1;
    out.push((Duration::ZERO, named_data("content_block_start", json!({
        "type":"content_block_start","index":text_index,"content_block":{"type":"text","text":""}
    }))));
    for (index, part) in parts.iter().enumerate() {
        let delay = if index == 0 { initial } else { interval };
        out.push((delay, named_data("content_block_delta", json!({
            "type":"content_block_delta","index":text_index,"delta":{"type":"text_delta","text":part}
        }))));
    }
    out.push((
        interval,
        named_data(
            "content_block_stop",
            json!({"type":"content_block_stop","index":text_index}),
        ),
    ));
    out.push((interval, named_data("message_delta", json!({
        "type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},
        "usage":{"input_tokens":12,"cache_read_input_tokens":5,"cache_creation_input_tokens":2,"output_tokens":parts.len(),"output_tokens_details":{"thinking_tokens":2},"cost":0.0003}
    }))));
    out.push((
        Duration::ZERO,
        named_data("message_stop", json!({"type":"message_stop"})),
    ));
    out.push((
        Duration::ZERO,
        Event::default().event("data").data("[DONE]"),
    ));
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

fn named_data(event: &str, value: Value) -> Event {
    Event::default().event(event).data(value.to_string())
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
    let source = text
        .rsplit_once("\n\n")
        .map_or(text, |(_, message)| message);
    let cleaned = source
        .replace("[[slow]]", "")
        .replace("[[error]]", "")
        .replace("[[rich]]", "")
        .replace("[[think]]", "")
        .trim()
        .to_owned();
    let title = cleaned.chars().take(60).collect::<String>();
    let split_index = title.chars().count() / 2;
    let split = title
        .char_indices()
        .nth(split_index)
        .map_or(title.len(), |(index, _)| index);
    let (first, second) = title.split_at(split);
    json!({
        "id":"msg-fake-title", "type":"message", "role":"assistant", "model":model_name(body),
        "content":[{"type":"text","text":first},{"type":"text","text":second}],
        "stop_reason":"end_turn","usage":{"input_tokens":8,"output_tokens":title.len() as u64,"cost":0.00001}
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
