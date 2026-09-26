use crate::{settings, state::AppState, uploads};
use chrono::{DateTime, Days, NaiveDate, Timelike, Utc};
use sqlx::SqlitePool;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error as ThisError;
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

pub async fn daily_snapshot_loop(pool: SqlitePool, data_dir: PathBuf, backup_keep: usize) {
    loop {
        sleep(until_0300_utc()).await;
        if let Err(error) = write_snapshot(&pool, &data_dir, backup_keep).await {
            tracing::error!(error = %error, "snapshot failed");
        }
    }
}

pub async fn write_snapshot(
    pool: &SqlitePool,
    data_dir: &Path,
    backup_keep: usize,
) -> Result<(), SnapshotError> {
    let started = Instant::now();
    let backups = data_dir.join("backups");
    fs::create_dir_all(&backups)?;
    set_dir_mode_0700(&backups)?;
    let temp_dir = data_dir.join("tmp");
    fs::create_dir_all(&temp_dir)?;
    set_dir_mode_0700(&temp_dir)?;
    let today = Utc::now().date_naive();
    let filename = format!("sprinter-{}.db", today.format("%Y-%m-%d"));
    let target = backups.join(filename);
    let temp = temp_dir.join(format!(
        ".{}.tmp",
        target.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_file(&temp);
    let escaped = temp.to_string_lossy().replace('\'', "''");
    if let Err(error) = sqlx::query(&format!("VACUUM INTO '{escaped}'"))
        .execute(pool)
        .await
    {
        let _ = fs::remove_file(&temp);
        return Err(SnapshotError::Database(error));
    }
    set_file_mode_0600(&temp)?;
    fs::rename(&temp, &target)?;
    let bytes = fs::metadata(&target)?.len();
    let pruned = prune_snapshots(&backups, backup_keep, &target)?;
    tracing::info!(
        path = %target.display(),
        bytes,
        duration_ms = started.elapsed().as_millis(),
        pruned,
        "snapshot written"
    );
    Ok(())
}

fn prune_snapshots(backups: &Path, keep: usize, current: &Path) -> Result<usize, std::io::Error> {
    let mut snapshots = fs::read_dir(backups)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("sprinter-") && name.ends_with(".db"))
        })
        .collect::<Vec<_>>();
    snapshots.sort();
    let remove_count = snapshots.len().saturating_sub(keep.max(1));
    let mut removed = 0;
    for path in snapshots.into_iter().take(remove_count) {
        if path != current && fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn until_0300_utc() -> Duration {
    let now = Utc::now();
    let today = now.date_naive();
    let target = today
        .and_hms_opt(3, 0, 0)
        .expect("valid UTC time")
        .and_utc();
    let target = if now < target {
        target
    } else {
        (today + Days::new(1))
            .and_hms_opt(3, 0, 0)
            .expect("valid UTC time")
            .and_utc()
    };
    (target - now).to_std().unwrap_or(Duration::from_secs(1))
}

fn set_dir_mode_0700(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

fn set_file_mode_0600(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, ThisError)]
pub enum SnapshotError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub fn next_snapshot_date(now: DateTime<Utc>) -> NaiveDate {
    let today = now.date_naive();
    let cutoff = today.and_hms_opt(3, 0, 0).expect("valid UTC time");
    if now.time().hour() < cutoff.hour() {
        today
    } else {
        today + Days::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::{next_snapshot_date, write_snapshot};
    use chrono::{NaiveDate, TimeZone, Utc};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn next_snapshot_is_at_three_utc() {
        let before = Utc.with_ymd_and_hms(2026, 9, 25, 2, 59, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 9, 25, 3, 0, 0).unwrap();
        assert_eq!(
            next_snapshot_date(before),
            NaiveDate::from_ymd_opt(2026, 9, 25).unwrap()
        );
        assert_eq!(
            next_snapshot_date(after),
            NaiveDate::from_ymd_opt(2026, 9, 26).unwrap()
        );
    }

    #[tokio::test]
    async fn writes_consistent_snapshot_and_prunes_old_days() {
        let dir = std::env::temp_dir().join(format!("sprinter-backup-{}", unique_id()));
        let data_dir = dir.join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("sprinter.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db_path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("CREATE TABLE sample(value TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO sample(value) VALUES ('backup data')")
            .execute(&pool)
            .await
            .unwrap();
        let backups = data_dir.join("backups");
        fs::create_dir_all(&backups).unwrap();
        fs::write(backups.join("sprinter-2000-01-01.db"), "old").unwrap();
        write_snapshot(&pool, &data_dir, 1).await.unwrap();
        let files = fs::read_dir(&backups)
            .unwrap()
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 1);
        let snapshot = files[0].path();
        assert!(fs::read_dir(data_dir.join("tmp")).unwrap().next().is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&snapshot).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&backups).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let backup = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&snapshot))
            .await
            .unwrap();
        let value: String = sqlx::query_scalar("SELECT value FROM sample")
            .fetch_one(&backup)
            .await
            .unwrap();
        assert_eq!(value, "backup data");
        backup.close().await;
        pool.close().await;
        fs::remove_dir_all(dir).unwrap();
    }

    fn unique_id() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
    }
}
