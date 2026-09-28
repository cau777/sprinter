use crate::{settings, state::AppState, uploads};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

const HOUR: Duration = Duration::from_secs(60 * 60);

pub async fn maintenance_loop(state: AppState) {
    run_hourly(&state).await;
    loop {
        sleep(HOUR).await;
        run_hourly(&state).await;
    }
}

async fn run_hourly(state: &AppState) {
    match uploads::gc(&state.pool, &state.config.data_dir, now_ms()).await {
        Ok(removed) if removed > 0 => tracing::info!(removed, "gc"),
        Ok(_) => tracing::debug!("gc"),
        Err(error) => tracing::warn!(error = %error, "upload garbage collection failed"),
    }
    match sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now_ms())
        .execute(&state.pool)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => {
            tracing::info!(sessions_expired = result.rows_affected(), "gc")
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(error = %error, "expired session cleanup failed"),
    }
    match settings::refresh_models(state).await {
        Ok(count) => tracing::info!(count, "models refreshed"),
        Err(error) => tracing::warn!(error, "models refresh failed"),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
