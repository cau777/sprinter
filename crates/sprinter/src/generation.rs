use crate::{
    auth,
    messages::{MessageRecord, PreparedGeneration, PromptMessage},
    openrouter::{ChatMessage, OpenRouterClient, ProviderEvent, ProviderUsage},
    settings,
    state::AppState,
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
    terminal: Option<StreamEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopReason {
    User,
    Shutdown,
}

pub struct Subscription {
    pub snapshot: String,
    pub initial: Option<StreamEvent>,
    pub events: broadcast::Receiver<StreamEvent>,
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
    },
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
            .json_data(json!({"content": subscription.snapshot}))
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
        } => {
            json!({"status": status, "finish_reason": finish_reason, "usage": usage, "cost": cost})
        }
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
        let (events, _) = broadcast::channel(256);
        let (cancel, cancel_rx) = watch::channel(None);
        let (finished, _) = watch::channel(false);
        let running = Arc::new(Running {
            id: assistant_id.clone(),
            chat_id: chat_id.clone(),
            buffer: RwLock::new(Buffer {
                content: prepared.assistant_message.content.clone(),
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
        let task = tokio::spawn(async move {
            manager
                .run_generation(
                    state,
                    prepared,
                    assistant_id,
                    key,
                    started_by,
                    task_running,
                    cancel_rx,
                    title_info,
                )
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
            let snapshot = buffer.content.clone();
            let initial = buffer.terminal.clone();
            let events = running.events.subscribe();
            return Ok(Subscription {
                snapshot,
                initial,
                events,
            });
        }
        drop(active);
        let row = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<i64>, Option<i64>, Option<i64>, Option<f64>)>(
            "SELECT content, status, finish_reason, error, prompt_tokens, completion_tokens, reasoning_tokens, cost FROM messages WHERE id = ?",
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GenerationError::Internal)?
        .ok_or(GenerationError::NotFound)?;
        let initial = match row.1.as_str() {
            "complete" | "cancelled" | "interrupted" => Some(StreamEvent::Done {
                status: row.1.clone(),
                finish_reason: row.2,
                usage: usage_from_values(row.4, row.5, row.6),
                cost: row.7,
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
            snapshot: row.0,
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
        Ok(sqlx::query("UPDATE messages SET status = 'interrupted', error = 'Server restarted during generation', updated_at = ? WHERE status = 'streaming'")
            .bind(now_ms()).execute(pool).await?.rows_affected())
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
        started_by: Option<String>,
        running: Arc<Running>,
        mut cancel_rx: watch::Receiver<Option<StopReason>>,
        title_info: Option<TitleInfo>,
    ) {
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
        let messages = prompt_for_provider(instructions.as_deref(), &prepared.prompt);
        let result = tokio::select! {
            changed = cancel_rx.changed() => {
                if changed.is_ok() && cancel_rx.borrow().is_some() {
                    if *cancel_rx.borrow() == Some(StopReason::Shutdown) {
                        self.finish_interrupted(&assistant_id, &running).await;
                    } else {
                        self.finish_cancelled(&assistant_id, &running).await;
                    }
                    None
                } else {
                    Some(self.client.stream_chat(&key, &prepared.model, &messages, prepared.pdf_engine.as_deref()).await)
                }
            }
            result = self.client.stream_chat(&key, &prepared.model, &messages, prepared.pdf_engine.as_deref()) => Some(result)
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
                            Some(Ok(ProviderEvent::Delta(content))) => {
                                let mut buffer = running.buffer.write().expect("generation buffer poisoned");
                                buffer.content.push_str(&content);
                                let _ = running.events.send(StreamEvent::Delta { content });
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
                        &assistant_id,
                        &prepared.model,
                        final_reason,
                        final_usage,
                        &running,
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
        let content = running
            .buffer
            .read()
            .expect("generation buffer poisoned")
            .content
            .clone();
        if let Err(error) = sqlx::query(
            "UPDATE messages SET content = ?, updated_at = ? WHERE id = ? AND status = 'streaming'",
        )
        .bind(content)
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await
        {
            tracing::error!(message_id = %id, error = %error, "could not flush generation content");
        }
    }

    async fn finish_complete(
        &self,
        id: &str,
        model: &str,
        finish_reason: Option<String>,
        usage: Option<ProviderUsage>,
        running: &Running,
    ) {
        let content = running
            .buffer
            .read()
            .expect("generation buffer poisoned")
            .content
            .clone();
        let (prompt, completion, reasoning, cost) =
            usage.as_ref().map_or((None, None, None, None), |usage| {
                (
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.reasoning_tokens,
                    usage.cost,
                )
            });
        if let Err(error) = sqlx::query("UPDATE messages SET content = ?, status = 'complete', error = NULL, finish_reason = ?, prompt_tokens = ?, completion_tokens = ?, reasoning_tokens = ?, cost = ?, updated_at = ? WHERE id = ?")
            .bind(content).bind(&finish_reason).bind(prompt).bind(completion).bind(reasoning).bind(cost).bind(now_ms()).bind(id).execute(&self.pool).await
        {
            tracing::error!(message_id = %id, error = %error, "could not persist completed generation");
        }
        let _ = model;
        let event = StreamEvent::Done {
            status: "complete".into(),
            finish_reason,
            usage: usage.map(|usage| StreamUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                reasoning_tokens: usage.reasoning_tokens,
            }),
            cost,
        };
        let mut buffer = running.buffer.write().expect("generation buffer poisoned");
        buffer.terminal = Some(event.clone());
        let _ = running.events.send(event);
    }

    async fn finish_cancelled(&self, id: &str, running: &Running) {
        let content = running
            .buffer
            .read()
            .expect("generation buffer poisoned")
            .content
            .clone();
        let _ = sqlx::query(
            "UPDATE messages SET content = ?, status = 'cancelled', updated_at = ? WHERE id = ?",
        )
        .bind(content)
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await;
        let event = StreamEvent::Done {
            status: "cancelled".into(),
            finish_reason: None,
            usage: None,
            cost: None,
        };
        let mut buffer = running.buffer.write().expect("generation buffer poisoned");
        buffer.terminal = Some(event.clone());
        let _ = running.events.send(event);
    }

    async fn finish_interrupted(&self, id: &str, running: &Running) {
        let content = running
            .buffer
            .read()
            .expect("generation buffer poisoned")
            .content
            .clone();
        let _ = sqlx::query("UPDATE messages SET content = ?, status = 'interrupted', error = 'Server is shutting down', updated_at = ? WHERE id = ?")
            .bind(content)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await;
        let event = StreamEvent::Done {
            status: "interrupted".into(),
            finish_reason: None,
            usage: None,
            cost: None,
        };
        let mut buffer = running.buffer.write().expect("generation buffer poisoned");
        buffer.terminal = Some(event.clone());
        let _ = running.events.send(event);
    }

    async fn mark_interrupted(&self, running: &Running) {
        let content = running
            .buffer
            .read()
            .expect("generation buffer poisoned")
            .content
            .clone();
        let _ = sqlx::query("UPDATE messages SET content = ?, status = 'interrupted', error = 'Server shutdown deadline expired', updated_at = ? WHERE id = ? AND status = 'streaming'")
            .bind(content)
            .bind(now_ms())
            .bind(&running.id)
            .execute(&self.pool)
            .await;
    }

    async fn finish_error(&self, id: &str, running: &Running, message: String) {
        let content = running
            .buffer
            .read()
            .expect("generation buffer poisoned")
            .content
            .clone();
        let _ = sqlx::query("UPDATE messages SET content = ?, status = 'error', error = ?, updated_at = ? WHERE id = ?")
            .bind(content).bind(&message).bind(now_ms()).bind(id).execute(&self.pool).await;
        let event = StreamEvent::Error {
            status: "error".into(),
            message,
        };
        let mut buffer = running.buffer.write().expect("generation buffer poisoned");
        buffer.terminal = Some(event.clone());
        let _ = running.events.send(event);
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

fn prompt_for_provider(instructions: Option<&str>, prompt: &[PromptMessage]) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(prompt.len() + usize::from(instructions.is_some()));
    if let Some(instructions) = instructions.filter(|value| !value.trim().is_empty()) {
        messages.push(ChatMessage {
            role: "system".into(),
            content: instructions.to_owned(),
        });
    }
    messages.extend(prompt.iter().map(|message| ChatMessage {
        role: message.role.clone(),
        content: message.content.clone(),
    }));
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

pub fn mark_interrupted_on_start(
    pool: &SqlitePool,
) -> impl std::future::Future<Output = Result<u64, sqlx::Error>> + '_ {
    Manager::recover_interrupted(pool)
}

pub async fn message_record(pool: &SqlitePool, id: &str) -> Result<MessageRecord, GenerationError> {
    sqlx::query_as::<_, MessageRecord>("SELECT id, chat_id, parent_id, role, content, status, error, model, generation_id, finish_reason, prompt_tokens, completion_tokens, reasoning_tokens, cost, created_at, updated_at FROM messages WHERE id = ?")
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
    use super::{GenerationError, StreamEvent};
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
        let row = sqlx::query_as::<_, (String, String, Option<f64>)>(
            "SELECT content, status, cost FROM messages WHERE id = ?",
        )
        .bind(&prepared.assistant_message.id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(row.0, late_content);
        assert_eq!(row.1, "complete");
        assert_eq!(row.2, Some(0.0003));
        assert!(fake.requests_snapshot().iter().any(|request| {
            request.path == "/api/v1/chat/completions"
                && request.body["stream"] == true
                && request.body["reasoning"]["exclude"] == true
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
            backup_keep: 7,
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
                pdf_engine: None,
            },
        )
        .await
        .unwrap()
    }
}
