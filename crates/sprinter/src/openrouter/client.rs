use async_stream::try_stream;
use futures_util::{Stream, StreamExt};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use std::{pin::Pin, time::Duration};

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
    pub cost: Option<f64>,
}

#[derive(Clone, Debug)]
pub enum ProviderEvent {
    Delta(String),
    Annotations(Value),
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
        pdf_engine: Option<&str>,
    ) -> Result<ProviderStream, ProviderError> {
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(key.expose_secret())
            .header("HTTP-Referer", "https://github.com/cau777/sprinter")
            .header("X-Title", "Sprinter")
            .json(&request_body(model, messages, true, pdf_engine))
            .send()
            .await
            .map_err(|_| ProviderError {
                message: "Could not connect to OpenRouter".into(),
                status: None,
            })?;
        if !response.status().is_success() {
            return Err(provider_error(
                response.status().as_u16(),
                response.json().await.ok(),
            ));
        }

        let mut stream = response.bytes_stream();
        let output = try_stream! {
            let mut buffer = String::new();
            let mut finish_reason: Option<String> = None;
            let mut final_usage: Option<ProviderUsage> = None;
            let mut done_sent = false;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| ProviderError {
                    message: "OpenRouter connection ended unexpectedly".into(), status: None,
                })?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(end) = buffer.find("\n\n") {
                    let frame = buffer[..end].to_owned();
                    buffer.drain(..end + 2);
                    if let Some(data) = frame_data(&frame) {
                        if data.trim() == "[DONE]" {
                            if !done_sent {
                                yield ProviderEvent::Done { finish_reason: finish_reason.take(), usage: final_usage.take() };
                                done_sent = true;
                            }
                            continue;
                        }
                        let value: Value = serde_json::from_str(data).map_err(|_| ProviderError {
                            message: "OpenRouter returned an invalid streaming response".into(), status: None,
                        })?;
                        if let Some(error) = value.get("error") {
                            let code = error.get("code").and_then(Value::as_u64).map(|v| v as u16);
                            let provider_error = provider_error(code.unwrap_or(502), Some(value.clone()));
                            Err(provider_error)?;
                        }
                        let choice = value.pointer("/choices/0");
                        if let Some(reason) = choice.and_then(|c| c.get("finish_reason")).and_then(Value::as_str) {
                            finish_reason = Some(reason.to_owned());
                        }
                        if let Some(content) = choice.and_then(|c| c.pointer("/delta/content")).and_then(Value::as_str) {
                            if !content.is_empty() { yield ProviderEvent::Delta(content.to_owned()); }
                        }
                        if let Some(annotations) = choice
                            .and_then(|c| c.pointer("/message/annotations").or_else(|| c.pointer("/delta/annotations")))
                        {
                            if let Some(items) = annotations.as_array() {
                                for item in items { yield ProviderEvent::Annotations(item.clone()); }
                            } else {
                                yield ProviderEvent::Annotations(annotations.clone());
                            }
                        }
                        if let Some(usage) = value.get("usage") {
                            final_usage = parse_usage(usage);
                        }
                    }
                }
            }
            if !done_sent {
                if !buffer.trim().is_empty() {
                    if let Some(data) = frame_data(&buffer) {
                        if data.trim() == "[DONE]" {
                            yield ProviderEvent::Done { finish_reason: finish_reason.take(), usage: final_usage.take() };
                            done_sent = true;
                        }
                    }
                }
                if !done_sent {
                    Err(ProviderError { message: "OpenRouter stream ended without a completion marker".into(), status: None })?;
                }
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
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(key.expose_secret())
            .header("HTTP-Referer", "https://github.com/cau777/sprinter")
            .header("X-Title", "Sprinter")
            .json(&request_body(model, &messages, false, None))
            .send()
            .await
            .map_err(|_| ProviderError {
                message: "Could not connect to OpenRouter".into(),
                status: None,
            })?;
        if !response.status().is_success() {
            return Err(provider_error(
                response.status().as_u16(),
                response.json().await.ok(),
            ));
        }
        let body: Value = response.json().await.map_err(|_| ProviderError {
            message: "OpenRouter returned an invalid title response".into(),
            status: None,
        })?;
        body.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| ProviderError {
                message: "OpenRouter returned no conversation title".into(),
                status: None,
            })
    }
}

fn request_body(
    model: &str,
    messages: &[ChatMessage],
    stream: bool,
    pdf_engine: Option<&str>,
) -> Value {
    let mut plugins = vec![json!({"id":"context-compression", "enabled":false})];
    if let Some(engine) = pdf_engine {
        plugins.push(json!({"id":"file-parser", "pdf":{"engine":engine}}));
    }
    let mut body = json!({
        "model": model,
        "messages": messages.iter().map(|message| json!({"role":message.role,"content":message.content})).collect::<Vec<_>>(),
        "stream": stream,
        "reasoning": {"exclude": true},
        "plugins": plugins
    });
    if !stream {
        body["max_tokens"] = json!(64);
    }
    body
}

fn frame_data(frame: &str) -> Option<&str> {
    frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .next()
}

fn parse_usage(value: &Value) -> Option<ProviderUsage> {
    Some(ProviderUsage {
        prompt_tokens: value.get("prompt_tokens").and_then(Value::as_i64),
        completion_tokens: value.get("completion_tokens").and_then(Value::as_i64),
        reasoning_tokens: value
            .pointer("/completion_tokens_details/reasoning_tokens")
            .and_then(Value::as_i64),
        cost: value.get("cost").and_then(Value::as_f64),
    })
}

fn provider_error(status: u16, body: Option<Value>) -> ProviderError {
    let upstream = body
        .as_ref()
        .and_then(|v| v.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let message = match status {
        401 => "OpenRouter rejected the saved API key. Check Settings.",
        402 => "OpenRouter has insufficient credits for this request.",
        429 => "OpenRouter rate limited this request. Try again shortly.",
        _ if body
            .as_ref()
            .and_then(|v| v.pointer("/error/code"))
            .and_then(Value::as_str)
            == Some("context_length_exceeded") =>
        {
            "This conversation is too long for the selected model. Switch to a model with a larger context window or start a new chat."
        }
        _ if status >= 500 => "OpenRouter is temporarily unavailable.",
        _ if !upstream.is_empty() => upstream,
        _ => "OpenRouter rejected this request.",
    };
    ProviderError {
        message: message.to_owned(),
        status: Some(status),
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatMessage, OpenRouterClient, ProviderEvent};
    use fake_openrouter::FakeOpenRouter;
    use futures_util::StreamExt;
    use secrecy::SecretString;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn streaming_request_uses_supported_flags_and_parses_final_usage() {
        let fake = FakeOpenRouter::new();
        let requests = fake.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let client = OpenRouterClient::new(format!("http://{address}/api/v1"));
        let messages = [ChatMessage {
            role: "user".into(),
            content: Value::String("protocol check".into()),
        }];
        let mut stream = client
            .stream_chat(
                &SecretString::from("test-key"),
                "test/text",
                &messages,
                Some("cloudflare-ai"),
            )
            .await
            .unwrap();
        let mut deltas = String::new();
        let mut cost = None;
        let mut finished = false;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                ProviderEvent::Delta(content) => deltas.push_str(&content),
                ProviderEvent::Annotations(_) => {}
                ProviderEvent::Done { usage, .. } => {
                    cost = usage.and_then(|usage| usage.cost);
                    finished = true;
                }
            }
        }
        assert!(deltas.contains("protocol check"));
        assert!(finished);
        assert_eq!(cost, Some(0.0003));
        let request = requests
            .requests_snapshot()
            .into_iter()
            .find(|request| request.path == "/api/v1/chat/completions")
            .unwrap();
        assert_eq!(request.body["stream"], true);
        assert_eq!(request.body["reasoning"]["exclude"], true);
        assert_eq!(request.body["plugins"][0]["id"], "context-compression");
        assert_eq!(request.body["plugins"][0]["enabled"], false);
        assert_eq!(request.body["plugins"][1]["pdf"]["engine"], "cloudflare-ai");
        assert_eq!(
            request.headers.get("http-referer").map(String::as_str),
            Some("https://github.com/cau777/sprinter")
        );
        server.abort();
    }

    #[tokio::test]
    async fn provider_error_chunks_are_reported_after_http_200() {
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
                None,
            )
            .await
            .unwrap();
        let mut deltas = false;
        let error = loop {
            match stream.next().await.unwrap() {
                Ok(ProviderEvent::Delta(_)) => deltas = true,
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        assert!(deltas);
        assert_eq!(error.status, Some(429));
        assert!(error.message.contains("rate limited"));
        server.abort();
    }

    #[tokio::test]
    async fn pdf_content_part_is_sent_and_stream_annotations_are_emitted() {
        let fake = FakeOpenRouter::new();
        let requests = fake.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let client = OpenRouterClient::new(format!("http://{address}/api/v1"));
        let messages = [ChatMessage {
            role: "user".into(),
            content: json!([{"type":"file","file":{"filename":"scan.pdf","file_data":"data:application/pdf;base64,JVBERg=="}}]),
        }];
        let mut stream = client
            .stream_chat(
                &SecretString::from("test-key"),
                "test/text",
                &messages,
                Some("cloudflare-ai"),
            )
            .await
            .unwrap();
        let mut annotations = Vec::new();
        while let Some(event) = stream.next().await {
            if let ProviderEvent::Annotations(annotation) = event.unwrap() {
                annotations.push(annotation);
            }
        }
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0]["file"]["name"], "scan.pdf");
        let request = requests
            .requests_snapshot()
            .into_iter()
            .find(|request| request.path == "/api/v1/chat/completions")
            .unwrap();
        assert_eq!(request.body["messages"][0]["content"][0]["type"], "file");
        assert_eq!(request.body["plugins"][1]["pdf"]["engine"], "cloudflare-ai");
        server.abort();
    }
}
