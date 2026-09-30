use crate::{
    auth,
    messages::{MessageRecord, PreparedGeneration},
    openrouter::{ChatMessage, OpenRouterClient, ProviderEvent, ProviderUsage},
    settings,
    state::AppState,
    tools::{Citation, ToolStep},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::StreamExt;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::json;
use sqlx::SqlitePool;
use std::convert::Infallible;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    sync::{Mutex, broadcast, watch},
    task::JoinHandle,
    time::{Instant, interval, timeout, timeout_at},
};
use tracing::Instrument;

const GLOBAL_LIMIT: usize = 8;
const FLUSH_EVERY: Duration = Duration::from_secs(1);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(8);

#[derive(Clone)]
pub struct Manager {
    pool: SqlitePool,
    client: OpenRouterClient,
    active: Arc<Mutex<HashMap<String, Arc<Running>>>>,
}

struct Running {
    id: String,
    chat_id: String,
    buffer: RwLock<Buffer>,
    events: broadcast::Sender<StreamEvent>,
    cancel: watch::Sender<Option<StopReason>>,
    finished: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

struct Buffer {
    content: String,
    tools: Vec<String>,
    citations: Vec<Citation>,
    tool_steps: Vec<ToolStep>,
    web_search_requests: Option<i64>,
    tool_cost: Option<f64>,
    tool_fallback: bool,
    terminal: Option<StreamEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopReason {
    User,
    Shutdown,
}

pub struct Subscription {
    pub snapshot: String,
    pub metadata: StreamSnapshot,
    pub initial: Option<StreamEvent>,
    pub events: broadcast::Receiver<StreamEvent>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamSnapshot {
    pub content: String,
    pub tools: Vec<String>,
    pub citations: Vec<Citation>,
    pub tool_steps: Vec<ToolStep>,
    pub web_search_requests: Option<i64>,
    pub tool_cost: Option<f64>,
    pub tool_fallback: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum StreamEvent {
    #[serde(rename = "delta")]
    Delta { content: String },
    #[serde(rename = "done")]
    Done {
        status: String,
        finish_reason: Option<String>,
        usage: Option<StreamUsage>,
        cost: Option<f64>,
        web_search_requests: Option<i64>,
        tool_cost: Option<f64>,
        tool_fallback: bool,
    },
    #[serde(rename = "citations")]
    Citations { items: Vec<Citation> },
    #[serde(rename = "tool_step")]
    ToolStep { step: ToolStep },
    #[serde(rename = "error")]
    Error { status: String, message: String },
    #[serde(rename = "title")]
    Title { chat_id: String, title: String },
}

impl StreamEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Delta { .. } => "delta",
            Self::Done { .. } => "done",
            Self::Citations { .. } => "citations",
            Self::ToolStep { .. } => "tool_step",
            Self::Error { .. } => "error",
            Self::Title { .. } => "title",
        }
    }
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/messages/{id}/stream", get(stream_message))
        .route("/api/messages/{id}/cancel", post(cancel_message))
        .route_layer(middleware::from_fn_with_state(state, auth::require_session))
}

async fn stream_message(
    Path(message_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let request_id = auth::request_id(&headers);
    let subscription = match state.generation.subscribe(&message_id).await {
        Ok(subscription) => subscription,
        Err(GenerationError::NotFound) => {
            return auth::api_error(
                &request_id,
                StatusCode::NOT_FOUND,
                "not_found",
                "Message not found",
                None,
            );
        }
        Err(_) => {
            return auth::api_error(
                &request_id,
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Could not open message stream",
                None,
            );
        }
    };
    let mut events = subscription.events;
    let output = async_stream::stream! {
        let snapshot = Event::default()
            .event("snapshot")
            .json_data(&subscription.metadata)
            .unwrap_or_else(|_| Event::default().event("snapshot").data("{}"));
        yield Ok::<Event, Infallible>(snapshot);
        if let Some(initial) = subscription.initial {
            yield Ok(stream_event(initial));
        }
        loop {
            match events.recv().await {
                Ok(event) => yield Ok(stream_event(event)),
                Err(broadcast::error::RecvError::Lagged(_)) => break,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    let mut response = Sse::new(output)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("heartbeat"),
        )
        .into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-cache".parse().unwrap());
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().unwrap());
    response
}

async fn cancel_message(
    Path(message_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    match state.generation.cancel_message(&message_id).await {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(GenerationError::NotFound) => auth::api_error(
            &auth::request_id(&headers),
            StatusCode::NOT_FOUND,
            "not_found",
            "Message is not generating",
            None,
        ),
        Err(_) => auth::api_error(
            &auth::request_id(&headers),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Could not cancel generation",
            None,
        ),
    }
}

fn stream_event(event: StreamEvent) -> Event {
    let name = event.name();
    let data = match event {
        StreamEvent::Delta { content } => json!({"content": content}),
        StreamEvent::Done {
            status,
            finish_reason,
            usage,
            cost,
            web_search_requests,
            tool_cost,
            tool_fallback,
        } => {
            json!({
                "status": status,
                "finish_reason": finish_reason,
                "usage": usage,
                "cost": cost,
                "web_search_requests": web_search_requests,
                "tool_cost": tool_cost,
                "tool_fallback": tool_fallback
            })
        }
        StreamEvent::Citations { items } => json!({"items": items}),
        StreamEvent::ToolStep { step } => json!({"step": step}),
        StreamEvent::Error { status, message } => json!({"status": status, "message": message}),
        StreamEvent::Title { chat_id, title } => json!({"chat_id": chat_id, "title": title}),
    };
    Event::default()
        .event(name)
        .json_data(data)
        .unwrap_or_else(|_| {
            Event::default()
                .event("error")
                .data("{\"status\":\"error\"}")
        })
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamUsage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

#[derive(Debug, Error)]
pub enum GenerationError {
    #[error("a generation is already active in this chat")]
    GenerationInProgress,
    #[error("the server is already handling eight generations")]
    TooManyGenerations,
    #[error("the message was not found")]
    NotFound,
    #[error("could not start generation")]
    Internal,
}

impl Manager {
    pub fn new(pool: SqlitePool, base_url: impl Into<String>) -> Self {
        Self {
            pool,
            client: OpenRouterClient::new(base_url),
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start(
        &self,
        app: &AppState,
        prepared: PreparedGeneration,
        key: SecretString,
        started_by: Option<String>,
    ) -> Result<(), GenerationError> {
        let assistant_id = prepared.assistant_message.id.clone();
        let chat_id = prepared.chat_id.clone();
        let tools = prepared
            .assistant_message
            .tools
            .clone()
            .unwrap_or_else(|| "[]".into());
        let stored = sqlx::query(
            "UPDATE messages SET tools = ?, citations = NULL, tool_steps = NULL, web_search_requests = NULL, tool_cost = NULL, tool_fallback = 0 WHERE id = ? AND status = 'streaming'",
        )
        .bind(&tools)
        .bind(&assistant_id)
        .execute(&self.pool)
        .await
        .map_err(|_| GenerationError::Internal)?;
        if stored.rows_affected() == 0 {
            return Err(GenerationError::NotFound);
        }
        let session_id = ensure_chat_session_id(&self.pool, &chat_id)
            .await
            .map_err(|_| GenerationError::Internal)?;
        let (events, _) = broadcast::channel(256);
        let (cancel, cancel_rx) = watch::channel(None);
        let (finished, _) = watch::channel(false);
        let running = Arc::new(Running {
            id: assistant_id.clone(),
            chat_id: chat_id.clone(),
            buffer: RwLock::new(Buffer {
                content: prepared.assistant_message.content.clone(),
                tools: serde_json::from_str(&tools).unwrap_or_default(),
                citations: Vec::new(),
                tool_steps: Vec::new(),
                web_search_requests: None,
                tool_cost: None,
                tool_fallback: false,
                terminal: None,
            }),
            events,
            cancel,
            finished,
            task: Mutex::new(None),
        });
        {
            let mut active = self.active.lock().await;
            if active.len() >= GLOBAL_LIMIT {
                return Err(GenerationError::TooManyGenerations);
            }
            if active.values().any(|item| item.chat_id == chat_id) {
                return Err(GenerationError::GenerationInProgress);
            }
            active.insert(assistant_id.clone(), running.clone());
        }

        let title_info = title_info(app, &prepared).await;
        let manager = self.clone();
        let state = app.clone();
        let task_running = running.clone();
        let generation_span = tracing::info_span!(
            "generation",
            gen = %assistant_id,
            chat = %chat_id,
            model = %prepared.model,
            tools = %prepared.active_tools.join(","),
            tz = prepared.timezone.as_deref().unwrap_or(""),
            started_by = started_by.as_deref().unwrap_or("")
        );
        let task = tokio::spawn(async move {
            manager
                .run_generation(
                    state,
                    prepared,
                    assistant_id,
                    key,
                    session_id,
                    started_by,
                    task_running,
                    cancel_rx,
                    title_info,
                )
                .instrument(generation_span)
                .await;
        });
        *running.task.lock().await = Some(task);
        Ok(())
    }

    pub async fn subscribe(&self, message_id: &str) -> Result<Subscription, GenerationError> {
        let active = self.active.lock().await;
        if let Some(running) = active.get(message_id) {
            // Delta append+broadcast also holds this lock, so each delta appears in
            // either the snapshot or the receiver, without a gap or duplication.
            let buffer = running.buffer.write().expect("generation buffer poisoned");
            let metadata = StreamSnapshot {
                content: buffer.content.clone(),
                tools: buffer.tools.clone(),
                citations: buffer.citations.clone(),
                tool_steps: buffer.tool_steps.clone(),
                web_search_requests: buffer.web_search_requests,
                tool_cost: buffer.tool_cost,
                tool_fallback: buffer.tool_fallback,
            };
            let initial = buffer.terminal.clone();
            let events = running.events.subscribe();
            return Ok(Subscription {
                snapshot: metadata.content.clone(),
                metadata,
                initial,
                events,
            });
        }
        drop(active);
        let row = sqlx::query_as::<_, (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<f64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<f64>,
            i64,
        )>(
            "SELECT content, status, finish_reason, error, prompt_tokens, completion_tokens, reasoning_tokens, cost, citations, tool_steps, tools, web_search_requests, tool_cost, tool_fallback FROM messages WHERE id = ?",
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GenerationError::Internal)?
        .ok_or(GenerationError::NotFound)?;
        let snapshot = StreamSnapshot {
            content: row.0.clone(),
            tools: crate::tools::decode_json_column(row.10.as_deref()),
            citations: crate::tools::decode_json_column(row.8.as_deref()),
            tool_steps: crate::tools::decode_json_column(row.9.as_deref()),
            web_search_requests: row.11,
            tool_cost: row.12,
            tool_fallback: row.13 != 0,
        };
        let initial = match row.1.as_str() {
            "complete" | "cancelled" | "interrupted" => Some(StreamEvent::Done {
                status: row.1.clone(),
                finish_reason: row.2,
                usage: usage_from_values(row.4, row.5, row.6),
                cost: row.7,
                web_search_requests: row.11,
                tool_cost: row.12,
                tool_fallback: row.13 != 0,
            }),
            "error" => Some(StreamEvent::Error {
                status: "error".into(),
                message: row.3.unwrap_or_else(|| "Generation failed".into()),
            }),
            _ => None,
        };
        let (sender, events) = broadcast::channel(1);
        drop(sender);
        Ok(Subscription {
            snapshot: snapshot.content.clone(),
            metadata: snapshot,
            initial,
            events,
        })
    }

    pub async fn cancel_message(&self, message_id: &str) -> Result<(), GenerationError> {
        let active = self.active.lock().await;
        let running = active.get(message_id).ok_or(GenerationError::NotFound)?;
        let _ = running.cancel.send(Some(StopReason::User));
        Ok(())
    }

    pub async fn cancel_chat(&self, chat_id: &str) -> Result<(), GenerationError> {
        let active = self.active.lock().await;
        let running = active
            .values()
            .find(|running| running.chat_id == chat_id)
            .ok_or(GenerationError::NotFound)?;
        let _ = running.cancel.send(Some(StopReason::User));
        Ok(())
    }

    pub async fn recover_interrupted(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT id, tool_steps FROM messages WHERE status = 'streaming'",
        )
        .fetch_all(pool)
        .await?;
        let count = rows.len() as u64;
        let mut tx = pool.begin().await?;
        for (id, raw_steps) in rows {
            let mut steps = crate::tools::decode_json_column::<Vec<ToolStep>>(raw_steps.as_deref());
            let mut steps_changed = false;
            for step in &mut steps {
                if step.status == "running" {
                    step.status = "error".into();
                    steps_changed = true;
                }
            }
            let steps =
                steps_changed.then(|| serde_json::to_string(&steps).expect("tool steps serialize"));
            sqlx::query("UPDATE messages SET status = 'interrupted', error = 'Server restarted during generation', tool_steps = COALESCE(?, tool_steps), updated_at = ? WHERE id = ? AND status = 'streaming'")
                .bind(steps)
                .bind(now_ms())
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(count)
    }

    pub async fn shutdown(&self) {
        let running = self
            .active
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut finished = Vec::with_capacity(running.len());
        for item in &running {
            finished.push(item.finished.subscribe());
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        for done in &mut finished {
            let _ = timeout_at(deadline, done.changed()).await;
        }
        for item in &running {
            if !*item.finished.borrow() {
                let _ = item.cancel.send(Some(StopReason::Shutdown));
                self.mark_interrupted(item).await;
                if let Some(task) = item.task.lock().await.take() {
                    task.abort();
                }
            }
        }
    }

    async fn run_generation(
        &self,
        app: AppState,
        prepared: PreparedGeneration,
        assistant_id: String,
        key: SecretString,
        session_id: String,
        started_by: Option<String>,
        running: Arc<Running>,
        mut cancel_rx: watch::Receiver<Option<StopReason>>,
        title_info: Option<TitleInfo>,
    ) {
        let generation_started = Instant::now();
        let title_task = title_info.map(|info| {
            spawn_title_task(
                self.client.clone(),
                self.pool.clone(),
                key.clone(),
                running.events.clone(),
                info,
            )
        });
        let instructions = settings::custom_instructions(&app).await.ok().flatten();
        let limits = settings::upload_limits(&app).await.unwrap_or_default();
        let image_support = settings::model_supports_images(&app, &prepared.model)
            .await
            .unwrap_or(false);
        let file_support = settings::model_supports_files(&app, &prepared.model)
            .await
            .unwrap_or(false);
        let expansion = crate::uploads::expand_prompt(
            &self.pool,
            &app.config.data_dir,
            &prepared.prompt,
            image_support,
            file_support,
            limits.total_prompt_bytes,
        )
        .await;
        let result = match expansion {
            Err(error) => {
                self.finish_error(&assistant_id, &running, error.to_string())
                    .await;
                None
            }
            Ok(expanded) => {
                tracing::info!(
                    gen = %assistant_id,
                    chat = %prepared.chat_id,
                    model = %prepared.model,
                    path_len = prepared.prompt.len(),
                    pdf_text = expanded.pdf_text,
                    pdf_native = expanded.pdf_native,
                    pdf_omitted = expanded.pdf_omitted,
                    tools = ?prepared.active_tools,
                    tz = prepared.timezone.as_deref().unwrap_or(""),
                    started_by = started_by.as_deref().unwrap_or(""),
                    "started"
                );
                tracing::debug!(chat = %prepared.chat_id, session_id = %session_id, "OpenRouter sticky session");
                let messages = prompt_for_provider(
                    instructions.as_deref(),
                    &expanded.messages,
                    prepared.timezone.as_deref(),
                );
                let native_pdf_fallback = expanded.needs_native_pdf_plugin();
                let tool_config = crate::openrouter::ToolRequestConfig {
                    enabled: prepared.active_tools.clone(),
                    web_search_only: prepared.web_search_only.clone(),
                };
                tokio::select! {
                    changed = cancel_rx.changed() => {
                        if changed.is_ok() && cancel_rx.borrow().is_some() {
                            if *cancel_rx.borrow() == Some(StopReason::Shutdown) {
                                self.finish_interrupted(&assistant_id, &running).await;
                            } else {
                                self.finish_cancelled(&assistant_id, &running).await;
                            }
                            None
                        } else {
                            Some(self.client.stream_chat_with_tools(&key, &prepared.model, &messages, native_pdf_fallback, &session_id, &tool_config).await)
                        }
                    }
                    result = self.client.stream_chat_with_tools(&key, &prepared.model, &messages, native_pdf_fallback, &session_id, &tool_config) => Some(result)
                }
            }
        };
        match result {
            None => {}
            Some(Err(error)) => {
                self.finish_error(&assistant_id, &running, error.message)
                    .await;
            }
            Some(Ok(mut stream)) => {
                let mut ticker = interval(FLUSH_EVERY);
                ticker.tick().await;
                let mut final_reason = None;
                let mut final_usage = None;
                let mut finished = false;
                let mut was_cancelled = false;
                let mut first_token = false;
                let mut provider_seen = None;
                let mut tool_started = HashMap::<String, Instant>::new();
                loop {
                    tokio::select! {
                        changed = cancel_rx.changed() => {
                            if changed.is_ok() && cancel_rx.borrow().is_some() {
                                was_cancelled = *cancel_rx.borrow() == Some(StopReason::User);
                                break;
                            }
                        }
                        _ = ticker.tick() => self.flush(&assistant_id, &running).await,
                        event = stream.next() => match event {
                            Some(Ok(ProviderEvent::Delta { content, provider, generation_id })) => {
                                provider_seen = provider.clone().or(provider_seen);
                                if !content.is_empty() && !first_token {
                                    first_token = true;
                                    tracing::info!(
                                        ttft_ms = generation_started.elapsed().as_millis(),
                                        provider = provider.as_deref(),
                                        or_id = generation_id.as_deref(),
                                        "first token"
                                    );
                                }
                                let mut buffer = running.buffer.write().expect("generation buffer poisoned");
                                buffer.content.push_str(&content);
                                let _ = running.events.send(StreamEvent::Delta { content });
                            }
                            Some(Ok(ProviderEvent::Citation(citation))) => {
                                let mut buffer = running.buffer.write().expect("generation buffer poisoned");
                                if !buffer.citations.iter().any(|item| item.url == citation.url) {
                                    tracing::debug!(citation_url = %citation.url, citation_title = %citation.title, "search citation received");
                                    buffer.citations.push(citation.clone());
                                    let _ = running.events.send(StreamEvent::Citations { items: vec![citation] });
                                }
                            }
                            Some(Ok(ProviderEvent::ToolStep(mut step))) => {
                                let now = Instant::now();
                                let mut buffer = running.buffer.write().expect("generation buffer poisoned");
                                if let Some(existing) = buffer.tool_steps.iter_mut().find(|item| item.id == step.id) {
                                    step.offset = existing.offset;
                                    *existing = step.clone();
                                } else {
                                    step.offset = buffer.content.chars().count();
                                    buffer.tool_steps.push(step.clone());
                                }
                                if step.status == "running" {
                                    tool_started.entry(step.id.clone()).or_insert(now);
                                }
                                let _ = running.events.send(StreamEvent::ToolStep { step: step.clone() });
                                drop(buffer);
                                if step.status == "done" {
                                    let duration_ms = tool_started
                                        .remove(&step.id)
                                        .map(|started| started.elapsed().as_millis())
                                        .unwrap_or_default();
                                    let stdout_bytes = step.output.as_ref().map_or(0, |output| output.stdout.len());
                                    let stderr_bytes = step.output.as_ref().map_or(0, |output| output.stderr.len());
                                    let command_preview = step
                                        .input
                                        .as_ref()
                                        .and_then(|input| input.get("command"))
                                        .and_then(serde_json::Value::as_str)
                                        .map(|command| log_preview(command, 200));
                                    tracing::info!(
                                        tool = %step.tool,
                                        exit_code = ?step.output.as_ref().and_then(|output| output.exit_code),
                                        duration_ms,
                                        stdout_bytes,
                                        stderr_bytes,
                                        command_preview = command_preview.as_deref().unwrap_or(""),
                                        "tool step"
                                    );
                                    tracing::debug!(tool_step = ?step, "tool step details");
                                }
                            }
                            Some(Ok(ProviderEvent::Done { finish_reason, usage })) => {
                                final_reason = finish_reason;
                                final_usage = usage;
                                finished = true;
                                break;
                            }
                            Some(Err(error)) => {
                                self.finish_error(&assistant_id, &running, error.message).await;
                                break;
                            }
                            None => {
                                self.finish_error(&assistant_id, &running, "OpenRouter stream ended unexpectedly".into()).await;
                                break;
                            }
                        }
                    }
                }
                if cancel_rx.borrow().is_some() && !was_cancelled {
                    self.finish_interrupted(&assistant_id, &running).await;
                } else if was_cancelled {
                    self.finish_cancelled(&assistant_id, &running).await;
                } else if finished {
                    self.finish_complete(
                        &app,
                        &assistant_id,
                        &prepared.model,
                        &prepared.active_tools,
                        provider_seen.as_deref(),
                        final_reason,
                        final_usage,
                        &running,
                        generation_started.elapsed().as_millis(),
                    )
                    .await;
                }
            }
        }
        if let Some(task) = title_task {
            let _ = timeout(Duration::from_secs(10), task).await;
        }
        if let Some(started_by) = started_by {
            tracing::debug!(message_id = %assistant_id, actor = %started_by, "generation task finished");
        }
        let _ = running.finished.send(true);
        self.active.lock().await.remove(&assistant_id);
    }

    async fn flush(&self, id: &str, running: &Running) {
        let (content, citations, tool_steps, web_search_requests, tool_cost, tool_fallback) = {
            let buffer = running.buffer.read().expect("generation buffer poisoned");
            (
                buffer.content.clone(),
                encode_optional_json(&buffer.citations),
                encode_optional_json(&buffer.tool_steps),
                buffer.web_search_requests,
                buffer.tool_cost,
                buffer.tool_fallback,
            )
        };
        if let Err(error) = sqlx::query(
            "UPDATE messages SET content = ?, citations = ?, tool_steps = ?, web_search_requests = ?, tool_cost = ?, tool_fallback = ?, updated_at = ? WHERE id = ? AND status = 'streaming'",
        )
        .bind(content)
        .bind(citations)
        .bind(tool_steps)
        .bind(web_search_requests)
        .bind(tool_cost)
        .bind(tool_fallback)
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await {
            tracing::error!(message_id = %id, error = %error, "could not flush generation content");
        }
    }

    async fn finish_complete(
        &self,
        app: &AppState,
        id: &str,
        model: &str,
        active_tools: &[String],
        provider: Option<&str>,
        finish_reason: Option<String>,
        usage: Option<ProviderUsage>,
        running: &Running,
        duration_ms: u128,
    ) {
        let (prompt, completion, reasoning, cost) =
            usage.as_ref().map_or((None, None, None, None), |usage| {
                (
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.reasoning_tokens,
                    usage.cost,
                )
            });
        let cache_read_tokens = usage
            .as_ref()
            .and_then(|usage| usage.cache_read_tokens)
            .unwrap_or_default();
        let web_search_requests = usage.as_ref().and_then(|usage| usage.web_search_requests);
        let tool_cost = if active_tools.is_empty() {
            None
        } else {
            usage.as_ref().and_then(derived_tool_cost)
        };
        let tool_fallback = active_tools
            .iter()
            .any(|tool| tool == crate::tools::WEB_SEARCH)
            && usage.as_ref().is_some_and(search_fallback_usage);
        if tool_fallback {
            settings::mark_web_search_fallback(app, model).await;
            tracing::warn!(
                tool = crate::tools::WEB_SEARCH,
                model,
                provider = provider.unwrap_or(""),
                tool_cost = tool_cost.unwrap_or_default(),
                "tool fallback"
            );
        }

        let (content, citations, tool_steps, tools, citation_count, tool_step_count) = {
            let mut buffer = running.buffer.write().expect("generation buffer poisoned");
            for step in &mut buffer.tool_steps {
                if step.status == "running" {
                    step.status = if step.tool == crate::tools::WEB_SEARCH {
                        "done".into()
                    } else {
                        "error".into()
                    };
                    let _ = running
                        .events
                        .send(StreamEvent::ToolStep { step: step.clone() });
                    if step.tool == crate::tools::WEB_SEARCH {
                        tracing::info!(
                            tool = %step.tool,
                            exit_code = Option::<i64>::None,
                            duration_ms = 0_u128,
                            stdout_bytes = 0_usize,
                            stderr_bytes = 0_usize,
                            command_preview = "",
                            "tool step"
                        );
                    }
                }
            }
            buffer.web_search_requests = web_search_requests;
            buffer.tool_cost = tool_cost;
            buffer.tool_fallback = tool_fallback;
            (
                buffer.content.clone(),
                encode_optional_json(&buffer.citations),
                encode_optional_json(&buffer.tool_steps),
                buffer.tools.clone(),
                buffer.citations.len(),
                buffer.tool_steps.len(),
            )
        };
        if let Err(error) = sqlx::query("UPDATE messages SET content = ?, citations = ?, tool_steps = ?, web_search_requests = ?, tool_cost = ?, tool_fallback = ?, status = 'complete', error = NULL, finish_reason = ?, prompt_tokens = ?, completion_tokens = ?, reasoning_tokens = ?, cost = ?, updated_at = ? WHERE id = ?")
            .bind(&content).bind(citations).bind(tool_steps).bind(web_search_requests).bind(tool_cost).bind(tool_fallback).bind(&finish_reason).bind(prompt).bind(completion).bind(reasoning).bind(cost).bind(now_ms()).bind(id).execute(&self.pool).await
        {
            tracing::error!(message_id = %id, error = %error, "could not persist completed generation");
        }
        let preview = log_preview(&content, 200);
        let subscribers = running.events.receiver_count();
        tracing::info!(
            gen = %id,
            model,
            status = "complete",
            finish = finish_reason.as_deref().unwrap_or(""),
            duration_ms,
            prompt_tokens = prompt.unwrap_or_default(),
            completion_tokens = completion.unwrap_or_default(),
            reasoning_tokens = reasoning.unwrap_or_default(),
            cache_read_tokens,
            cost = cost.unwrap_or_default(),
            tools = ?tools,
            web_search_requests = web_search_requests.unwrap_or_default(),
            citations = citation_count,
            tool_steps = tool_step_count,
            tool_cost = tool_cost.unwrap_or_default(),
            subscribers,
            chars = content.chars().count(),
            preview,
            "completed"
        );
        let event = StreamEvent::Done {
            status: "complete".into(),
            finish_reason,
            usage: usage.map(|usage| StreamUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                reasoning_tokens: usage.reasoning_tokens,
            }),
            cost,
            web_search_requests,
            tool_cost,
            tool_fallback,
        };
        let mut buffer = running.buffer.write().expect("generation buffer poisoned");
        buffer.terminal = Some(event.clone());
        let _ = running.events.send(event);
    }

    async fn finish_cancelled(&self, id: &str, running: &Running) {
        close_open_tool_steps(running);
        let snapshot = capture_snapshot(running);
        let content = snapshot.content.clone();
        let chars = content.chars().count();
        let preview = log_preview(&content, 200);
        let _ = persist_partial_snapshot(&self.pool, id, &snapshot, "cancelled", None).await;
        tracing::info!(
            gen = %id,
            status = "cancelled",
            chars,
            preview,
            "cancelled"
        );
        let event = StreamEvent::Done {
            status: "cancelled".into(),
            finish_reason: None,
            usage: None,
            cost: None,
            web_search_requests: snapshot.web_search_requests,
            tool_cost: snapshot.tool_cost,
            tool_fallback: snapshot.tool_fallback,
        };
        let mut buffer = running.buffer.write().expect("generation buffer poisoned");
        buffer.terminal = Some(event.clone());
        let _ = running.events.send(event);
    }

    async fn finish_interrupted(&self, id: &str, running: &Running) {
        close_open_tool_steps(running);
        let snapshot = capture_snapshot(running);
        let content = snapshot.content.clone();
        let chars = content.chars().count();
        let preview = log_preview(&content, 200);
        let _ = persist_partial_snapshot(
            &self.pool,
            id,
            &snapshot,
            "interrupted",
            Some("Server is shutting down"),
        )
        .await;
        tracing::warn!(
            gen = %id,
            status = "interrupted",
            chars,
            preview,
            "interrupted"
        );
        let event = StreamEvent::Done {
            status: "interrupted".into(),
            finish_reason: None,
            usage: None,
            cost: None,
            web_search_requests: snapshot.web_search_requests,
            tool_cost: snapshot.tool_cost,
            tool_fallback: snapshot.tool_fallback,
        };
        let mut buffer = running.buffer.write().expect("generation buffer poisoned");
        buffer.terminal = Some(event.clone());
        let _ = running.events.send(event);
    }

    async fn mark_interrupted(&self, running: &Running) {
        close_open_tool_steps(running);
        let snapshot = capture_snapshot(running);
        let content = snapshot.content.clone();
        let chars = content.chars().count();
        let preview = log_preview(&content, 200);
        let _ = persist_partial_snapshot(
            &self.pool,
            &running.id,
            &snapshot,
            "interrupted",
            Some("Server shutdown deadline expired"),
        )
        .await;
        tracing::warn!(
            gen = %running.id,
            chat = %running.chat_id,
            status = "interrupted",
            chars,
            preview,
            "interrupted"
        );
    }

    async fn finish_error(&self, id: &str, running: &Running, message: String) {
        close_open_tool_steps(running);
        let snapshot = capture_snapshot(running);
        let content = snapshot.content.clone();
        let chars = content.chars().count();
        let preview = log_preview(&content, 200);
        let error = log_preview(&message, 200);
        let _ = persist_partial_snapshot(&self.pool, id, &snapshot, "error", Some(&message)).await;
        tracing::warn!(
            gen = %id,
            status = "error",
            chars,
            preview,
            error,
            "error"
        );
        let event = StreamEvent::Error {
            status: "error".into(),
            message,
        };
        let mut buffer = running.buffer.write().expect("generation buffer poisoned");
        buffer.terminal = Some(event.clone());
        let _ = running.events.send(event);
    }
}

fn capture_snapshot(running: &Running) -> StreamSnapshot {
    let buffer = running.buffer.read().expect("generation buffer poisoned");
    StreamSnapshot {
        content: buffer.content.clone(),
        tools: buffer.tools.clone(),
        citations: buffer.citations.clone(),
        tool_steps: buffer.tool_steps.clone(),
        web_search_requests: buffer.web_search_requests,
        tool_cost: buffer.tool_cost,
        tool_fallback: buffer.tool_fallback,
    }
}

fn close_open_tool_steps(running: &Running) {
    let mut buffer = running.buffer.write().expect("generation buffer poisoned");
    for step in &mut buffer.tool_steps {
        if step.status == "running" {
            step.status = "error".into();
            let _ = running
                .events
                .send(StreamEvent::ToolStep { step: step.clone() });
        }
    }
}

fn encode_optional_json<T: Serialize>(items: &[T]) -> Option<String> {
    (!items.is_empty()).then(|| serde_json::to_string(items).expect("tool metadata serializes"))
}

async fn persist_partial_snapshot(
    pool: &SqlitePool,
    id: &str,
    snapshot: &StreamSnapshot,
    status: &str,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE messages SET content = ?, citations = ?, tool_steps = ?, web_search_requests = ?, tool_cost = ?, tool_fallback = ?, status = ?, error = ?, updated_at = ? WHERE id = ? AND status = 'streaming'")
        .bind(&snapshot.content)
        .bind(encode_optional_json(&snapshot.citations))
        .bind(encode_optional_json(&snapshot.tool_steps))
        .bind(snapshot.web_search_requests)
        .bind(snapshot.tool_cost)
        .bind(snapshot.tool_fallback)
        .bind(status)
        .bind(error)
        .bind(now_ms())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

fn derived_tool_cost(usage: &ProviderUsage) -> Option<f64> {
    usage
        .cost
        .zip(usage.upstream_inference_cost)
        .map(|(cost, upstream)| (cost - upstream).max(0.0))
        .or(usage.server_tool_cost)
}

fn search_fallback_usage(usage: &ProviderUsage) -> bool {
    usage.tool_calls_requested.unwrap_or_default() > 0
        || usage.tool_calls_executed.unwrap_or_default() > 0
        || usage
            .cost
            .zip(usage.upstream_inference_cost)
            .is_some_and(|(cost, upstream)| cost - upstream > 0.000001)
}

fn log_preview(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let preview = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

#[derive(Clone)]
struct TitleInfo {
    chat_id: String,
    user_text: String,
    model: String,
}

async fn title_info(app: &AppState, prepared: &PreparedGeneration) -> Option<TitleInfo> {
    let user = prepared.user_message.as_ref()?;
    let is_first = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM messages WHERE chat_id = ? AND role = 'user'",
    )
    .bind(&prepared.chat_id)
    .fetch_one(&app.pool)
    .await
    .ok()?
        == 1;
    if !is_first {
        return None;
    }
    let title_model = settings::title_model(app)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| prepared.model.clone());
    Some(TitleInfo {
        chat_id: prepared.chat_id.clone(),
        user_text: user.content.clone(),
        model: title_model,
    })
}

fn spawn_title_task(
    client: OpenRouterClient,
    pool: SqlitePool,
    key: SecretString,
    events: broadcast::Sender<StreamEvent>,
    info: TitleInfo,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let fallback = info.user_text.chars().take(60).collect::<String>();
        let title = client
            .complete_title(&key, &info.model, &info.user_text)
            .await
            .map(|title| title.chars().take(60).collect::<String>())
            .unwrap_or(fallback);
        let update = sqlx::query(
            "UPDATE chats SET title = ?, updated_at = ? WHERE id = ? AND title_source = 'auto'",
        )
        .bind(&title)
        .bind(now_ms())
        .bind(&info.chat_id)
        .execute(&pool)
        .await;
        match update {
            Ok(result) if result.rows_affected() > 0 => {
                let _ = events.send(StreamEvent::Title {
                    chat_id: info.chat_id,
                    title,
                });
            }
            Ok(_) => {}
            Err(error) => tracing::error!(error = %error, "could not persist generated chat title"),
        }
    })
}

const MATH_FORMATTING_INSTRUCTIONS: &str = "When writing mathematics, use LaTeX delimiters: `$...$` for inline math and `$$...$$` for display math. Do not use plain square brackets as math delimiters.";

fn prompt_for_provider(
    instructions: Option<&str>,
    prompt: &[ChatMessage],
    timezone: Option<&str>,
) -> Vec<ChatMessage> {
    let mut system_parts = vec![MATH_FORMATTING_INSTRUCTIONS.to_owned()];
    if let Some(instructions) = instructions.filter(|value| !value.trim().is_empty()) {
        system_parts.push(instructions.to_owned());
    }
    match timezone {
        Some(name) => match name.parse::<chrono_tz::Tz>() {
            Ok(timezone) => {
                let date = chrono::Utc::now().with_timezone(&timezone);
                system_parts.push(format!(
                    "Today's date for the user: {} (time zone {}).",
                    date.format("%A, %Y-%m-%d"),
                    name
                ));
            }
            Err(_) => {
                tracing::debug!(timezone = %name, "local date omitted because the time zone is invalid")
            }
        },
        None => tracing::debug!("local date omitted because the time zone is missing"),
    }
    let mut messages = Vec::with_capacity(prompt.len() + 1);
    messages.push(ChatMessage {
        role: "system".into(),
        content: serde_json::Value::String(system_parts.join("\n\n")),
    });
    messages.extend_from_slice(prompt);
    messages
}

fn usage_from_values(
    prompt: Option<i64>,
    completion: Option<i64>,
    reasoning: Option<i64>,
) -> Option<StreamUsage> {
    (prompt.is_some() || completion.is_some() || reasoning.is_some()).then_some(StreamUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        reasoning_tokens: reasoning,
    })
}

async fn ensure_chat_session_id(pool: &SqlitePool, chat_id: &str) -> Result<String, sqlx::Error> {
    let current =
        sqlx::query_scalar::<_, Option<String>>("SELECT session_id FROM chats WHERE id = ?")
            .bind(chat_id)
            .fetch_optional(pool)
            .await?
            .flatten();
    if let Some(session_id) = current {
        return Ok(session_id);
    }

    let candidate = crate::chats::new_session_id();
    sqlx::query("UPDATE chats SET session_id = ? WHERE id = ? AND session_id IS NULL")
        .bind(candidate)
        .bind(chat_id)
        .execute(pool)
        .await?;
    sqlx::query_scalar::<_, Option<String>>("SELECT session_id FROM chats WHERE id = ?")
        .bind(chat_id)
        .fetch_optional(pool)
        .await?
        .flatten()
        .ok_or(sqlx::Error::RowNotFound)
}

pub fn mark_interrupted_on_start(
    pool: &SqlitePool,
) -> impl std::future::Future<Output = Result<u64, sqlx::Error>> + '_ {
    Manager::recover_interrupted(pool)
}

pub async fn message_record(pool: &SqlitePool, id: &str) -> Result<MessageRecord, GenerationError> {
    sqlx::query_as::<_, MessageRecord>("SELECT id, chat_id, parent_id, role, content, status, error, model, generation_id, finish_reason, prompt_tokens, completion_tokens, reasoning_tokens, cost, tools, citations, tool_steps, web_search_requests, tool_cost, tool_fallback, created_at, updated_at FROM messages WHERE id = ?")
        .bind(id).fetch_optional(pool).await.map_err(|_| GenerationError::Internal)?.ok_or(GenerationError::NotFound)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::{GenerationError, MATH_FORMATTING_INSTRUCTIONS, StreamEvent};
    use crate::{
        config::Config,
        db,
        messages::{self, SendMessageRequest},
        state::AppState,
    };
    use fake_openrouter::FakeOpenRouter;
    use secrecy::SecretString;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
    use tokio::{net::TcpListener, time::timeout};

    #[tokio::test]
    async fn late_subscriber_gets_snapshot_and_live_deltas_without_a_gap() {
        let (state, fake, server) = test_state().await;
        let prepared = send(&state, "hello generation").await;
        state
            .generation
            .start(
                &state,
                prepared.clone(),
                SecretString::from("test-key"),
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            state
                .generation
                .start(
                    &state,
                    prepared.clone(),
                    SecretString::from("test-key"),
                    None,
                )
                .await,
            Err(GenerationError::GenerationInProgress)
        ));

        let mut first = state
            .generation
            .subscribe(&prepared.assistant_message.id)
            .await
            .unwrap();
        let first_delta = loop {
            match timeout(Duration::from_secs(3), first.events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                StreamEvent::Delta { content } => break content,
                _ => {}
            }
        };
        let mut late = state
            .generation
            .subscribe(&prepared.assistant_message.id)
            .await
            .unwrap();
        let mut first_content = std::mem::take(&mut first.snapshot);
        first_content.push_str(&first_delta);
        let (first_content, first_status) = drain(first_content, &mut first).await;
        let late_snapshot = std::mem::take(&mut late.snapshot);
        let (late_content, late_status) = drain(late_snapshot, &mut late).await;
        assert_eq!(first_status.as_deref(), Some("complete"));
        assert_eq!(late_status.as_deref(), Some("complete"));
        assert_eq!(late_content, first_content);
        assert!(late_content.contains("You said: hello generation"));
        let row = sqlx::query_as::<_, (String, String, Option<f64>, Option<String>, Option<i64>, Option<i64>, Option<i64>)>(
            "SELECT content, status, cost, finish_reason, prompt_tokens, completion_tokens, reasoning_tokens FROM messages WHERE id = ?",
        )
        .bind(&prepared.assistant_message.id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(row.0, late_content);
        assert_eq!(row.1, "complete");
        assert_eq!(row.2, Some(0.0003));
        assert_eq!(row.3.as_deref(), Some("stop"));
        assert_eq!(row.4, Some(19));
        assert_eq!(row.5, Some(4));
        assert_eq!(row.6, Some(2));
        let session_id: String = sqlx::query_scalar("SELECT session_id FROM chats WHERE id = ?")
            .bind(&prepared.chat_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(session_id.len(), 32);
        assert!(session_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(fake.requests_snapshot().iter().any(|request| {
            request.path == "/api/v1/messages"
                && request.body["stream"] == true
                && request.body["session_id"] == session_id
                && request.body["system"].as_str().is_some_and(|system| {
                    system.contains(MATH_FORMATTING_INSTRUCTIONS)
                        && system.contains("Keep the answer concise")
                })
                && request.body.get("max_tokens").is_none()
                && request.body.get("reasoning").is_none()
        }));
        assert!(fake.requests_snapshot().iter().any(|request| {
            request.path == "/api/v1/messages"
                && request.body["stream"] == false
                && request.body["max_tokens"] == 64
                && request.body.get("session_id").is_none()
        }));
        server.abort();
        state.pool.close().await;
    }

    #[tokio::test]
    async fn cancel_preserves_partial_content_and_marks_cancelled() {
        let (state, _fake, server) = test_state().await;
        let prepared = send(&state, "[[slow]] keep partial").await;
        let id = prepared.assistant_message.id.clone();
        state
            .generation
            .start(&state, prepared, SecretString::from("test-key"), None)
            .await
            .unwrap();
        let mut subscription = state.generation.subscribe(&id).await.unwrap();
        loop {
            match timeout(Duration::from_secs(3), subscription.events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                StreamEvent::Delta { .. } => {
                    break;
                }
                _ => {}
            }
        }
        state.generation.cancel_message(&id).await.unwrap();
        let snapshot = std::mem::take(&mut subscription.snapshot);
        let (_, status) = drain(snapshot, &mut subscription).await;
        assert_eq!(status.as_deref(), Some("cancelled"));
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT content, status FROM messages WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(row.1, "cancelled");
        assert!(!row.0.is_empty());
        server.abort();
        state.pool.close().await;
    }

    #[tokio::test]
    async fn existing_chat_gets_one_stable_random_session_id_on_generation() {
        let (state, _fake, server) = test_state().await;
        let prepared = send(&state, "legacy chat session").await;
        sqlx::query("UPDATE chats SET session_id = NULL WHERE id = ?")
            .bind(&prepared.chat_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let first = super::ensure_chat_session_id(&state.pool, &prepared.chat_id)
            .await
            .unwrap();
        let second = super::ensure_chat_session_id(&state.pool, &prepared.chat_id)
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        server.abort();
        state.pool.close().await;
    }

    #[tokio::test]
    async fn scanned_pdf_uses_native_fallback_only_for_file_capable_models() {
        let (state, fake, server) = test_state().await;
        tokio::fs::create_dir_all(&state.config.data_dir)
            .await
            .unwrap();
        let upload = crate::uploads::store_stream(
            &state.pool,
            &state.config.data_dir,
            "scan.pdf",
            futures_util::stream::iter(vec![Ok::<_, std::io::Error>(
                axum::body::Bytes::from_static(b"%PDF-1.7 fake"),
            )]),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE uploads SET text_chars = 0, text_pages = 1, text_empty_pages = 1, text_extractor = 'pdfjs-test' WHERE id = ?")
            .bind(&upload.id)
            .execute(&state.pool)
            .await
            .unwrap();
        let prepared = messages::send_new_chat(
            &state.pool,
            Some("test/file"),
            SendMessageRequest {
                parent_id: None,
                content: "summarize this".into(),
                attachment_ids: vec![upload.id.clone()],
                model: None,
            },
        )
        .await
        .unwrap();
        state
            .generation
            .start(
                &state,
                prepared.clone(),
                SecretString::from("test-key"),
                None,
            )
            .await
            .unwrap();
        let mut subscription = state
            .generation
            .subscribe(&prepared.assistant_message.id)
            .await
            .unwrap();
        let snapshot = std::mem::take(&mut subscription.snapshot);
        let (_, status) = drain(snapshot, &mut subscription).await;
        assert_eq!(status.as_deref(), Some("complete"));
        let request = fake
            .requests_snapshot()
            .into_iter()
            .find(|request| request.path == "/api/v1/messages" && request.body["stream"] == true)
            .unwrap();
        let content = request.body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "user")
            .unwrap()["content"]
            .as_array()
            .unwrap();
        assert_eq!(content[1]["type"], "document");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "application/pdf");
        assert_eq!(content[1]["title"], "scan.pdf");
        assert!(content[1]["source"]["data"].as_str().is_some());
        assert_eq!(request.body["plugins"][1]["id"], "file-parser");
        assert_eq!(request.body["plugins"][1]["pdf"]["engine"], "native");
        server.abort();
        state.pool.close().await;
    }

    async fn drain(
        mut content: String,
        subscription: &mut super::Subscription,
    ) -> (String, Option<String>) {
        let mut status = match subscription.initial.take() {
            Some(StreamEvent::Done { status, .. }) | Some(StreamEvent::Error { status, .. }) => {
                Some(status)
            }
            _ => None,
        };
        loop {
            let received = timeout(Duration::from_secs(3), subscription.events.recv()).await;
            match received {
                Ok(Ok(StreamEvent::Delta { content: delta })) => content.push_str(&delta),
                Ok(Ok(StreamEvent::Done {
                    status: final_status,
                    ..
                })) => {
                    status = Some(final_status);
                }
                Ok(Ok(StreamEvent::Error {
                    status: final_status,
                    ..
                })) => {
                    status = Some(final_status);
                }
                Ok(Ok(StreamEvent::Title { .. })) => {}
                Ok(Ok(StreamEvent::Citations { .. } | StreamEvent::ToolStep { .. })) => {}
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                    panic!("subscriber lagged")
                }
                Err(_) if status.is_some() => break,
                Err(_) => panic!("generation event stream timed out"),
            }
        }
        (content, status)
    }

    async fn test_state() -> (AppState, FakeOpenRouter, tokio::task::JoinHandle<()>) {
        let fake = FakeOpenRouter::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let base_url = format!("http://{address}/api/v1");
        let requests = fake.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(":memory:")
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        db::migrate(&pool).await.unwrap();
        let config = Arc::new(Config {
            master_password: Some(SecretString::from("test-password")),
            data_dir: PathBuf::from("/tmp/sprinter-generation-test"),
            bind_address: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            trusted_proxies: Vec::new(),
            worker_threads: 2,
            log_filter: "warn".into(),
            log_keep_days: 7,
            insecure_cookies: true,
            openrouter_base_url: base_url,
        });
        sqlx::query(
            "INSERT INTO settings(key, value, updated_at) VALUES('custom_instructions', ?, 1)",
        )
        .bind(serde_json::to_string("Keep the answer concise").unwrap())
        .execute(&pool)
        .await
        .unwrap();
        let state = AppState::new(pool, config);
        (state, requests, server)
    }

    async fn send(state: &AppState, content: &str) -> crate::messages::PreparedGeneration {
        messages::send_new_chat(
            &state.pool,
            Some("test/text"),
            SendMessageRequest {
                parent_id: None,
                content: content.into(),
                attachment_ids: Vec::new(),
                model: None,
            },
        )
        .await
        .unwrap()
    }
}
