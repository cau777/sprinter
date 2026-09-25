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
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
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
    let attachments = sqlx::query_as::<_, (String, String, i64, Option<String>, Option<String>)>(
        "SELECT ma.message_id, ma.upload_id, ma.position, ma.pdf_engine, ma.parse_cache \
         FROM message_attachments ma JOIN messages m ON m.id = ma.message_id \
         WHERE m.chat_id = ? ORDER BY ma.message_id, ma.position",
    )
    .bind(chat_id)
    .fetch_all(pool)
    .await?;
    let mut by_message: HashMap<String, Vec<MessageAttachment>> = HashMap::new();
    for (message_id, upload_id, position, pdf_engine, parse_cache) in attachments {
        by_message
            .entry(message_id)
            .or_default()
            .push(MessageAttachment {
                upload_id,
                position,
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
    if request.content.trim().is_empty() {
        return Err(MessageError::EmptyContent);
    }
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
    if request.content.trim().is_empty() {
        return Err(MessageError::EmptyContent);
    }
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
    let now = now_ms();
    let user_id = Uuid::now_v7().to_string();
    let assistant_id = Uuid::now_v7().to_string();
    let previous_leaf_id = current_leaf(tx, chat_id).await?;
    let prompt = prompt_path(tx, chat_id, request.parent_id.as_deref()).await?;
    sqlx::query(
        "INSERT INTO messages(id, chat_id, parent_id, role, content, status, created_at, updated_at) \
         VALUES(?, ?, ?, 'user', ?, 'complete', ?, ?)",
    ).bind(&user_id).bind(chat_id).bind(&request.parent_id).bind(&request.content).bind(now).bind(now)
        .execute(&mut **tx).await?;
    for (position, upload_id) in request.attachment_ids.iter().enumerate() {
        sqlx::query("INSERT INTO message_attachments(message_id, upload_id, position, pdf_engine) VALUES(?, ?, ?, ?)")
            .bind(&user_id).bind(upload_id).bind(position as i64).bind(&request.pdf_engine)
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
    Ok(sqlx::query_as::<_, MessageRecord>(
        "SELECT id, chat_id, parent_id, role, content, status, error, model, generation_id, finish_reason, \
                prompt_tokens, completion_tokens, reasoning_tokens, cost, created_at, updated_at \
         FROM messages WHERE id = ?",
    ).bind(id).fetch_optional(&mut **tx).await?)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
