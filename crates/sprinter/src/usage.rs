use crate::{auth, settings, state::AppState};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, Days, NaiveDate, Utc};
use chrono_tz::Tz;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use ts_rs::TS;

const BALANCE_TTL: Duration = Duration::from_secs(60);
const CHAT_LIMIT: usize = 20;

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct UsageTotals {
    #[ts(type = "number")]
    pub prompt_tokens: i64,
    #[ts(type = "number")]
    pub completion_tokens: i64,
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ModelUsage {
    pub model: String,
    #[ts(type = "number")]
    pub prompt_tokens: i64,
    #[ts(type = "number")]
    pub completion_tokens: i64,
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ChatUsage {
    pub chat_id: String,
    pub chat_title: Option<String>,
    #[ts(type = "number")]
    pub prompt_tokens: i64,
    #[ts(type = "number")]
    pub completion_tokens: i64,
    pub cost: f64,
    pub title_cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct DailyUsage {
    pub day: String,
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct UsageResponse {
    pub balance: Option<f64>,
    pub totals: UsagePeriods,
    pub by_model: Vec<ModelUsage>,
    pub by_chat: Vec<ChatUsage>,
    pub daily: Vec<DailyUsage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct UsagePeriods {
    pub today: UsageTotals,
    pub d7: UsageTotals,
    pub d30: UsageTotals,
    pub all: UsageTotals,
}

#[derive(Deserialize)]
struct UsageQuery {
    tz: Option<String>,
}

#[derive(FromRow)]
struct UsageRow {
    model: String,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    cost: Option<f64>,
    created_at: i64,
    chat_id: Option<String>,
    chat_title: Option<String>,
    source: String,
}

#[derive(Clone)]
struct CachedBalance {
    key_hash: [u8; 32],
    loaded_at: Instant,
    balance: Option<f64>,
}

static BALANCE_CACHE: OnceLock<Mutex<Option<CachedBalance>>> = OnceLock::new();

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/usage", get(usage_handler))
        .route_layer(middleware::from_fn_with_state(state, auth::require_session))
}

async fn usage_handler(
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
    headers: HeaderMap,
) -> Result<Json<UsageResponse>, UsageError> {
    let tz = query
        .tz
        .as_deref()
        .unwrap_or("UTC")
        .parse::<Tz>()
        .map_err(|_| UsageError::invalid_timezone(&headers))?;
    let response = usage(&state.pool, tz)
        .await
        .map_err(|_| UsageError::internal(&headers))?;
    let (balance, cache_hit) = fetch_balance(&state).await;
    tracing::debug!("usage viewed");
    tracing::info!(balance = ?balance, cache_hit, "balance fetched");
    Ok(Json(UsageResponse {
        balance,
        ..response
    }))
}

async fn usage(pool: &sqlx::SqlitePool, tz: Tz) -> Result<UsageResponse, sqlx::Error> {
    let rows = sqlx::query_as::<_, UsageRow>(
        "SELECT m.model, m.prompt_tokens, m.completion_tokens, m.cost, m.created_at, \
                m.chat_id, c.title AS chat_title, 'message' AS source \
         FROM messages m LEFT JOIN chats c ON c.id = m.chat_id WHERE m.model IS NOT NULL \
         UNION ALL \
         SELECT e.model, e.prompt_tokens, e.completion_tokens, e.cost, e.created_at, \
                e.chat_id, c.title AS chat_title, e.source AS source \
         FROM usage_events e LEFT JOIN chats c ON c.id = e.chat_id \
         UNION ALL \
         SELECT r.model, r.prompt_tokens, r.completion_tokens, r.cost, \
                CAST(strftime('%s', r.day) AS INTEGER) * 1000 AS created_at, \
                NULL AS chat_id, NULL AS chat_title, 'rollup' AS source \
         FROM usage_rollup r",
    )
    .fetch_all(pool)
    .await?;

    let local_today = Utc::now().with_timezone(&tz).date_naive();
    let start_7 = local_today - Days::new(6);
    let start_30 = local_today - Days::new(29);
    let mut totals = UsagePeriods {
        today: UsageTotals::default(),
        d7: UsageTotals::default(),
        d30: UsageTotals::default(),
        all: UsageTotals::default(),
    };
    let mut by_model = HashMap::<String, UsageTotals>::new();
    let mut by_chat = HashMap::<String, ChatUsage>::new();
    let mut daily = HashMap::<NaiveDate, f64>::new();

    for row in rows {
        let timestamp = DateTime::<Utc>::from_timestamp_millis(row.created_at)
            .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
        let day = timestamp.with_timezone(&tz).date_naive();
        let amount = UsageTotals {
            prompt_tokens: row.prompt_tokens.unwrap_or(0),
            completion_tokens: row.completion_tokens.unwrap_or(0),
            cost: row.cost.unwrap_or(0.0),
        };
        totals.all.add(&amount);
        if day == local_today {
            totals.today.add(&amount);
        }
        if day >= start_7 && day <= local_today {
            totals.d7.add(&amount);
        }
        if day >= start_30 && day <= local_today {
            totals.d30.add(&amount);
            *daily.entry(day).or_default() += amount.cost;
        }
        by_model.entry(row.model.clone()).or_default().add(&amount);
        if let Some(chat_id) = row.chat_id {
            let chat = by_chat.entry(chat_id.clone()).or_insert_with(|| ChatUsage {
                chat_id,
                chat_title: row.chat_title.clone(),
                prompt_tokens: 0,
                completion_tokens: 0,
                cost: 0.0,
                title_cost: 0.0,
            });
            chat.prompt_tokens += amount.prompt_tokens;
            chat.completion_tokens += amount.completion_tokens;
            chat.cost += amount.cost;
            if row.source == "title" {
                chat.title_cost += amount.cost;
            }
        }
    }

    let mut by_model = by_model
        .into_iter()
        .map(|(model, total)| ModelUsage {
            model,
            prompt_tokens: total.prompt_tokens,
            completion_tokens: total.completion_tokens,
            cost: total.cost,
        })
        .collect::<Vec<_>>();
    by_model.sort_by(|left, right| right.cost.total_cmp(&left.cost));
    let mut by_chat = by_chat.into_values().collect::<Vec<_>>();
    by_chat.sort_by(|left, right| right.cost.total_cmp(&left.cost));
    by_chat.truncate(CHAT_LIMIT);
    let daily = (0..30)
        .rev()
        .map(|offset| {
            let day = local_today - Days::new(offset);
            DailyUsage {
                day: day.to_string(),
                cost: daily.get(&day).copied().unwrap_or(0.0),
            }
        })
        .collect();
    Ok(UsageResponse {
        balance: None,
        totals,
        by_model,
        by_chat,
        daily,
    })
}

impl UsageTotals {
    fn add(&mut self, other: &Self) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.cost += other.cost;
    }
}

async fn fetch_balance(state: &AppState) -> (Option<f64>, bool) {
    let key = match settings::provider_key(state).await {
        Ok(Some(key)) => key,
        Ok(None) => return (None, false),
        Err(error) => {
            tracing::warn!(error, "usage balance unavailable");
            return (None, false);
        }
    };
    let key_hash: [u8; 32] = Sha256::digest(key.expose_secret().as_bytes()).into();
    let cache = BALANCE_CACHE.get_or_init(|| Mutex::new(None));
    if let Some(cached) = cache.lock().expect("balance cache poisoned").as_ref()
        && cached.key_hash == key_hash
        && cached.loaded_at.elapsed() < BALANCE_TTL
    {
        return (cached.balance, true);
    }
    let url = format!(
        "{}/key",
        state.config.openrouter_base_url.trim_end_matches('/')
    );
    let result = reqwest::Client::new()
        .get(url)
        .bearer_auth(key.expose_secret())
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    let balance = match result {
        Ok(response) if response.status().is_success() => response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| body.pointer("/data/limit_remaining").cloned())
            .and_then(|value| match value {
                serde_json::Value::Number(number) => number.as_f64(),
                serde_json::Value::String(value) => value.parse().ok(),
                _ => None,
            }),
        Ok(response) => {
            tracing::warn!(status = response.status().as_u16(), "balance fetch failed");
            return (None, false);
        }
        Err(_) => {
            tracing::warn!("balance fetch failed");
            return (None, false);
        }
    };
    *cache.lock().expect("balance cache poisoned") = Some(CachedBalance {
        key_hash,
        loaded_at: Instant::now(),
        balance,
    });
    (balance, false)
}

struct UsageError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: String,
}

impl UsageError {
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
    fn invalid_timezone(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::BAD_REQUEST,
            "invalid_timezone",
            "tz must be a valid IANA time zone",
        )
    }
    fn internal(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred",
        )
    }
}

impl IntoResponse for UsageError {
    fn into_response(self) -> Response {
        auth::api_error(&self.request_id, self.status, self.code, self.message, None)
    }
}

#[cfg(test)]
mod tests {
    use super::usage;
    use crate::{chats, db};
    use chrono_tz::UTC;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn includes_messages_events_and_deleted_chat_rollup_in_totals() {
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
        sqlx::query("INSERT INTO chats(id, title, model, created_at, updated_at) VALUES ('chat-a', 'A', 'model/a', ?, ?)")
            .bind(now_ms()).bind(now_ms()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages(id, chat_id, role, content, status, model, prompt_tokens, completion_tokens, cost, created_at, updated_at) VALUES ('msg-a', 'chat-a', 'assistant', 'reply', 'complete', 'model/a', 4, 2, 0.5, ?, ?)")
            .bind(now_ms()).bind(now_ms()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO usage_events(id, chat_id, source, model, prompt_tokens, completion_tokens, cost, created_at) VALUES ('event-a', 'chat-a', 'title', 'model/title', 1, 1, 0.1, ?)")
            .bind(now_ms()).execute(&pool).await.unwrap();
        let before = usage(&pool, UTC).await.unwrap();
        assert_eq!(before.totals.all.cost, 0.6);
        assert_eq!(before.by_chat[0].title_cost, 0.1);
        chats::delete_chat(&pool, "chat-a").await.unwrap();
        let after = usage(&pool, UTC).await.unwrap();
        assert_eq!(after.totals.all.cost, before.totals.all.cost);
        assert_eq!(after.by_chat.len(), 0);
        assert!(
            after
                .by_model
                .iter()
                .any(|model| model.model == "model/a" && model.cost == 0.5)
        );
        pool.close().await;
    }

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}
