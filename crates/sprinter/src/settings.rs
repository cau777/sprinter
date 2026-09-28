use crate::{auth, config::Config, state::AppState};
use argon2::Argon2;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    http::StatusCode,
    middleware,
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use rand::{RngCore, rngs::OsRng};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use ts_rs::TS;

const MODELS_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
pub struct SettingsState {
    pool: SqlitePool,
    config: Arc<Config>,
    http: reqwest::Client,
    models: Arc<Mutex<Option<CachedModels>>>,
    api_key: Arc<RwLock<Option<SecretString>>>,
}

#[derive(Clone)]
struct CachedModels {
    loaded_at: Instant,
    items: Vec<ApiModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiModel {
    pub id: String,
    pub name: String,
    #[ts(type = "number")]
    pub context_length: u64,
    pub pricing: ModelPricing,
    pub input_modalities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelPricing {
    pub prompt: String,
    pub completion: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelsResponse {
    pub items: Vec<ApiModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OpenRouterKeyStatus {
    pub set: bool,
    pub hint: Option<String>,
    pub valid: bool,
    pub readable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UploadLimits {
    #[ts(type = "number")]
    pub image_bytes: u64,
    #[ts(type = "number")]
    pub pdf_bytes: u64,
    #[ts(type = "number")]
    pub text_bytes: u64,
    #[ts(type = "number")]
    pub files_per_message: u64,
    #[ts(type = "number")]
    pub total_prompt_bytes: u64,
}

impl Default for UploadLimits {
    fn default() -> Self {
        Self {
            image_bytes: 20 * 1024 * 1024,
            pdf_bytes: 50 * 1024 * 1024,
            text_bytes: 1024 * 1024,
            files_per_message: 10,
            total_prompt_bytes: 100 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingsResponse {
    pub openrouter_api_key: OpenRouterKeyStatus,
    pub default_model: Option<String>,
    pub title_model: Option<String>,
    pub favorite_models: Vec<String>,
    pub custom_instructions: String,
    pub pdf_engine: String,
    pub upload_limits: UploadLimits,
}

#[derive(Deserialize)]
struct RefreshQuery {
    refresh: Option<bool>,
}

#[derive(Clone, Serialize, Deserialize)]
struct EncryptedKey {
    salt: String,
    nonce: String,
    ciphertext: String,
    hint: String,
    valid: bool,
    readable: bool,
}

impl SettingsState {
    pub fn new(pool: SqlitePool, config: Arc<Config>) -> Self {
        Self {
            pool,
            config,
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .build()
                .expect("HTTP client configuration is valid"),
            models: Arc::new(Mutex::new(None)),
            api_key: Arc::new(RwLock::new(None)),
        }
    }

    fn base_url(&self, endpoint: &str) -> String {
        format!(
            "{}/{}",
            self.config.openrouter_base_url.trim_end_matches('/'),
            endpoint
        )
    }
}

pub async fn provider_key(state: &AppState) -> Result<Option<SecretString>, &'static str> {
    if let Some(cached) = cached_key(&state.settings) {
        return Ok(Some(cached));
    }
    let Some(mut encrypted) = stored_key(&state.settings.pool)
        .await
        .map_err(|_| "Could not read the stored OpenRouter key")?
    else {
        return Ok(None);
    };
    if !encrypted.readable {
        return Err("The stored OpenRouter key is unreadable");
    }
    let password = state
        .settings
        .config
        .master_password
        .as_ref()
        .ok_or("Master password is unavailable")?;
    match decrypt_key_with_password(password.expose_secret().as_bytes(), &encrypted) {
        Ok(value) => {
            let secret = SecretString::from(value);
            *state.settings.api_key.write().expect("key cache poisoned") = Some(secret.clone());
            Ok(Some(secret))
        }
        Err(_) => {
            encrypted.readable = false;
            persist_key(&state.settings.pool, &encrypted)
                .await
                .map_err(|_| "Could not save the unreadable key status")?;
            Err("The stored OpenRouter key could not be decrypted")
        }
    }
}

pub async fn default_model(state: &AppState) -> Result<Option<String>, &'static str> {
    setting_string(&state.settings.pool, "default_model")
        .await
        .map_err(|_| "Could not read the default model setting")
}

pub async fn title_model(state: &AppState) -> Result<Option<String>, &'static str> {
    setting_string(&state.settings.pool, "title_model")
        .await
        .map_err(|_| "Could not read the title model setting")
}

pub async fn custom_instructions(state: &AppState) -> Result<Option<String>, &'static str> {
    setting_string(&state.settings.pool, "custom_instructions")
        .await
        .map_err(|_| "Could not read custom instructions")
}

/// Returns cached OpenRouter modality support when the model record is available.
/// Unknown models are treated as non-vision by callers so existing image history
/// stays visible as a placeholder until model metadata can be refreshed.
pub async fn model_supports_images(state: &AppState, model: &str) -> Option<bool> {
    let Json(response) = get_models_inner(state.settings.clone(), RefreshQuery { refresh: None })
        .await
        .ok()?;
    response
        .items
        .iter()
        .find(|item| item.id == model)
        .map(|item| item.input_modalities.iter().any(|m| m == "image"))
}

/// Force a model catalog refresh for the background maintenance task.
pub async fn refresh_models(state: &AppState) -> Result<usize, &'static str> {
    let Json(response) = get_models_inner(
        state.settings.clone(),
        RefreshQuery {
            refresh: Some(true),
        },
    )
    .await
    .map_err(|error| error.message)?;
    Ok(response.items.len())
}

pub async fn default_pdf_engine(state: &AppState) -> Result<String, &'static str> {
    Ok(setting_value(&state.settings.pool, "pdf_engine")
        .await
        .map_err(|_| "Could not read the PDF engine setting")?
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "cloudflare-ai".to_owned()))
}

pub async fn upload_limits(state: &AppState) -> Result<UploadLimits, &'static str> {
    Ok(setting_value(&state.settings.pool, "upload_limits")
        .await
        .map_err(|_| "Could not read upload limits")?
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default())
}

pub fn router(state: AppState) -> Router<AppState> {
    let auth_state = state.clone();
    Router::new()
        .route("/api/settings", get(get_settings).patch(patch_settings))
        .route("/api/models", get(get_models))
        .route_layer(middleware::from_fn_with_state(
            auth_state,
            auth::require_session,
        ))
}

async fn get_settings(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SettingsResponse>, ResponseError> {
    let request_id = auth::request_id(&headers);
    get_settings_inner(app.settings.clone())
        .await
        .map_err(|error| error.with_request_id(request_id))
}

async fn get_settings_inner(
    state: Arc<SettingsState>,
) -> Result<Json<SettingsResponse>, ResponseError> {
    let mut key = stored_key(&state.pool).await?;
    if let Some(encrypted) = key.as_mut() {
        if encrypted.readable && cached_key(&state).is_none() {
            let password = state.config.master_password.as_ref().ok_or_else(|| {
                ResponseError::internal("master_password_missing", "Master password is unavailable")
            })?;
            match decrypt_key_with_password(password.expose_secret().as_bytes(), encrypted) {
                Ok(secret) => {
                    *state.api_key.write().expect("key cache poisoned") =
                        Some(SecretString::from(secret));
                }
                Err(_) => {
                    encrypted.readable = false;
                    persist_key(&state.pool, encrypted).await?;
                }
            }
        }
    }
    let key_status = match key {
        Some(key) => OpenRouterKeyStatus {
            set: true,
            hint: Some(key.hint),
            valid: key.valid && key.readable,
            readable: key.readable,
        },
        None => OpenRouterKeyStatus {
            set: false,
            hint: None,
            valid: false,
            readable: true,
        },
    };
    let settings = SettingsResponse {
        openrouter_api_key: key_status,
        default_model: setting_string(&state.pool, "default_model").await?,
        title_model: setting_string(&state.pool, "title_model").await?,
        favorite_models: setting_value(&state.pool, "favorite_models")
            .await?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        custom_instructions: setting_value(&state.pool, "custom_instructions")
            .await?
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        pdf_engine: setting_value(&state.pool, "pdf_engine")
            .await?
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "cloudflare-ai".into()),
        upload_limits: setting_value(&state.pool, "upload_limits")
            .await?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
    };
    Ok(Json(settings))
}

async fn patch_settings(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<Value>,
) -> Result<Json<SettingsResponse>, ResponseError> {
    let request_id = auth::request_id(&headers);
    patch_settings_inner(app.settings.clone(), patch)
        .await
        .map_err(|error| error.with_request_id(request_id))
}

async fn patch_settings_inner(
    state: Arc<SettingsState>,
    patch: Value,
) -> Result<Json<SettingsResponse>, ResponseError> {
    let Some(values) = patch.as_object() else {
        return Err(ResponseError::bad_request(
            "invalid_settings",
            "Settings patch must be a JSON object",
        ));
    };
    for key in values.keys() {
        if !matches!(
            key.as_str(),
            "openrouter_api_key"
                | "default_model"
                | "title_model"
                | "favorite_models"
                | "custom_instructions"
                | "pdf_engine"
                | "upload_limits"
        ) {
            return Err(ResponseError::bad_request(
                "unknown_setting",
                "Unknown setting",
            ));
        }
    }

    // Validate the complete patch before persisting any of it.
    let mut updates = Vec::new();
    for name in ["default_model", "title_model"] {
        if let Some(value) = values.get(name) {
            if !(value.is_null() || value.as_str().is_some()) {
                return Err(ResponseError::bad_request(
                    "invalid_setting",
                    "Model settings must be strings or null",
                ));
            }
            updates.push((name, value.clone()));
        }
    }
    if let Some(value) = values.get("favorite_models") {
        if !value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
        {
            return Err(ResponseError::bad_request(
                "invalid_setting",
                "favorite_models must be an array of strings",
            ));
        }
        updates.push(("favorite_models", value.clone()));
    }
    if let Some(value) = values.get("custom_instructions") {
        if !value.is_string() {
            return Err(ResponseError::bad_request(
                "invalid_setting",
                "custom_instructions must be a string",
            ));
        }
        updates.push(("custom_instructions", value.clone()));
    }
    if let Some(value) = values.get("pdf_engine") {
        if !matches!(
            value.as_str(),
            Some("cloudflare-ai" | "mistral-ocr" | "native")
        ) {
            return Err(ResponseError::bad_request(
                "invalid_setting",
                "pdf_engine must be cloudflare-ai, mistral-ocr, or native",
            ));
        }
        updates.push(("pdf_engine", value.clone()));
    }
    if let Some(value) = values.get("upload_limits") {
        let limits: UploadLimits = serde_json::from_value(value.clone()).map_err(|_| {
            ResponseError::bad_request("invalid_setting", "upload_limits has an invalid shape")
        })?;
        validate_limits(&limits)?;
        updates.push((
            "upload_limits",
            serde_json::to_value(limits).expect("upload limits serialize"),
        ));
    }

    let mut plaintext_key = None;
    let new_key = match values.get("openrouter_api_key") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(value)) if value.trim().is_empty() => Some(None),
        Some(Value::String(value)) => {
            validate_api_key(&state, value).await?;
            plaintext_key = Some(Some(value.clone()));
            Some(Some(encrypt_key(&state, value)?))
        }
        Some(_) => {
            return Err(ResponseError::bad_request(
                "invalid_setting",
                "openrouter_api_key must be a string or null",
            ));
        }
    };

    let mut tx = state.pool.begin().await.map_err(ResponseError::database)?;
    for (name, value) in updates {
        let value = serde_json::to_string(&value).expect("JSON value serializes");
        sqlx::query("INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
            .bind(name).bind(value).bind(now_ms())
            .execute(&mut *tx).await.map_err(ResponseError::database)?;
    }
    if let Some(key) = &new_key {
        match key {
            Some(key) => {
                let value = serde_json::to_string(key).expect("encrypted key serializes");
                sqlx::query("INSERT INTO settings (key, value, updated_at) VALUES ('openrouter_api_key', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
                    .bind(value).bind(now_ms()).execute(&mut *tx).await.map_err(ResponseError::database)?;
            }
            None => {
                sqlx::query("DELETE FROM settings WHERE key = 'openrouter_api_key'")
                    .execute(&mut *tx)
                    .await
                    .map_err(ResponseError::database)?;
            }
        }
    }
    tx.commit().await.map_err(ResponseError::database)?;

    if let Some(key) = new_key {
        debug_assert!(key.is_none() || plaintext_key.as_ref().is_some_and(Option::is_some));
        *state.api_key.write().expect("key cache poisoned") =
            plaintext_key.flatten().map(SecretString::from);
    }

    get_settings_inner(state).await
}

async fn get_models(
    State(app): State<AppState>,
    Query(query): Query<RefreshQuery>,
    headers: HeaderMap,
) -> Result<Json<ModelsResponse>, ResponseError> {
    let request_id = auth::request_id(&headers);
    get_models_inner(app.settings.clone(), query)
        .await
        .map_err(|error| error.with_request_id(request_id))
}

async fn get_models_inner(
    state: Arc<SettingsState>,
    query: RefreshQuery,
) -> Result<Json<ModelsResponse>, ResponseError> {
    let mut cache = state.models.lock().await;
    if !query.refresh.unwrap_or(false) {
        if let Some(cached) = cache
            .as_ref()
            .filter(|c| c.loaded_at.elapsed() < MODELS_TTL)
        {
            return Ok(Json(ModelsResponse {
                items: cached.items.clone(),
            }));
        }
    }
    let response = state
        .http
        .get(state.base_url("models"))
        .send()
        .await
        .map_err(|_| ResponseError::provider("model_fetch_failed", "Could not reach OpenRouter"))?;
    if !response.status().is_success() {
        return Err(ResponseError::provider(
            "model_fetch_failed",
            "OpenRouter returned an error while listing models",
        ));
    }
    let body: Value = response.json().await.map_err(|_| {
        ResponseError::provider(
            "invalid_provider_response",
            "OpenRouter returned invalid model data",
        )
    })?;
    let upstream = body.get("data").and_then(Value::as_array).ok_or_else(|| {
        ResponseError::provider(
            "invalid_provider_response",
            "OpenRouter model response has no data array",
        )
    })?;
    let items = upstream.iter().filter_map(parse_model).collect::<Vec<_>>();
    *cache = Some(CachedModels {
        loaded_at: Instant::now(),
        items: items.clone(),
    });
    Ok(Json(ModelsResponse { items }))
}

fn parse_model(value: &Value) -> Option<ApiModel> {
    let id = value.get("id")?.as_str()?.to_owned();
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_owned();
    let context_length = value
        .get("context_length")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let pricing = value.get("pricing").cloned().unwrap_or_default();
    let prompt = pricing
        .get("prompt")
        .map(value_string)
        .unwrap_or_else(|| "0".into());
    let completion = pricing
        .get("completion")
        .map(value_string)
        .unwrap_or_else(|| "0".into());
    let input_modalities = value
        .pointer("/architecture/input_modalities")
        .and_then(Value::as_array)
        .map(|modalities| {
            modalities
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| vec!["text".into()]);
    Some(ApiModel {
        id,
        name,
        context_length,
        pricing: ModelPricing { prompt, completion },
        input_modalities,
    })
}

fn value_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

async fn validate_api_key(state: &SettingsState, key: &str) -> Result<(), ResponseError> {
    let response = state
        .http
        .get(state.base_url("key"))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| {
            ResponseError::provider(
                "key_validation_failed",
                "Could not reach OpenRouter to validate the API key",
            )
        })?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err(ResponseError::bad_request(
            "invalid_api_key",
            "OpenRouter rejected this API key",
        ));
    }
    Err(ResponseError::provider(
        "key_validation_failed",
        "OpenRouter returned an error while validating the API key",
    ))
}

fn encrypt_key(state: &SettingsState, secret: &str) -> Result<EncryptedKey, ResponseError> {
    let password = state.config.master_password.as_ref().ok_or_else(|| {
        ResponseError::internal("master_password_missing", "Master password is unavailable")
    })?;
    encrypt_key_with_password(password.expose_secret().as_bytes(), secret)
}

fn encrypt_key_with_password(password: &[u8], secret: &str) -> Result<EncryptedKey, ResponseError> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    let key = derive_key(password, &salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), secret.as_bytes())
        .map_err(|_| {
            ResponseError::internal(
                "key_encryption_failed",
                "Could not encrypt the OpenRouter key",
            )
        })?;
    let last4 = secret
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    Ok(EncryptedKey {
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
        hint: format!("sk-or-…{last4}"),
        valid: true,
        readable: true,
    })
}

fn decrypt_key_with_password(
    password: &[u8],
    encrypted: &EncryptedKey,
) -> Result<String, ResponseError> {
    if !encrypted.readable {
        return Err(ResponseError::internal(
            "stored_key_unreadable",
            "The saved OpenRouter key is unreadable",
        ));
    }
    let salt = STANDARD.decode(&encrypted.salt).map_err(|_| {
        ResponseError::internal(
            "stored_key_invalid",
            "Stored OpenRouter key data is invalid",
        )
    })?;
    let nonce = STANDARD.decode(&encrypted.nonce).map_err(|_| {
        ResponseError::internal(
            "stored_key_invalid",
            "Stored OpenRouter key data is invalid",
        )
    })?;
    let ciphertext = STANDARD.decode(&encrypted.ciphertext).map_err(|_| {
        ResponseError::internal(
            "stored_key_invalid",
            "Stored OpenRouter key data is invalid",
        )
    })?;
    if salt.len() != 16 || nonce.len() != 24 || ciphertext.len() < 16 {
        return Err(ResponseError::internal(
            "stored_key_invalid",
            "Stored OpenRouter key data is invalid",
        ));
    }
    let key = derive_key(password, &salt)?;
    let plaintext = XChaCha20Poly1305::new((&key).into())
        .decrypt(XNonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| {
            ResponseError::internal(
                "stored_key_unreadable",
                "The saved OpenRouter key cannot be decrypted",
            )
        })?;
    String::from_utf8(plaintext).map_err(|_| {
        ResponseError::internal(
            "stored_key_unreadable",
            "The saved OpenRouter key is not valid text",
        )
    })
}

fn cached_key(state: &SettingsState) -> Option<SecretString> {
    state.api_key.read().expect("key cache poisoned").clone()
}

async fn persist_key(pool: &SqlitePool, key: &EncryptedKey) -> Result<(), ResponseError> {
    let value = serde_json::to_string(key).expect("encrypted key serializes");
    sqlx::query("UPDATE settings SET value = ?, updated_at = ? WHERE key = 'openrouter_api_key'")
        .bind(value)
        .bind(now_ms())
        .execute(pool)
        .await
        .map_err(ResponseError::database)?;
    Ok(())
}

fn derive_key(password: &[u8], salt: &[u8]) -> Result<[u8; 32], ResponseError> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password, salt, &mut key)
        .map_err(|_| {
            ResponseError::internal(
                "key_derivation_failed",
                "Could not derive the OpenRouter encryption key",
            )
        })?;
    Ok(key)
}

async fn stored_key(pool: &SqlitePool) -> Result<Option<EncryptedKey>, ResponseError> {
    let Some(value) = setting_value(pool, "openrouter_api_key").await? else {
        return Ok(None);
    };
    serde_json::from_value(value).map(Some).map_err(|_| {
        ResponseError::internal(
            "stored_key_invalid",
            "Stored OpenRouter key data is invalid",
        )
    })
}

async fn setting_string(pool: &SqlitePool, name: &str) -> Result<Option<String>, ResponseError> {
    Ok(setting_value(pool, name)
        .await?
        .and_then(|value| value.as_str().map(str::to_owned)))
}

async fn setting_value(pool: &SqlitePool, name: &str) -> Result<Option<Value>, ResponseError> {
    let value = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(name)
        .fetch_optional(pool)
        .await
        .map_err(ResponseError::database)?;
    value
        .map(|value| {
            serde_json::from_str(&value).map_err(|_| {
                ResponseError::internal("stored_setting_invalid", "A saved setting is invalid")
            })
        })
        .transpose()
}

fn validate_limits(limits: &UploadLimits) -> Result<(), ResponseError> {
    if limits.image_bytes == 0
        || limits.image_bytes > 40 * 1024 * 1024
        || limits.pdf_bytes == 0
        || limits.pdf_bytes > 100 * 1024 * 1024
        || limits.text_bytes == 0
        || limits.text_bytes > 5 * 1024 * 1024
        || limits.files_per_message == 0
        || limits.files_per_message > 20
        || limits.total_prompt_bytes == 0
        || limits.total_prompt_bytes > 200 * 1024 * 1024
    {
        return Err(ResponseError::bad_request(
            "invalid_upload_limits",
            "Upload limits must be within the documented hard limits",
        ));
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug)]
struct ResponseError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: String,
}
impl ResponseError {
    fn bad_request(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message,
            request_id: "unknown".into(),
        }
    }
    fn internal(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
            message,
            request_id: "unknown".into(),
        }
    }
    fn provider(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code,
            message,
            request_id: "unknown".into(),
        }
    }
    fn database(error: sqlx::Error) -> Self {
        tracing::error!(error = %error, "settings database error");
        Self::internal("database_error", "Could not access saved settings")
    }
    fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = request_id;
        self
    }
}
impl axum::response::IntoResponse for ResponseError {
    fn into_response(self) -> axum::response::Response {
        auth::api_error(&self.request_id, self.status, self.code, self.message, None)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RefreshQuery, SettingsState, decrypt_key_with_password, encrypt_key_with_password,
        get_models_inner, parse_model, patch_settings_inner,
    };
    use crate::{config::Config, settings, state::AppState};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use secrecy::SecretString;
    use serde_json::json;
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use std::{net::SocketAddr, path::PathBuf, sync::Arc};
    use tower::ServiceExt;

    #[test]
    fn api_key_encryption_round_trips_and_authenticates_ciphertext() {
        let mut encrypted =
            encrypt_key_with_password(b"master password", "sk-or-v1-secret1234").unwrap();
        assert_eq!(
            decrypt_key_with_password(b"master password", &encrypted).unwrap(),
            "sk-or-v1-secret1234"
        );
        assert!(decrypt_key_with_password(b"changed password", &encrypted).is_err());

        encrypted.ciphertext.replace_range(0..4, "AAAA");
        assert!(decrypt_key_with_password(b"master password", &encrypted).is_err());
    }

    #[test]
    fn model_records_are_trimmed_to_picker_fields() {
        let model = parse_model(&json!({
            "id": "vendor/model",
            "name": "Vendor Model",
            "context_length": 32768,
            "pricing": {"prompt": "0.000001", "completion": "0.000002", "request": "0"},
            "architecture": {"input_modalities": ["text", "image"], "output_modalities": ["text"]},
            "description": "should not be exposed"
        }))
        .unwrap();
        assert_eq!(model.id, "vendor/model");
        assert_eq!(model.context_length, 32768);
        assert_eq!(model.pricing.prompt, "0.000001");
        assert_eq!(model.input_modalities, ["text", "image"]);
    }

    #[tokio::test]
    async fn settings_and_models_require_a_session() {
        let pool = test_pool().await;
        let config = test_config("http://127.0.0.1:1/api/v1");
        let state = AppState::new(pool.clone(), config);
        let app = settings::router(state.clone()).with_state(state);

        for path in ["/api/settings", "/api/models"] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn api_key_is_validated_encrypted_and_model_results_are_cached() {
        use serde_json::json;
        use tokio::net::TcpListener;

        let fake = fake_openrouter::FakeOpenRouter::new();
        let request_log = fake.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, fake.router()).await.unwrap();
        });
        let base_url = format!("http://{address}/api/v1");
        let pool = test_pool().await;
        let state = Arc::new(SettingsState::new(pool.clone(), test_config(&base_url)));

        let error = patch_settings_inner(state.clone(), json!({"openrouter_api_key":"bad-key"}))
            .await
            .unwrap_err();
        assert_eq!(error.code, "invalid_api_key");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM settings WHERE key = 'openrouter_api_key'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );

        let saved = patch_settings_inner(state.clone(), json!({"openrouter_api_key":"test-key"}))
            .await
            .unwrap();
        assert!(saved.0.openrouter_api_key.set);
        assert_eq!(
            saved.0.openrouter_api_key.hint.as_deref(),
            Some("sk-or-…-key")
        );
        let stored = sqlx::query_scalar::<_, String>(
            "SELECT value FROM settings WHERE key = 'openrouter_api_key'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!stored.contains("test-key"));

        let first = get_models_inner(state.clone(), RefreshQuery { refresh: None })
            .await
            .unwrap();
        let second = get_models_inner(state.clone(), RefreshQuery { refresh: None })
            .await
            .unwrap();
        assert_eq!(first.0.items.len(), 3);
        assert_eq!(second.0.items[1].id, "test/vision");
        assert_eq!(
            request_log
                .requests_snapshot()
                .iter()
                .filter(|request| request.path == "/api/v1/models")
                .count(),
            1
        );

        server.abort();
        pool.close().await;
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(":memory:")
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    fn test_config(base_url: &str) -> Arc<Config> {
        Arc::new(Config {
            master_password: Some(SecretString::from("test-password")),
            data_dir: PathBuf::from("/tmp/sprinter-settings-test"),
            bind_address: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            trusted_proxies: Vec::new(),
            worker_threads: 2,
            log_filter: "warn".into(),
            log_keep_days: 7,
            insecure_cookies: true,
            openrouter_base_url: base_url.into(),
        })
    }
}
