use crate::config::Config;
use sqlx::{sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions}, SqlitePool};
use std::{error::Error, fs};

pub async fn connect(config: &Config) -> Result<SqlitePool, Box<dyn Error>> {
    fs::create_dir_all(&config.data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config.data_dir, fs::Permissions::from_mode(0o700))?;
    }
    let options = SqliteConnectOptions::new()
        .filename(config.data_dir.join("sprinter.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .pragma("cache_size", "-8192")
        .pragma("mmap_size", "8388608");
    Ok(SqlitePoolOptions::new().max_connections(4).connect_with(options).await?)
}
