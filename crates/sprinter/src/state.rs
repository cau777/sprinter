use crate::{auth::AuthContext, config::Config};
use sqlx::SqlitePool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    pub auth: Arc<AuthContext>,
}

impl AppState {
    pub fn new(pool: SqlitePool, config: Arc<Config>) -> Self {
        let auth = Arc::new(AuthContext::new(&config));
        Self { pool, config, auth }
    }
}
