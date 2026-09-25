use crate::{auth::AuthContext, config::Config, generation::Manager, settings::SettingsState};
use sqlx::SqlitePool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    pub auth: Arc<AuthContext>,
    pub settings: Arc<SettingsState>,
    pub generation: Arc<Manager>,
}

impl AppState {
    pub fn new(pool: SqlitePool, config: Arc<Config>) -> Self {
        let auth = Arc::new(AuthContext::new(&config));
        let settings = Arc::new(SettingsState::new(pool.clone(), config.clone()));
        let generation = Arc::new(Manager::new(
            pool.clone(),
            config.openrouter_base_url.clone(),
        ));
        Self {
            pool,
            config,
            auth,
            settings,
            generation,
        }
    }
}
