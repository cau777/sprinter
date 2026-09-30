use crate::config::Config;
use sqlx::{
    SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{error::Error, fs};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn connect(config: &Config) -> Result<SqlitePool, Box<dyn Error>> {
    fs::create_dir_all(&config.data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(&config.data_dir, fs::Permissions::from_mode(0o700))
        {
            // Mounted volumes may be permission-managed by the runtime (or use
            // a filesystem that rejects chmod). The directory still has to be
            // writable below; keep serving and make the operator aware that
            // their mount is responsible for protecting application data.
            eprintln!(
                "could not secure data directory {}: {error}; continuing with mount-managed permissions",
                config.data_dir.display()
            );
        }
    }
    let options = SqliteConnectOptions::new()
        .filename(config.data_dir.join("sprinter.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .pragma("cache_size", "-8192")
        .pragma("mmap_size", "8388608");
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

#[cfg(test)]
mod tests {
    use super::{SqlitePoolOptions, migrate};
    use sqlx::SqlitePool;

    async fn fts_count(pool: &SqlitePool, term: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM search_fts WHERE search_fts MATCH ?")
            .bind(term)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn empty_database_migrates_and_search_tracks_final_messages_and_titles() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(":memory:")
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        migrate(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO chats (id, model, created_at, updated_at) VALUES ('chat-1', 'test/text', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, chat_id, role, content, status, created_at, updated_at) VALUES ('message-1', 'chat-1', 'assistant', 'needle reply', 'streaming', 2, 2)",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(fts_count(&pool, "needle").await, 0);

        sqlx::query("UPDATE messages SET status = 'complete' WHERE id = 'message-1'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(fts_count(&pool, "needle").await, 1);

        sqlx::query("UPDATE chats SET title = 'needle title' WHERE id = 'chat-1'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(fts_count(&pool, "needle").await, 2);

        sqlx::query("UPDATE messages SET content = 'replacement reply' WHERE id = 'message-1'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(fts_count(&pool, "needle").await, 1);
        assert_eq!(fts_count(&pool, "replacement").await, 1);

        pool.close().await;
    }
}
