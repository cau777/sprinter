use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error as ThisError;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChatSummary {
    pub id: String,
    pub title: Option<String>,
    pub model: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatPage {
    pub items: Vec<ChatSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, ThisError)]
pub enum ChatError {
    #[error("chat not found")]
    NotFound,
    #[error("invalid cursor")]
    InvalidCursor,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
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

pub async fn create_chat(pool: &SqlitePool, model: &str) -> Result<ChatSummary, ChatError> {
    let id = Uuid::now_v7().to_string();
    let now = now_ms();
    sqlx::query("INSERT INTO chats(id, model, created_at, updated_at) VALUES(?, ?, ?, ?)")
        .bind(&id)
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
