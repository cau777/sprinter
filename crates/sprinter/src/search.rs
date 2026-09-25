use crate::{auth, state::AppState};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    middleware,
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashSet;
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, FromRow, TS)]
pub struct SearchResult {
    pub chat_id: String,
    pub chat_title: Option<String>,
    pub message_id: Option<String>,
    pub snippet: String,
    pub rank: f64,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/search", get(search_handler))
        .route_layer(middleware::from_fn_with_state(state, auth::require_session))
}

async fn search_handler(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<SearchResult>>, SearchError> {
    let query = query.q.unwrap_or_default();
    let query = query.trim();
    if query.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let started = std::time::Instant::now();
    let results = search(&state.pool, query).await.map_err(|error| {
        tracing::warn!(error = %error, "search failed");
        SearchError::invalid_query(&headers)
    })?;
    tracing::info!(
        query = %preview(query, 200),
        results = results.len(),
        duration_ms = started.elapsed().as_millis(),
        "search"
    );
    Ok(Json(results))
}

pub async fn search(
    pool: &sqlx::SqlitePool,
    query: &str,
) -> Result<Vec<SearchResult>, sqlx::Error> {
    let hits = sqlx::query_as::<_, SearchResult>(
        "SELECT search_fts.chat_id AS chat_id, chats.title AS chat_title, \
                search_fts.message_id AS message_id, \
                snippet(search_fts, 0, '<mark>', '</mark>', '…', 12) AS snippet, \
                bm25(search_fts) AS rank \
         FROM search_fts JOIN chats ON chats.id = search_fts.chat_id \
         WHERE search_fts MATCH ? ORDER BY bm25(search_fts)",
    )
    .bind(query)
    .fetch_all(pool)
    .await?;

    let mut seen = HashSet::new();
    Ok(hits
        .into_iter()
        .filter(|hit| seen.insert(hit.chat_id.clone()))
        .take(50)
        .collect())
}

fn preview(value: &str, max_chars: usize) -> String {
    let value = value.replace(['\n', '\r'], " ");
    let mut chars = value.chars();
    let preview = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

struct SearchError {
    request_id: String,
}

impl SearchError {
    fn invalid_query(headers: &HeaderMap) -> Self {
        Self {
            request_id: auth::request_id(headers),
        }
    }
}

impl axum::response::IntoResponse for SearchError {
    fn into_response(self) -> axum::response::Response {
        auth::api_error(
            &self.request_id,
            axum::http::StatusCode::BAD_REQUEST,
            "invalid_search_query",
            "Search query is invalid",
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::search;
    use crate::db;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn returns_best_hit_per_chat_for_messages_and_titles() {
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
        sqlx::query("INSERT INTO chats(id, title, model, created_at, updated_at) VALUES ('a', 'needle title', 'test', 1, 1), ('b', 'other title', 'test', 1, 1)")
            .execute(&pool)
            .await.unwrap();
        sqlx::query("INSERT INTO messages(id, chat_id, role, content, status, created_at, updated_at) VALUES ('m1', 'a', 'user', 'needle body', 'complete', 2, 2), ('m2', 'a', 'assistant', 'another needle body', 'complete', 3, 3), ('m3', 'b', 'assistant', 'needle body', 'complete', 2, 2)")
            .execute(&pool).await.unwrap();

        let results = search(&pool, "needle").await.unwrap();
        assert_eq!(results.len(), 2);
        let a = results.iter().find(|result| result.chat_id == "a").unwrap();
        assert_eq!(a.chat_title.as_deref(), Some("needle title"));
        assert_eq!(
            results
                .iter()
                .filter(|result| result.chat_id == "a")
                .count(),
            1
        );
        assert!(
            results
                .iter()
                .any(|result| result.chat_id == "b" && result.message_id.as_deref() == Some("m3"))
        );
        pool.close().await;
    }
}
