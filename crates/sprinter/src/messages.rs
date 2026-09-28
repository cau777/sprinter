use crate::{auth, chats, settings, state::AppState};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error as ThisError;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, FromRow, TS)]
pub struct MessageRecord {
    pub id: String,
    pub chat_id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub content: String,
    pub status: String,
    pub error: Option<String>,
    pub model: Option<String>,
    pub generation_id: Option<String>,
    pub finish_reason: Option<String>,
    #[ts(type = "number | null")]
    pub prompt_tokens: Option<i64>,
    #[ts(type = "number | null")]
    pub completion_tokens: Option<i64>,
    #[ts(type = "number | null")]
    pub reasoning_tokens: Option<i64>,
    pub cost: Option<f64>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[sqlx(skip)]
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, FromRow, TS)]
pub struct MessageAttachment {
    pub upload_id: String,
    #[ts(type = "number")]
    pub position: i64,
    pub filename: String,
    pub kind: String,
    pub mime: String,
    #[ts(type = "number")]
    pub size: i64,
    pub pdf_engine: Option<String>,
    pub parse_cache: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SendMessageRequest {
    pub parent_id: Option<String>,
    pub content: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    pub model: Option<String>,
    pub pdf_engine: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RegenerateRequest {
    pub model: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SwitchBranchRequest {
    pub message_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: String,
    pub attachment_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreparedGeneration {
    pub chat_id: String,
    /// The chat leaf before this generation moved it to the new assistant message.
    pub previous_leaf_id: Option<String>,
    /// True only when this generation created the chat itself.
    pub created_chat: bool,
    pub user_message: Option<MessageRecord>,
    pub assistant_message: MessageRecord,
    pub model: String,
    pub prompt: Vec<PromptMessage>,
    pub pdf_engine: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct SendMessageResponse {
    pub user_message: MessageRecord,
    pub assistant_message: MessageRecord,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct NewChatMessageResponse {
    pub chat: chats::ChatSummary,
    pub user_message: MessageRecord,
    pub assistant_message: MessageRecord,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct RegenerateMessageResponse {
    pub assistant_message: MessageRecord,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/chats/new/messages", post(send_new_handler))
        .route("/api/chats/{id}/messages", post(send_handler))
        .route("/api/messages/{id}/regenerate", post(regenerate_handler))
        .route_layer(middleware::from_fn_with_state(state, auth::require_session))
}

async fn send_handler(
    State(state): State<AppState>,
    Path(chat_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, MessageApiError> {
    let request_id = auth::request_id(&headers);
    if request
        .model
        .as_ref()
        .is_some_and(|model| model.trim().is_empty())
    {
        return Err(MessageApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_model",
            "Model cannot be empty",
        ));
    }
    let key = preflight_key(&state, &headers).await?;
    let prepared = send_to_chat(&state.pool, &chat_id, request)
        .await
        .map_err(|error| MessageApiError::from_message(error, &headers))?;
    match state
        .generation
        .start(&state, prepared.clone(), key, Some(request_id))
        .await
    {
        Ok(()) => Ok(Json(SendMessageResponse {
            user_message: prepared
                .user_message
                .expect("send exchanges include a user message"),
            assistant_message: prepared.assistant_message,
        })),
        Err(error) => {
            if let Err(rollback_error) = rollback_generation(&state.pool, &prepared).await {
                tracing::error!(error = %rollback_error, "could not roll back rejected generation");
            }
            Err(MessageApiError::from_generation(error, &headers))
        }
    }
}

async fn send_new_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<NewChatMessageResponse>, MessageApiError> {
    let request_id = auth::request_id(&headers);
    if request
        .model
        .as_ref()
        .is_some_and(|model| model.trim().is_empty())
    {
        return Err(MessageApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_model",
            "Model cannot be empty",
        ));
    }
    let key = preflight_key(&state, &headers).await?;
    let default_model = settings::default_model(&state)
        .await
        .map_err(|_| MessageApiError::internal(&headers))?;
    if request.model.is_none() && default_model.is_none() {
        return Err(MessageApiError::new(
            &headers,
            StatusCode::CONFLICT,
            "no_default_model",
            "Choose a default model in Settings",
        ));
    }
    let prepared = send_new_chat(&state.pool, default_model.as_deref(), request)
        .await
        .map_err(|error| MessageApiError::from_message(error, &headers))?;
    let chat = match chats::get_chat(&state.pool, &prepared.chat_id).await {
        Ok(Some(chat)) => chat,
        _ => {
            if let Err(rollback_error) = rollback_generation(&state.pool, &prepared).await {
                tracing::error!(error = %rollback_error, "could not roll back failed new chat lookup");
            }
            return Err(MessageApiError::internal(&headers));
        }
    };
    match state
        .generation
        .start(&state, prepared.clone(), key, Some(request_id))
        .await
    {
        Ok(()) => Ok(Json(NewChatMessageResponse {
            chat,
            user_message: prepared
                .user_message
                .expect("send exchanges include a user message"),
            assistant_message: prepared.assistant_message,
        })),
        Err(error) => {
            if let Err(rollback_error) = rollback_generation(&state.pool, &prepared).await {
                tracing::error!(error = %rollback_error, "could not roll back rejected generation");
            }
            Err(MessageApiError::from_generation(error, &headers))
        }
    }
}

async fn regenerate_handler(
    State(state): State<AppState>,
    Path(message_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<RegenerateRequest>,
) -> Result<Json<RegenerateMessageResponse>, MessageApiError> {
    let request_id = auth::request_id(&headers);
    if request
        .model
        .as_ref()
        .is_some_and(|model| model.trim().is_empty())
    {
        return Err(MessageApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_model",
            "Model cannot be empty",
        ));
    }
    let key = preflight_key(&state, &headers).await?;
    let prepared = regenerate(&state.pool, &message_id, request.model.as_deref())
        .await
        .map_err(|error| MessageApiError::from_message(error, &headers))?;
    match state
        .generation
        .start(&state, prepared.clone(), key, Some(request_id))
        .await
    {
        Ok(()) => Ok(Json(RegenerateMessageResponse {
            assistant_message: prepared.assistant_message,
        })),
        Err(error) => {
            if let Err(rollback_error) = rollback_generation(&state.pool, &prepared).await {
                tracing::error!(error = %rollback_error, "could not roll back rejected regeneration");
            }
            Err(MessageApiError::from_generation(error, &headers))
        }
    }
}

async fn preflight_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<secrecy::SecretString, MessageApiError> {
    match settings::provider_key(state).await {
        Ok(Some(key)) => Ok(key),
        Ok(None) | Err(_) => Err(MessageApiError::new(
            headers,
            StatusCode::CONFLICT,
            "no_api_key",
            "Add or replace the OpenRouter API key in Settings",
        )),
    }
}

struct MessageApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: String,
}

impl MessageApiError {
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

    fn internal(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred",
        )
    }

    fn from_message(error: MessageError, headers: &HeaderMap) -> Self {
        match error {
            MessageError::NotFound => Self::new(
                headers,
                StatusCode::NOT_FOUND,
                "not_found",
                "Chat or message not found",
            ),
            MessageError::InvalidParent | MessageError::InvalidMessage => Self::new(
                headers,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "The message or parent is invalid",
            ),
            MessageError::EmptyContent => Self::new(
                headers,
                StatusCode::BAD_REQUEST,
                "empty_content",
                "Message content cannot be empty",
            ),
            MessageError::ContentTooLarge => Self::new(
                headers,
                StatusCode::PAYLOAD_TOO_LARGE,
                "message_too_large",
                "Message content exceeds 256 KB",
            ),
            MessageError::InvalidAttachments => Self::new(
                headers,
                StatusCode::BAD_REQUEST,
                "invalid_attachments",
                "One or more attachments are invalid",
            ),
            MessageError::TooManyAttachments => Self::new(
                headers,
                StatusCode::BAD_REQUEST,
                "too_many_attachments",
                "This message exceeds the file count limit",
            ),
            MessageError::PromptTooLarge => Self::new(
                headers,
                StatusCode::PAYLOAD_TOO_LARGE,
                "attachments_too_large",
                "Attachments in this prompt exceed the configured limit",
            ),
            MessageError::InvalidPdfEngine => Self::new(
                headers,
                StatusCode::BAD_REQUEST,
                "invalid_pdf_engine",
                "The PDF parser engine is invalid",
            ),
            MessageError::GenerationInProgress => Self::new(
                headers,
                StatusCode::CONFLICT,
                "generation_in_progress",
                "A generation is already active in this chat",
            ),
            MessageError::NoDefaultModel => Self::new(
                headers,
                StatusCode::CONFLICT,
                "no_default_model",
                "Choose a default model in Settings",
            ),
            MessageError::Database(_) => Self::internal(headers),
        }
    }

    fn from_generation(error: crate::generation::GenerationError, headers: &HeaderMap) -> Self {
        match error {
            crate::generation::GenerationError::GenerationInProgress => Self::new(
                headers,
                StatusCode::CONFLICT,
                "generation_in_progress",
                "A generation is already active in this chat",
            ),
            crate::generation::GenerationError::TooManyGenerations => Self::new(
                headers,
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_generations",
                "The server is handling its generation limit",
            ),
            crate::generation::GenerationError::NotFound => Self::new(
                headers,
                StatusCode::NOT_FOUND,
                "not_found",
                "Message not found",
            ),
            crate::generation::GenerationError::Internal => Self::internal(headers),
        }
    }
}

impl IntoResponse for MessageApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({"error":{"code":self.code,"message":self.message,"request_id":self.request_id}})),
        )
            .into_response()
    }
}

#[derive(Debug, ThisError)]
pub enum MessageError {
    #[error("chat or message not found")]
    NotFound,
    #[error("message parent is not in this chat")]
    InvalidParent,
    #[error("message is not an assistant message")]
    InvalidMessage,
    #[error("message content cannot be empty")]
    EmptyContent,
    #[error("message content exceeds 256 KB")]
    ContentTooLarge,
    #[error("one or more attachments are invalid")]
    InvalidAttachments,
    #[error("message has too many attachments")]
    TooManyAttachments,
    #[error("attachments exceed the prompt size limit")]
    PromptTooLarge,
    #[error("PDF parser engine is invalid")]
    InvalidPdfEngine,
    #[error("a generation is already active in this chat")]
    GenerationInProgress,
    #[error("default model is not configured")]
    NoDefaultModel,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub async fn get_chat_messages(
    pool: &SqlitePool,
    chat_id: &str,
) -> Result<Vec<MessageRecord>, MessageError> {
    let mut messages = sqlx::query_as::<_, MessageRecord>(
        "SELECT id, chat_id, parent_id, role, content, status, error, model, generation_id, finish_reason, \
                prompt_tokens, completion_tokens, reasoning_tokens, cost, created_at, updated_at \
         FROM messages WHERE chat_id = ? ORDER BY created_at, id",
    ).bind(chat_id).fetch_all(pool).await?;
    let attachments = sqlx::query_as::<_, (String, String, i64, String, String, String, i64, Option<String>, Option<String>)>(
        "SELECT ma.message_id, ma.upload_id, ma.position, u.filename, u.kind, u.mime, u.size, ma.pdf_engine, ma.parse_cache \
         FROM message_attachments ma JOIN messages m ON m.id = ma.message_id JOIN uploads u ON u.id = ma.upload_id \
         WHERE m.chat_id = ? ORDER BY ma.message_id, ma.position",
    )
    .bind(chat_id)
    .fetch_all(pool)
    .await?;
    let mut by_message: HashMap<String, Vec<MessageAttachment>> = HashMap::new();
    for (message_id, upload_id, position, filename, kind, mime, size, pdf_engine, parse_cache) in
        attachments
    {
        by_message
            .entry(message_id)
            .or_default()
            .push(MessageAttachment {
                upload_id,
                position,
                filename,
                kind,
                mime,
                size,
                pdf_engine,
                parse_cache,
            });
    }
    for message in &mut messages {
        message.attachments = by_message.remove(&message.id).unwrap_or_default();
    }
    Ok(messages)
}

pub async fn send_to_chat(
    pool: &SqlitePool,
    chat_id: &str,
    request: SendMessageRequest,
) -> Result<PreparedGeneration, MessageError> {
    validate_message_content(&request.content)?;
    let mut tx = pool.begin().await?;
    acquire_chat_write_lock(&mut tx, chat_id).await?;
    let chat_model = sqlx::query_scalar::<_, String>("SELECT model FROM chats WHERE id = ?")
        .bind(chat_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(MessageError::NotFound)?;
    ensure_idle(&mut tx, chat_id).await?;
    validate_parent(&mut tx, chat_id, request.parent_id.as_deref()).await?;
    let model = request.model.clone().unwrap_or(chat_model);
    let generation = insert_exchange(&mut tx, chat_id, request, model).await?;
    tx.commit().await?;
    Ok(generation)
}

pub async fn send_new_chat(
    pool: &SqlitePool,
    default_model: Option<&str>,
    request: SendMessageRequest,
) -> Result<PreparedGeneration, MessageError> {
    validate_message_content(&request.content)?;
    if request.parent_id.is_some() {
        return Err(MessageError::InvalidParent);
    }
    let model = request
        .model
        .clone()
        .or_else(|| default_model.map(str::to_owned))
        .ok_or(MessageError::NoDefaultModel)?;
    let id = Uuid::now_v7().to_string();
    let now = now_ms();
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO chats(id, model, created_at, updated_at) VALUES(?, ?, ?, ?)")
        .bind(&id)
        .bind(&model)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    let mut generation = insert_exchange(&mut tx, &id, request, model).await?;
    generation.created_chat = true;
    tx.commit().await?;
    Ok(generation)
}

pub async fn regenerate(
    pool: &SqlitePool,
    original_assistant_id: &str,
    override_model: Option<&str>,
) -> Result<PreparedGeneration, MessageError> {
    let chat_id = sqlx::query_scalar::<_, String>("SELECT chat_id FROM messages WHERE id = ?")
        .bind(original_assistant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(MessageError::NotFound)?;
    let mut tx = pool.begin().await?;
    acquire_chat_write_lock(&mut tx, &chat_id).await?;
    let original = sqlx::query_as::<_, (String, String, Option<String>, String)>(
        "SELECT id, chat_id, parent_id, role FROM messages WHERE id = ?",
    )
    .bind(original_assistant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(MessageError::NotFound)?;
    if original.3 != "assistant" {
        return Err(MessageError::InvalidMessage);
    }
    ensure_idle(&mut tx, &original.1).await?;
    validate_parent(&mut tx, &original.1, original.2.as_deref()).await?;
    let chat_model = sqlx::query_scalar::<_, String>("SELECT model FROM chats WHERE id = ?")
        .bind(&original.1)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(MessageError::NotFound)?;
    let model = override_model.unwrap_or(&chat_model).to_owned();
    let previous_leaf_id = current_leaf(&mut tx, &original.1).await?;
    let parent_id = original.2;
    let assistant_id = Uuid::now_v7().to_string();
    let now = now_ms();
    let prompt = prompt_path(&mut tx, &original.1, parent_id.as_deref()).await?;
    sqlx::query(
        "INSERT INTO messages(id, chat_id, parent_id, role, content, status, model, created_at, updated_at) \
         VALUES(?, ?, ?, 'assistant', '', 'streaming', ?, ?, ?)",
    ).bind(&assistant_id).bind(&original.1).bind(&parent_id).bind(&model).bind(now).bind(now)
        .execute(&mut *tx).await?;
    sqlx::query("UPDATE chats SET current_leaf_id = ?, updated_at = ? WHERE id = ?")
        .bind(&assistant_id)
        .bind(now)
        .bind(&original.1)
        .execute(&mut *tx)
        .await?;
    let assistant = get_message_tx(&mut tx, &assistant_id)
        .await?
        .ok_or(MessageError::NotFound)?;
    tx.commit().await?;
    Ok(PreparedGeneration {
        chat_id: original.1,
        previous_leaf_id,
        created_chat: false,
        user_message: None,
        assistant_message: assistant,
        model,
        prompt,
        pdf_engine: None,
    })
}

pub async fn switch_branch(
    pool: &SqlitePool,
    chat_id: &str,
    message_id: &str,
) -> Result<Option<String>, MessageError> {
    let mut tx = pool.begin().await?;
    let belongs = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ? AND chat_id = ?)",
    )
    .bind(message_id)
    .bind(chat_id)
    .fetch_one(&mut *tx)
    .await?;
    if belongs == 0 {
        return Err(MessageError::NotFound);
    }
    let leaf = sqlx::query_scalar::<_, String>(
        "WITH RECURSIVE descendants(id, created_at) AS ( \
             SELECT id, created_at FROM messages WHERE id = ? AND chat_id = ? \
             UNION ALL \
             SELECT child.id, child.created_at FROM messages child JOIN descendants parent ON child.parent_id = parent.id \
             WHERE child.chat_id = ? \
         ) \
         SELECT d.id FROM descendants d WHERE NOT EXISTS (SELECT 1 FROM messages child WHERE child.parent_id = d.id) \
         ORDER BY d.created_at DESC, d.id DESC LIMIT 1",
    ).bind(message_id).bind(chat_id).bind(chat_id).fetch_optional(&mut *tx).await?;
    sqlx::query("UPDATE chats SET current_leaf_id = ?, updated_at = ? WHERE id = ?")
        .bind(&leaf)
        .bind(now_ms())
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(leaf)
}

async fn insert_exchange(
    tx: &mut Transaction<'_, Sqlite>,
    chat_id: &str,
    request: SendMessageRequest,
    model: String,
) -> Result<PreparedGeneration, MessageError> {
    if request
        .pdf_engine
        .as_deref()
        .is_some_and(|engine| !matches!(engine, "cloudflare-ai" | "mistral-ocr" | "native"))
    {
        return Err(MessageError::InvalidPdfEngine);
    }
    let now = now_ms();
    let user_id = Uuid::now_v7().to_string();
    let assistant_id = Uuid::now_v7().to_string();
    let previous_leaf_id = current_leaf(tx, chat_id).await?;
    let prompt = prompt_path(tx, chat_id, request.parent_id.as_deref()).await?;
    let attachment_kinds = validate_attachments(tx, &prompt, &request.attachment_ids).await?;
    let stored_pdf_engine = request
        .pdf_engine
        .clone()
        .or(read_default_pdf_engine(tx).await?);
    sqlx::query(
        "INSERT INTO messages(id, chat_id, parent_id, role, content, status, created_at, updated_at) \
         VALUES(?, ?, ?, 'user', ?, 'complete', ?, ?)",
    ).bind(&user_id).bind(chat_id).bind(&request.parent_id).bind(&request.content).bind(now).bind(now)
        .execute(&mut **tx).await?;
    for (position, upload_id) in request.attachment_ids.iter().enumerate() {
        sqlx::query("INSERT INTO message_attachments(message_id, upload_id, position, pdf_engine) VALUES(?, ?, ?, ?)")
            .bind(&user_id).bind(upload_id).bind(position as i64)
            .bind(if attachment_kinds.get(upload_id).map(String::as_str) == Some("pdf") { stored_pdf_engine.as_deref() } else { None })
            .execute(&mut **tx).await?;
    }
    sqlx::query(
        "INSERT INTO messages(id, chat_id, parent_id, role, content, status, model, created_at, updated_at) \
         VALUES(?, ?, ?, 'assistant', '', 'streaming', ?, ?, ?)",
    ).bind(&assistant_id).bind(chat_id).bind(&user_id).bind(&model).bind(now).bind(now)
        .execute(&mut **tx).await?;
    sqlx::query("UPDATE chats SET current_leaf_id = ?, updated_at = ? WHERE id = ?")
        .bind(&assistant_id)
        .bind(now)
        .bind(chat_id)
        .execute(&mut **tx)
        .await?;

    let user_message = get_message_tx(tx, &user_id)
        .await?
        .ok_or(MessageError::NotFound)?;
    let assistant_message = get_message_tx(tx, &assistant_id)
        .await?
        .ok_or(MessageError::NotFound)?;
    let mut prompt = prompt;
    prompt.push(PromptMessage {
        role: "user".to_owned(),
        content: request.content,
        attachment_ids: request.attachment_ids,
    });
    Ok(PreparedGeneration {
        chat_id: chat_id.to_owned(),
        previous_leaf_id,
        created_chat: false,
        user_message: Some(user_message),
        assistant_message,
        model,
        prompt,
        pdf_engine: request.pdf_engine,
    })
}

fn validate_message_content(content: &str) -> Result<(), MessageError> {
    if content.trim().is_empty() {
        return Err(MessageError::EmptyContent);
    }
    if content.len() > 256 * 1024 {
        return Err(MessageError::ContentTooLarge);
    }
    Ok(())
}

async fn validate_attachments(
    tx: &mut Transaction<'_, Sqlite>,
    prompt: &[PromptMessage],
    new_ids: &[String],
) -> Result<HashMap<String, String>, MessageError> {
    if new_ids.len() > 20 {
        return Err(MessageError::TooManyAttachments);
    }
    let mut kinds = HashMap::new();
    let mut total = 0_u64;
    for id in prompt
        .iter()
        .flat_map(|message| message.attachment_ids.iter())
        .chain(new_ids)
    {
        let (kind, size) =
            sqlx::query_as::<_, (String, i64)>("SELECT kind, size FROM uploads WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(MessageError::InvalidAttachments)?;
        total = total.saturating_add(size.max(0) as u64);
        if new_ids.contains(id) {
            kinds.insert(id.clone(), kind);
        }
    }
    let raw_limits: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'upload_limits'")
            .fetch_optional(&mut **tx)
            .await?;
    let limits = raw_limits
        .and_then(|value| serde_json::from_str::<settings::UploadLimits>(&value).ok())
        .unwrap_or_default();
    if new_ids.len() as u64 > limits.files_per_message.min(20) {
        return Err(MessageError::TooManyAttachments);
    }
    if total > limits.total_prompt_bytes.min(200 * 1024 * 1024) {
        return Err(MessageError::PromptTooLarge);
    }
    if kinds.len() != new_ids.len() {
        return Err(MessageError::InvalidAttachments);
    }
    Ok(kinds)
}

async fn read_default_pdf_engine(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Option<String>, MessageError> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'pdf_engine'")
            .fetch_optional(&mut **tx)
            .await?;
    Ok(raw.and_then(|value| serde_json::from_str(&value).ok()))
}

async fn current_leaf(
    tx: &mut Transaction<'_, Sqlite>,
    chat_id: &str,
) -> Result<Option<String>, MessageError> {
    Ok(
        sqlx::query_scalar::<_, Option<String>>("SELECT current_leaf_id FROM chats WHERE id = ?")
            .bind(chat_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten(),
    )
}

/// Removes the rows created for a generation that could not be handed to the
/// generation manager and restores the chat's previous selected leaf.
pub async fn rollback_generation(
    pool: &SqlitePool,
    generation: &PreparedGeneration,
) -> Result<(), MessageError> {
    let mut tx = pool.begin().await?;
    if generation.created_chat {
        sqlx::query("DELETE FROM chats WHERE id = ?")
            .bind(&generation.chat_id)
            .execute(&mut *tx)
            .await?;
    } else {
        let assistant_id = &generation.assistant_message.id;
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(assistant_id)
            .execute(&mut *tx)
            .await?;
        if let Some(user) = &generation.user_message {
            sqlx::query("DELETE FROM messages WHERE id = ?")
                .bind(&user.id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE chats SET current_leaf_id = ?, updated_at = ? WHERE id = ?")
            .bind(&generation.previous_leaf_id)
            .bind(now_ms())
            .bind(&generation.chat_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn validate_parent(
    tx: &mut Transaction<'_, Sqlite>,
    chat_id: &str,
    parent_id: Option<&str>,
) -> Result<(), MessageError> {
    if let Some(parent_id) = parent_id {
        let belongs = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ? AND chat_id = ?)",
        )
        .bind(parent_id)
        .bind(chat_id)
        .fetch_one(&mut **tx)
        .await?;
        if belongs == 0 {
            return Err(MessageError::InvalidParent);
        }
    }
    Ok(())
}

async fn ensure_idle(tx: &mut Transaction<'_, Sqlite>, chat_id: &str) -> Result<(), MessageError> {
    let active = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE chat_id = ? AND status = 'streaming')",
    )
    .bind(chat_id)
    .fetch_one(&mut **tx)
    .await?;
    if active != 0 {
        return Err(MessageError::GenerationInProgress);
    }
    Ok(())
}

async fn acquire_chat_write_lock(
    tx: &mut Transaction<'_, Sqlite>,
    chat_id: &str,
) -> Result<(), MessageError> {
    let result = sqlx::query("UPDATE chats SET updated_at = updated_at WHERE id = ?")
        .bind(chat_id)
        .execute(&mut **tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(MessageError::NotFound);
    }
    Ok(())
}

async fn prompt_path(
    tx: &mut Transaction<'_, Sqlite>,
    chat_id: &str,
    leaf_id: Option<&str>,
) -> Result<Vec<PromptMessage>, MessageError> {
    let Some(leaf_id) = leaf_id else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query_as::<_, (String, String, String, String, i64)>(
        "WITH RECURSIVE path(id, parent_id, role, content, depth) AS ( \
             SELECT id, parent_id, role, content, 0 FROM messages WHERE id = ? AND chat_id = ? \
             UNION ALL \
             SELECT parent.id, parent.parent_id, parent.role, parent.content, path.depth + 1 \
             FROM messages parent JOIN path ON parent.id = path.parent_id WHERE parent.chat_id = ? \
         ) \
         SELECT id, role, content, parent_id, depth FROM path ORDER BY depth DESC",
    )
    .bind(leaf_id)
    .bind(chat_id)
    .bind(chat_id)
    .fetch_all(&mut **tx)
    .await?;
    if rows.is_empty() {
        return Err(MessageError::InvalidParent);
    }
    let mut output = Vec::with_capacity(rows.len());
    for (id, role, content, _, _) in rows {
        let attachment_ids = sqlx::query_scalar::<_, String>(
            "SELECT upload_id FROM message_attachments WHERE message_id = ? ORDER BY position",
        )
        .bind(id)
        .fetch_all(&mut **tx)
        .await?;
        output.push(PromptMessage {
            role,
            content,
            attachment_ids,
        });
    }
    Ok(output)
}

async fn get_message_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<Option<MessageRecord>, MessageError> {
    let mut message = sqlx::query_as::<_, MessageRecord>(
        "SELECT id, chat_id, parent_id, role, content, status, error, model, generation_id, finish_reason, \
                prompt_tokens, completion_tokens, reasoning_tokens, cost, created_at, updated_at \
         FROM messages WHERE id = ?",
    ).bind(id).fetch_optional(&mut **tx).await?;
    if let Some(message) = message.as_mut() {
        message.attachments = sqlx::query_as::<_, MessageAttachment>(
            "SELECT ma.upload_id, ma.position, u.filename, u.kind, u.mime, u.size, ma.pdf_engine, ma.parse_cache \
             FROM message_attachments ma JOIN uploads u ON u.id = ma.upload_id \
             WHERE ma.message_id = ? ORDER BY ma.position",
        ).bind(id).fetch_all(&mut **tx).await?;
    }
    Ok(message)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::{SendMessageRequest, regenerate, send_new_chat, send_to_chat, switch_branch};
    use crate::db;
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

    #[tokio::test]
    async fn edits_regeneration_and_switch_preserve_the_message_tree_and_prompt_path() {
        let pool = test_pool().await;
        let first = send_new_chat(&pool, Some("test/chat"), request(None, "original root"))
            .await
            .unwrap();
        assert_eq!(prompt_pairs(&first.prompt), [("user", "original root")]);
        mark_complete(&pool, &first.assistant_message.id).await;

        let followup = send_to_chat(
            &pool,
            &first.chat_id,
            request(Some(&first.assistant_message.id), "follow up"),
        )
        .await
        .unwrap();
        assert_eq!(
            prompt_pairs(&followup.prompt),
            [
                ("user", "original root"),
                ("assistant", ""),
                ("user", "follow up"),
            ]
        );
        mark_complete(&pool, &followup.assistant_message.id).await;

        let retried = regenerate(&pool, &first.assistant_message.id, Some("test/override"))
            .await
            .unwrap();
        assert_eq!(retried.model, "test/override");
        assert_eq!(
            retried.assistant_message.model.as_deref(),
            Some("test/override")
        );
        assert_eq!(prompt_pairs(&retried.prompt), [("user", "original root")]);
        mark_complete(&pool, &retried.assistant_message.id).await;

        let edited = send_to_chat(&pool, &first.chat_id, request(None, "edited root"))
            .await
            .unwrap();
        assert_eq!(prompt_pairs(&edited.prompt), [("user", "edited root")]);
        mark_complete(&pool, &edited.assistant_message.id).await;

        // Force a stable ordering even when the test runs within one millisecond.
        sqlx::query("UPDATE messages SET created_at = 100 WHERE id = ?")
            .bind(&followup.assistant_message.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET created_at = 200 WHERE id = ?")
            .bind(&retried.assistant_message.id)
            .execute(&pool)
            .await
            .unwrap();
        let leaf = switch_branch(
            &pool,
            &first.chat_id,
            &first.user_message.as_ref().unwrap().id,
        )
        .await
        .unwrap();
        assert_eq!(leaf.as_deref(), Some(retried.assistant_message.id.as_str()));
        let current_leaf = sqlx::query_scalar::<_, Option<String>>(
            "SELECT current_leaf_id FROM chats WHERE id = ?",
        )
        .bind(&first.chat_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(current_leaf.as_deref(), leaf.as_deref());

        let message_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE chat_id = ?")
                .bind(&first.chat_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(message_count, 7);
        pool.close().await;
    }

    #[tokio::test]
    async fn send_validates_attachment_limits_and_records_the_effective_pdf_engine() {
        let pool = test_pool().await;
        let limits = serde_json::json!({
            "image_bytes": 1024, "pdf_bytes": 1024, "text_bytes": 1024,
            "files_per_message": 1, "total_prompt_bytes": 2048
        });
        sqlx::query("INSERT INTO settings(key, value, updated_at) VALUES('upload_limits', ?, 1)")
            .bind(serde_json::to_string(&limits).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO settings(key, value, updated_at) VALUES('pdf_engine', ?, 1)")
            .bind(serde_json::to_string("mistral-ocr").unwrap())
            .execute(&pool)
            .await
            .unwrap();
        for (id, filename) in [("upload-one", "one.pdf"), ("upload-two", "two.pdf")] {
            sqlx::query("INSERT INTO uploads(id, sha256, filename, mime, kind, size, created_at) VALUES(?, ?, ?, 'application/pdf', 'pdf', 100, 1)")
                .bind(id).bind(id.repeat(32)).bind(filename).execute(&pool).await.unwrap();
        }
        let mut invalid = request(None, "hi");
        invalid.attachment_ids.push("missing-upload".into());
        assert!(matches!(
            send_new_chat(&pool, Some("test/chat"), invalid).await,
            Err(super::MessageError::InvalidAttachments)
        ));

        let mut too_many = request(None, "hi");
        too_many.attachment_ids = vec!["upload-one".into(), "upload-two".into()];
        assert!(matches!(
            send_new_chat(&pool, Some("test/chat"), too_many).await,
            Err(super::MessageError::TooManyAttachments)
        ));

        let mut valid = request(None, "hi");
        valid.attachment_ids.push("upload-one".into());
        let prepared = send_new_chat(&pool, Some("test/chat"), valid)
            .await
            .unwrap();
        assert_eq!(prepared.prompt[0].attachment_ids, ["upload-one"]);
        let engine: Option<String> = sqlx::query_scalar(
            "SELECT pdf_engine FROM message_attachments WHERE upload_id = 'upload-one'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(engine.as_deref(), Some("mistral-ocr"));

        let oversized = request(None, &"x".repeat(256 * 1024 + 1));
        assert!(matches!(
            send_new_chat(&pool, Some("test/chat"), oversized).await,
            Err(super::MessageError::ContentTooLarge)
        ));
        pool.close().await;
    }

    fn request(parent_id: Option<&str>, content: &str) -> SendMessageRequest {
        SendMessageRequest {
            parent_id: parent_id.map(str::to_owned),
            content: content.to_owned(),
            attachment_ids: Vec::new(),
            model: None,
            pdf_engine: None,
        }
    }

    fn prompt_pairs(prompt: &[super::PromptMessage]) -> Vec<(&str, &str)> {
        prompt
            .iter()
            .map(|message| (message.role.as_str(), message.content.as_str()))
            .collect()
    }

    async fn mark_complete(pool: &SqlitePool, id: &str) {
        sqlx::query("UPDATE messages SET status = 'complete' WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn test_pool() -> SqlitePool {
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
        pool
    }
}
