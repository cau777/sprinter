use async_stream::try_stream;
use futures_util::{Stream, StreamExt};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use std::{collections::HashSet, pin::Pin, time::Duration};

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: Value,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderUsage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cost: Option<f64>,
}

#[derive(Clone, Debug)]
pub enum ProviderEvent {
    Delta {
        content: String,
        provider: Option<String>,
        generation_id: Option<String>,
    },
    Done {
        finish_reason: Option<String>,
        usage: Option<ProviderUsage>,
    },
}

pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send>>;

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    pub message: String,
    pub status: Option<u16>,
}

#[derive(Clone)]
pub struct OpenRouterClient {
    http: reqwest::Client,
    base_url: String,
}

impl OpenRouterClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .expect("HTTP client configuration is valid"),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    pub async fn stream_chat(
        &self,
        key: &SecretString,
        model: &str,
        messages: &[ChatMessage],
        native_pdf_fallback: bool,
        session_id: &str,
    ) -> Result<ProviderStream, ProviderError> {
        let response = self
            .http
            .post(format!("{}/messages", self.base_url))
            .bearer_auth(key.expose_secret())
            .header("HTTP-Referer", "https://github.com/cau777/sprinter")
            .header("X-Title", "Sprinter")
            .json(&request_body(
                model,
                messages,
                true,
                native_pdf_fallback,
                Some(session_id),
            ))
            .send()
            .await
            .map_err(|_| ProviderError {
                message: "Could not connect to OpenRouter".into(),
                status: None,
            })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(provider_error(Some(status), response.json().await.ok()));
        }

        let mut stream = response.bytes_stream();
        let output = try_stream! {
            let mut buffer = Vec::new();
            let mut parser = MessagesStreamParser::default();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| ProviderError {
                    message: "OpenRouter connection ended unexpectedly".into(),
                    status: None,
                })?;
                buffer.extend_from_slice(&chunk);
                while let Some(frame) = take_sse_frame(&mut buffer) {
                    if let Some(data) = frame_data(&frame)? {
                        if let Some(event) = parser.parse_data(&data)? {
                            yield event;
                        }
                    }
                }
            }
            if !buffer.is_empty() {
                if let Some(data) = frame_data(&buffer)? {
                    if let Some(event) = parser.parse_data(&data)? {
                        yield event;
                    }
                }
            }
            if !parser.completed {
                Err(ProviderError {
                    message: "OpenRouter stream ended without a completion marker".into(),
                    status: None,
                })?;
            }
        };
        Ok(Box::pin(output))
    }

    pub async fn complete_title(
        &self,
        key: &SecretString,
        model: &str,
        user_text: &str,
    ) -> Result<String, ProviderError> {
        let messages = [ChatMessage {
            role: "user".into(),
            content: Value::String(format!(
                "Write a concise title for this conversation. Reply with only the title.\n\n{user_text}"
            )),
        }];
        let response = self
            .http
            .post(format!("{}/messages", self.base_url))
            .bearer_auth(key.expose_secret())
            .header("HTTP-Referer", "https://github.com/cau777/sprinter")
            .header("X-Title", "Sprinter")
            .json(&request_body(model, &messages, false, false, None))
            .send()
            .await
            .map_err(|_| ProviderError {
                message: "Could not connect to OpenRouter".into(),
                status: None,
            })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(provider_error(Some(status), response.json().await.ok()));
        }
        let body: Value = response.json().await.map_err(|_| ProviderError {
            message: "OpenRouter returned an invalid title response".into(),
            status: None,
        })?;
        parse_title(&body).ok_or_else(|| ProviderError {
            message: "OpenRouter returned no conversation title".into(),
            status: None,
        })
    }
}

fn request_body(
    model: &str,
    messages: &[ChatMessage],
    stream: bool,
    native_pdf_fallback: bool,
    session_id: Option<&str>,
) -> Value {
    let mut plugins = vec![json!({"id":"context-compression", "enabled":false})];
    if native_pdf_fallback {
        plugins.push(json!({"id":"file-parser", "pdf":{"engine":"native"}}));
    }
    let mut system_parts = Vec::new();
    let api_messages = messages
        .iter()
        .filter_map(|message| {
            if message.role == "system" {
                if let Some(text) = message
                    .content
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                {
                    system_parts.push(text);
                }
                None
            } else {
                Some(json!({"role":message.role,"content":message.content}))
            }
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": model,
        "messages": api_messages,
        "stream": stream,
        "plugins": plugins
    });
    let system = system_parts.join("\n\n");
    if !system.is_empty() {
        body["system"] = Value::String(system);
    }
    if let Some(session_id) = session_id {
        body["session_id"] = Value::String(session_id.to_owned());
    }
    if !stream {
        body["max_tokens"] = json!(64);
    }
    body
}

#[derive(Default)]
struct MessagesStreamParser {
    text_blocks: HashSet<i64>,
    provider: Option<String>,
    generation_id: Option<String>,
    finish_reason: Option<String>,
    usage: Option<ProviderUsage>,
    completed: bool,
}

impl MessagesStreamParser {
    fn parse_data(&mut self, data: &str) -> Result<Option<ProviderEvent>, ProviderError> {
        if data.trim() == "[DONE]" || self.completed {
            return Ok(None);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| ProviderError {
            message: "OpenRouter returned an invalid streaming response".into(),
            status: None,
        })?;
        match value.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(message) = value.get("message") {
                    self.generation_id =
                        message.get("id").and_then(Value::as_str).map(str::to_owned);
                    self.provider = message
                        .get("provider")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    self.usage = merge_usage(
                        self.usage.take(),
                        message.get("usage").and_then(parse_usage),
                    );
                }
            }
            Some("content_block_start") => {
                let index = value.get("index").and_then(Value::as_i64);
                let block_type = value.pointer("/content_block/type").and_then(Value::as_str);
                if let (Some(index), Some("text")) = (index, block_type) {
                    self.text_blocks.insert(index);
                }
            }
            Some("content_block_delta") => {
                let index = value.get("index").and_then(Value::as_i64);
                let delta = value.get("delta");
                if index.is_some_and(|index| self.text_blocks.contains(&index))
                    && delta
                        .and_then(|delta| delta.get("type"))
                        .and_then(Value::as_str)
                        == Some("text_delta")
                    && let Some(content) = delta
                        .and_then(|delta| delta.get("text"))
                        .and_then(Value::as_str)
                    && !content.is_empty()
                {
                    return Ok(Some(ProviderEvent::Delta {
                        content: content.to_owned(),
                        provider: self.provider.clone(),
                        generation_id: self.generation_id.clone(),
                    }));
                }
            }
            Some("content_block_stop") => {
                if let Some(index) = value.get("index").and_then(Value::as_i64) {
                    self.text_blocks.remove(&index);
                }
            }
            Some("message_delta") => {
                self.finish_reason = value
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                    .map(map_finish_reason)
                    .or_else(|| self.finish_reason.take());
                self.usage =
                    merge_usage(self.usage.take(), value.get("usage").and_then(parse_usage));
            }
            Some("message_stop") => {
                self.completed = true;
                return Ok(Some(ProviderEvent::Done {
                    finish_reason: self.finish_reason.take(),
                    usage: self.usage.take(),
                }));
            }
            Some("error") => return Err(provider_error(None, Some(value))),
            _ => {}
        }
        Ok(None)
    }
}

fn merge_usage(old: Option<ProviderUsage>, new: Option<ProviderUsage>) -> Option<ProviderUsage> {
    match (old, new) {
        (None, usage) | (usage, None) => usage,
        (Some(old), Some(new)) => Some(ProviderUsage {
            prompt_tokens: new.prompt_tokens.or(old.prompt_tokens),
            completion_tokens: new.completion_tokens.or(old.completion_tokens),
            reasoning_tokens: new.reasoning_tokens.or(old.reasoning_tokens),
            cache_read_tokens: new.cache_read_tokens.or(old.cache_read_tokens),
            cost: new.cost.or(old.cost),
        }),
    }
}

fn parse_usage(value: &Value) -> Option<ProviderUsage> {
    let input_tokens = value.get("input_tokens").and_then(Value::as_i64);
    let cache_read_tokens = value.get("cache_read_input_tokens").and_then(Value::as_i64);
    let cache_creation_tokens = value
        .get("cache_creation_input_tokens")
        .and_then(Value::as_i64);
    let has_prompt_tokens =
        input_tokens.is_some() || cache_read_tokens.is_some() || cache_creation_tokens.is_some();
    Some(ProviderUsage {
        prompt_tokens: has_prompt_tokens.then(|| {
            input_tokens.unwrap_or_default()
                + cache_read_tokens.unwrap_or_default()
                + cache_creation_tokens.unwrap_or_default()
        }),
        completion_tokens: value.get("output_tokens").and_then(Value::as_i64),
        reasoning_tokens: value
            .pointer("/output_tokens_details/thinking_tokens")
            .and_then(Value::as_i64),
        cache_read_tokens,
        cost: value.get("cost").and_then(Value::as_f64),
    })
}

fn map_finish_reason(reason: &str) -> String {
    match reason {
        "end_turn" | "stop_sequence" => "stop".into(),
        "max_tokens" => "length".into(),
        "tool_use" => "tool_calls".into(),
        "refusal" => "content_filter".into(),
        other => other.to_owned(),
    }
}

fn take_sse_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let lf = find_bytes(buffer, b"\n\n").map(|index| (index, 2));
    let crlf = find_bytes(buffer, b"\r\n\r\n").map(|index| (index, 4));
    let (index, separator_len) = match (lf, crlf) {
        (Some(lf), Some(crlf)) => {
            if lf.0 <= crlf.0 {
                lf
            } else {
                crlf
            }
        }
        (Some(lf), None) => lf,
        (None, Some(crlf)) => crlf,
        (None, None) => return None,
    };
    let mut frame = buffer.drain(..index + separator_len).collect::<Vec<_>>();
    frame.truncate(index);
    Some(frame)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn frame_data(frame: &[u8]) -> Result<Option<String>, ProviderError> {
    let frame = std::str::from_utf8(frame).map_err(|_| ProviderError {
        message: "OpenRouter returned an invalid streaming response".into(),
        status: None,
    })?;
    let mut lines = Vec::new();
    for raw_line in frame.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(data) = line.strip_prefix("data:") {
            lines.push(data.strip_prefix(' ').unwrap_or(data));
        }
    }
    Ok((!lines.is_empty()).then(|| lines.join("\n")))
}

fn parse_title(body: &Value) -> Option<String> {
    let title = body
        .get("content")?
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<String>()
        .trim()
        .to_owned();
    (!title.is_empty()).then_some(title)
}

fn provider_error(status: Option<u16>, body: Option<Value>) -> ProviderError {
    let error = body.as_ref().and_then(|value| value.get("error"));
    let upstream = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let error_type = error
        .and_then(|error| error.get("error_type").or_else(|| error.get("type")))
        .and_then(Value::as_str)
        .unwrap_or("");
    let lower_upstream = upstream.to_ascii_lowercase();
    let context_error = lower_upstream.contains("maximum context length");
    let message = match status {
        Some(401) => "OpenRouter rejected the saved API key. Check Settings.",
        Some(402) => "OpenRouter has insufficient credits for this request.",
        Some(429) => "OpenRouter rate limited this request. Try again shortly.",
        Some(400) if context_error => {
            "This conversation is too long for the selected model. Switch to a model with a larger context window or start a new chat."
        }
        Some(status) if status >= 500 => "OpenRouter is temporarily unavailable.",
        _ if status.is_none() && error_type == "authentication" => {
            "OpenRouter rejected the saved API key. Check Settings."
        }
        _ if status.is_none() && error_type == "payment_required" => {
            "OpenRouter has insufficient credits for this request."
        }
        _ if status.is_none() && error_type == "rate_limit_exceeded" => {
            "OpenRouter rate limited this request. Try again shortly."
        }
        _ if (status == Some(400) || error_type == "invalid_request") && context_error => {
            "This conversation is too long for the selected model. Switch to a model with a larger context window or start a new chat."
        }
        _ if status.is_some_and(|status| status >= 500) => "OpenRouter is temporarily unavailable.",
        _ if !upstream.is_empty() => upstream,
        _ => "OpenRouter rejected this request.",
    };
    ProviderError {
        message: message.to_owned(),
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatMessage, OpenRouterClient, ProviderEvent, ProviderUsage, provider_error};
    use fake_openrouter::FakeOpenRouter;
    use futures_util::StreamExt;
    use secrecy::SecretString;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn messages_stream_maps_prompt_blocks_and_parses_metadata_and_usage() {
        let fake = FakeOpenRouter::new();
        let requests = fake.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let client = OpenRouterClient::new(format!("http://{address}/api/v1"));
        let session_id = "0123456789abcdef0123456789abcdef";
        let messages = [
            ChatMessage {
                role: "system".into(),
                content: Value::String("Keep the answer concise".into()),
            },
            ChatMessage {
                role: "user".into(),
                content: json!([
                    {"type":"text","text":"protocol check [[think]]"},
                    {"type":"image","source":{"type":"base64","media_type":"image/webp","data":"aW1hZ2U="}},
                    {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"JVBERg=="},"title":"scan.pdf"}
                ]),
            },
        ];
        let mut stream = client
            .stream_chat(
                &SecretString::from("test-key"),
                "test/text",
                &messages,
                true,
                session_id,
            )
            .await
            .unwrap();
        let mut output = String::new();
        let mut done = None;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                ProviderEvent::Delta {
                    content,
                    provider,
                    generation_id,
                } => {
                    assert_eq!(provider.as_deref(), Some("Azure"));
                    assert_eq!(generation_id.as_deref(), Some("gen-fake"));
                    output.push_str(&content);
                }
                ProviderEvent::Done {
                    finish_reason,
                    usage,
                } => done = Some((finish_reason, usage)),
            }
        }
        assert_eq!(output, "You said: protocol check");
        let (finish, usage) = done.unwrap();
        assert_eq!(finish.as_deref(), Some("stop"));
        let usage = usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(19));
        assert_eq!(usage.completion_tokens, Some(4));
        assert_eq!(usage.reasoning_tokens, Some(2));
        assert_eq!(usage.cache_read_tokens, Some(5));
        assert_eq!(usage.cost, Some(0.0003));

        let request = requests
            .requests_snapshot()
            .into_iter()
            .find(|request| request.path == "/api/v1/messages" && request.body["stream"] == true)
            .unwrap();
        assert_eq!(request.body["model"], "test/text");
        assert_eq!(request.body["system"], "Keep the answer concise");
        assert_eq!(request.body["session_id"], session_id);
        assert_eq!(request.body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(request.body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(request.body["messages"][0]["content"][1]["type"], "image");
        assert_eq!(
            request.body["messages"][0]["content"][2]["type"],
            "document"
        );
        assert_eq!(
            request.body["messages"][0]["content"][2]["title"],
            "scan.pdf"
        );
        assert_eq!(request.body["plugins"][0]["id"], "context-compression");
        assert_eq!(request.body["plugins"][0]["enabled"], false);
        assert_eq!(request.body["plugins"][1]["id"], "file-parser");
        assert_eq!(request.body["plugins"][1]["pdf"]["engine"], "native");
        assert!(request.body.get("max_tokens").is_none());
        assert!(request.body.get("reasoning").is_none());
        assert_eq!(
            request.headers.get("http-referer").map(String::as_str),
            Some("https://github.com/cau777/sprinter")
        );
        assert_eq!(
            request.headers.get("x-title").map(String::as_str),
            Some("Sprinter")
        );
        assert!(!request.headers.contains_key("anthropic-version"));
        server.abort();
    }

    #[tokio::test]
    async fn title_uses_messages_without_a_session_and_joins_text_blocks() {
        let fake = FakeOpenRouter::new();
        let requests = fake.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let client = OpenRouterClient::new(format!("http://{address}/api/v1"));
        let title = client
            .complete_title(
                &SecretString::from("test-key"),
                "test/title",
                "A concise title",
            )
            .await
            .unwrap();
        assert_eq!(title, "A concise title");
        let request = requests.requests_snapshot().into_iter().next().unwrap();
        assert_eq!(request.path, "/api/v1/messages");
        assert_eq!(request.body["stream"], false);
        assert_eq!(request.body["max_tokens"], 64);
        assert!(request.body.get("session_id").is_none());
        assert_eq!(request.body["messages"][0]["role"], "user");
        assert!(
            request.body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("A concise title")
        );
        server.abort();
    }

    #[tokio::test]
    async fn http_errors_use_status_and_messages_api_error_shape() {
        let fake = FakeOpenRouter::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let client = OpenRouterClient::new(format!("http://{address}/api/v1"));
        let messages = [ChatMessage {
            role: "user".into(),
            content: Value::String("HTTP error check".into()),
        }];
        for (status, expected) in [
            (400, "too long"),
            (401, "Check Settings"),
            (402, "insufficient credits"),
            (429, "rate limited"),
            (503, "temporarily unavailable"),
        ] {
            let result = client
                .stream_chat(
                    &SecretString::from("test-key"),
                    &format!("test/http-error-{status}"),
                    &messages,
                    false,
                    "0123456789abcdef0123456789abcdef",
                )
                .await;
            let error = match result {
                Ok(_) => panic!("expected HTTP {status} to fail"),
                Err(error) => error,
            };
            assert_eq!(error.status, Some(status));
            assert!(
                error
                    .message
                    .to_ascii_lowercase()
                    .contains(&expected.to_ascii_lowercase())
            );
        }
        server.abort();
    }

    #[tokio::test]
    async fn midstream_errors_are_mapped_after_partial_text() {
        let fake = FakeOpenRouter::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let client = OpenRouterClient::new(format!("http://{address}/api/v1"));
        let messages = [ChatMessage {
            role: "user".into(),
            content: Value::String("[[error]]".into()),
        }];
        let mut stream = client
            .stream_chat(
                &SecretString::from("test-key"),
                "test/text",
                &messages,
                false,
                "f".repeat(32).as_str(),
            )
            .await
            .unwrap();
        let mut partial = String::new();
        let error = loop {
            match stream.next().await.unwrap() {
                Ok(ProviderEvent::Delta { content, .. }) => partial.push_str(&content),
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        assert_eq!(partial, "Partial reply");
        assert_eq!(error.status, None);
        assert!(error.message.contains("rate limited"));
        server.abort();
    }

    #[tokio::test]
    async fn stream_requires_message_stop_and_ignores_the_done_tail() {
        let fake = FakeOpenRouter::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let client = OpenRouterClient::new(format!("http://{address}/api/v1"));
        let messages = [ChatMessage {
            role: "user".into(),
            content: Value::String("[[truncated]]".into()),
        }];
        let mut stream = client
            .stream_chat(
                &SecretString::from("test-key"),
                "test/text",
                &messages,
                false,
                "e".repeat(32).as_str(),
            )
            .await
            .unwrap();
        let mut saw_error = false;
        while let Some(event) = stream.next().await {
            if let Err(error) = event {
                assert!(error.message.contains("without a completion marker"));
                saw_error = true;
            }
        }
        assert!(saw_error);
        server.abort();
    }

    #[test]
    fn usage_merges_cached_input_and_maps_all_documented_finish_reasons() {
        let initial = super::parse_usage(&json!({
            "input_tokens": 3,
            "cache_read_input_tokens": 12510,
            "cache_creation_input_tokens": null
        }))
        .unwrap();
        let final_usage = super::parse_usage(&json!({
            "output_tokens": 7,
            "output_tokens_details": {"thinking_tokens": 4},
            "cost": 0.25
        }))
        .unwrap();
        let merged = super::merge_usage(Some(initial), Some(final_usage)).unwrap();
        assert_eq!(merged.prompt_tokens, Some(12513));
        assert_eq!(merged.completion_tokens, Some(7));
        assert_eq!(merged.reasoning_tokens, Some(4));
        assert_eq!(merged.cache_read_tokens, Some(12510));
        assert_eq!(merged.cost, Some(0.25));
        assert_eq!(super::map_finish_reason("end_turn"), "stop");
        assert_eq!(super::map_finish_reason("stop_sequence"), "stop");
        assert_eq!(super::map_finish_reason("max_tokens"), "length");
        assert_eq!(super::map_finish_reason("tool_use"), "tool_calls");
        assert_eq!(super::map_finish_reason("refusal"), "content_filter");
        assert_eq!(
            super::map_finish_reason("provider_specific"),
            "provider_specific"
        );
    }

    #[test]
    fn errors_follow_status_then_error_type_and_message_fallbacks() {
        let cases = [
            (
                Some(401),
                json!({"error":{"message":"bad key"}}),
                "Check Settings",
            ),
            (
                Some(402),
                json!({"error":{"message":"no credits"}}),
                "insufficient credits",
            ),
            (
                Some(429),
                json!({"error":{"message":"slow down"}}),
                "rate limited",
            ),
            (
                Some(400),
                json!({"error":{"message":"This endpoint's maximum context length is 200000 tokens"}}),
                "too long",
            ),
            (
                Some(503),
                json!({"error":{"message":"maintenance"}}),
                "temporarily unavailable",
            ),
            (Some(418), json!({"error":{"message":"teapot"}}), "teapot"),
        ];
        for (status, body, expected) in cases {
            let error = provider_error(status, Some(body));
            assert!(
                error
                    .message
                    .to_ascii_lowercase()
                    .contains(&expected.to_ascii_lowercase())
            );
        }
        let rate_limited = provider_error(
            None,
            Some(
                json!({"type":"error","error":{"type":"rate_limit_error","error_type":"rate_limit_exceeded","message":"limited"}}),
            ),
        );
        assert!(rate_limited.message.contains("rate limited"));
        let context = provider_error(
            None,
            Some(
                json!({"error":{"type":"invalid_request_error","error_type":"invalid_request","message":"maximum context length exceeded"}}),
            ),
        );
        assert!(context.message.contains("too long"));
        assert_eq!(
            provider_error(None, None).message,
            "OpenRouter rejected this request."
        );
    }

    #[test]
    fn title_joins_text_blocks_trims_and_ignores_other_blocks() {
        let body = json!({"content":[
            {"type":"thinking","thinking":"hidden"},
            {"type":"text","text":"  A concise "},
            {"type":"text","text":"title  "}
        ]});
        assert_eq!(
            super::parse_title(&body).as_deref(),
            Some("A concise title")
        );
        assert!(super::parse_title(&json!({"content":[{"type":"text","text":"  "}]})).is_none());
    }

    #[test]
    fn sse_framing_handles_crlf_and_utf8_split_across_network_chunks() {
        let bytes = b"event: content_block_delta\r\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"caf\xc3\xa9\"}}\r\n\r\n";
        let split = bytes.iter().position(|byte| *byte == 0xc3).unwrap() + 1;
        let mut buffer = bytes[..split].to_vec();
        assert!(super::take_sse_frame(&mut buffer).is_none());
        buffer.extend_from_slice(&bytes[split..]);
        let frame = super::take_sse_frame(&mut buffer).unwrap();
        let data = super::frame_data(&frame).unwrap().unwrap();
        assert!(data.contains("café"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn usage_parser_tolerates_missing_fields() {
        let usage: Option<ProviderUsage> = super::parse_usage(&json!({}));
        assert!(usage.is_some());
        let usage = usage.unwrap();
        assert_eq!(usage.prompt_tokens, None);
        assert_eq!(usage.completion_tokens, None);
        assert_eq!(usage.reasoning_tokens, None);
        assert_eq!(usage.cache_read_tokens, None);
        assert_eq!(usage.cost, None);
    }
}
