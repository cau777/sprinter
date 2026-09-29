use crate::{auth, settings, state::AppState};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error as ThisError;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow, TS)]
pub struct ChatSummary {
    pub id: String,
    pub title: Option<String>,
    pub model: String,
    #[ts(type = "number")]
    pub updated_at: i64,
}

pub fn new_session_id() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ChatPage {
    pub items: Vec<ChatSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ChatDetail {
    pub id: String,
    pub title: Option<String>,
    pub title_source: String,
    pub model: String,
    pub current_leaf_id: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub messages: Vec<crate::messages::MessageRecord>,
}

#[derive(Debug, ThisError)]
pub enum ChatError {
    #[error("chat not found")]
    NotFound,
    #[error("invalid cursor")]
    InvalidCursor,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Message(#[from] crate::messages::MessageError),
}

#[derive(Deserialize)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct CreateChatRequest {
    model: Option<String>,
}

#[derive(Deserialize)]
struct UpdateChatRequest {
    title: Option<String>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct SwitchBranchRequest {
    message_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SwitchBranchResponse {
    pub current_leaf_id: String,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/chats", get(list_handler).post(create_handler))
        .route(
            "/api/chats/{id}",
            get(detail_handler)
                .patch(update_handler)
                .delete(delete_handler),
        )
        .route(
            "/api/chats/{id}/switch",
            axum::routing::post(switch_handler),
        )
        .route_layer(middleware::from_fn_with_state(state, auth::require_session))
}

async fn switch_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SwitchBranchRequest>,
) -> Result<Json<SwitchBranchResponse>, ChatApiError> {
    if request.message_id.is_empty() {
        return Err(ChatApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Choose a message branch",
        ));
    }
    let leaf = crate::messages::switch_branch(&state.pool, &id, &request.message_id)
        .await
        .map_err(|error| ChatApiError::from_error(ChatError::Message(error), &headers))?
        .ok_or_else(|| ChatApiError::not_found(&headers))?;
    Ok(Json(SwitchBranchResponse {
        current_leaf_id: leaf,
    }))
}

async fn list_handler(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
    headers: HeaderMap,
) -> Result<Json<ChatPage>, ChatApiError> {
    Ok(Json(
        list_chats(
            &state.pool,
            query.cursor.as_deref(),
            query.limit.unwrap_or(50),
        )
        .await
        .map_err(|e| ChatApiError::from_error(e, &headers))?,
    ))
}

async fn detail_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ChatDetail>, ChatApiError> {
    get_chat_detail(&state.pool, &id)
        .await
        .map_err(|e| ChatApiError::from_error(e, &headers))?
        .map(Json)
        .ok_or_else(|| ChatApiError::not_found(&headers))
}

async fn create_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateChatRequest>,
) -> Result<(StatusCode, Json<ChatSummary>), ChatApiError> {
    let model = match request.model {
        Some(model) if !model.trim().is_empty() => model,
        _ => settings::default_model(&state)
            .await
            .map_err(|_| ChatApiError::internal(&headers))?
            .ok_or_else(|| {
                ChatApiError::new(
                    &headers,
                    StatusCode::CONFLICT,
                    "no_default_model",
                    "Choose a default model in Settings",
                )
            })?,
    };
    Ok((
        StatusCode::CREATED,
        Json(
            create_chat(&state.pool, &model)
                .await
                .map_err(|e| ChatApiError::from_error(e, &headers))?,
        ),
    ))
}

async fn update_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UpdateChatRequest>,
) -> Result<Json<ChatSummary>, ChatApiError> {
    if request.title.is_none() && request.model.is_none() {
        return Err(ChatApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Provide a title or model to update",
        ));
    }
    Ok(Json(
        rename_chat(
            &state.pool,
            &id,
            request.title.as_deref(),
            request.model.as_deref(),
        )
        .await
        .map_err(|e| ChatApiError::from_error(e, &headers))?,
    ))
}

async fn delete_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ChatApiError> {
    match state.generation.cancel_chat(&id).await {
        Ok(()) | Err(crate::generation::GenerationError::NotFound) => {}
        Err(_) => return Err(ChatApiError::internal(&headers)),
    }
    delete_chat(&state.pool, &id)
        .await
        .map_err(|e| ChatApiError::from_error(e, &headers))?;
    Ok(StatusCode::NO_CONTENT)
}

struct ChatApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: String,
}
impl ChatApiError {
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
    fn not_found(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::NOT_FOUND,
            "not_found",
            "Chat not found",
        )
    }
    fn from_error(error: ChatError, headers: &HeaderMap) -> Self {
        match error {
            ChatError::NotFound => Self::not_found(headers),
            ChatError::Message(crate::messages::MessageError::NotFound) => Self::not_found(headers),
            ChatError::InvalidCursor => Self::new(
                headers,
                StatusCode::BAD_REQUEST,
                "invalid_cursor",
                "Cursor is invalid",
            ),
            _ => Self::internal(headers),
        }
    }
}
impl IntoResponse for ChatApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({"error":{"code":self.code,"message":self.message,"request_id":self.request_id}}))).into_response()
    }
}

pub async fn list_chats(
    pool: &SqlitePool,
    cursor: Option<&str>,
    limit: usize,
) -> Result<ChatPage, ChatError> {
    let cursor = cursor.map(decode_cursor).transpose()?;
    let limit = limit.clamp(1, 100);
    let fetch_limit = (limit + 1) as i64;
    let (cursor_time, cursor_id) = cursor.map_or((None, None), |(time, id)| (Some(time), Some(id)));
    let mut items = sqlx::query_as::<_, ChatSummary>(
        "SELECT id, title, model, updated_at FROM chats \
         WHERE (? IS NULL OR updated_at < ? OR (updated_at = ? AND id < ?)) \
         ORDER BY updated_at DESC, id DESC LIMIT ?",
    )
    .bind(cursor_time)
    .bind(cursor_time)
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = has_more.then(|| {
        let last = items
            .last()
            .expect("a page with a further row is non-empty");
        encode_cursor(last.updated_at, &last.id)
    });
    Ok(ChatPage { items, next_cursor })
}

pub async fn get_chat(pool: &SqlitePool, id: &str) -> Result<Option<ChatSummary>, ChatError> {
    Ok(sqlx::query_as::<_, ChatSummary>(
        "SELECT id, title, model, updated_at FROM chats WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn get_chat_detail(pool: &SqlitePool, id: &str) -> Result<Option<ChatDetail>, ChatError> {
    let row = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            i64,
            i64,
        ),
    >(
        "SELECT id, title, title_source, model, current_leaf_id, created_at, updated_at \
         FROM chats WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some((id, title, title_source, model, current_leaf_id, created_at, updated_at)) = row
    else {
        return Ok(None);
    };
    let messages = crate::messages::get_chat_messages(pool, &id).await?;
    Ok(Some(ChatDetail {
        id,
        title,
        title_source,
        model,
        current_leaf_id,
        created_at,
        updated_at,
        messages,
    }))
}

pub async fn create_chat(pool: &SqlitePool, model: &str) -> Result<ChatSummary, ChatError> {
    let id = Uuid::now_v7().to_string();
    let session_id = new_session_id();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO chats(id, session_id, model, created_at, updated_at) VALUES(?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(model)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    get_chat(pool, &id).await?.ok_or(ChatError::NotFound)
}

pub async fn rename_chat(
    pool: &SqlitePool,
    id: &str,
    title: Option<&str>,
    model: Option<&str>,
) -> Result<ChatSummary, ChatError> {
    let result = sqlx::query(
        "UPDATE chats SET title = COALESCE(?, title), \
             title_source = CASE WHEN ? IS NULL THEN title_source ELSE 'manual' END, \
             model = COALESCE(?, model), updated_at = ? WHERE id = ?",
    )
    .bind(title)
    .bind(title)
    .bind(model)
    .bind(now_ms())
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ChatError::NotFound);
    }
    get_chat(pool, id).await?.ok_or(ChatError::NotFound)
}

pub async fn delete_chat(pool: &SqlitePool, id: &str) -> Result<u64, ChatError> {
    let mut tx = pool.begin().await?;
    let exists = sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM chats WHERE id = ?)")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if exists == 0 {
        return Err(ChatError::NotFound);
    }
    let messages = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE chat_id = ?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO usage_rollup(day, model, prompt_tokens, completion_tokens, cost) \
         SELECT strftime('%Y-%m-%d', created_at / 1000, 'unixepoch'), model, \
                COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0), COALESCE(SUM(cost), 0) \
         FROM messages WHERE chat_id = ? AND model IS NOT NULL AND 1 \
         GROUP BY strftime('%Y-%m-%d', created_at / 1000, 'unixepoch'), model \
         ON CONFLICT(day, model) DO UPDATE SET \
            prompt_tokens = usage_rollup.prompt_tokens + excluded.prompt_tokens, \
            completion_tokens = usage_rollup.completion_tokens + excluded.completion_tokens, \
            cost = usage_rollup.cost + excluded.cost",
    ).bind(id).execute(&mut *tx).await?;
    let deleted = sqlx::query("DELETE FROM chats WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(messages.max(deleted.rows_affected() as i64) as u64)
}

fn encode_cursor(updated_at: i64, id: &str) -> String {
    format!("{updated_at}:{id}")
}

fn decode_cursor(cursor: &str) -> Result<(i64, String), ChatError> {
    let (timestamp, id) = cursor.split_once(':').ok_or(ChatError::InvalidCursor)?;
    let timestamp = timestamp.parse().map_err(|_| ChatError::InvalidCursor)?;
    if id.is_empty() {
        return Err(ChatError::InvalidCursor);
    }
    Ok((timestamp, id.to_owned()))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
