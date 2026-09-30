use crate::{auth, state::AppState};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::put,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{
    fs::{self, File},
    io::Write,
    path::{Path as FsPath, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use uuid::Uuid;

const TMP_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
const ORPHAN_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
const PDF_TEXT_MAX_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_PDF_EXTRACTOR: &str = "pdfjs-6.3.289";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfTextStats {
    pub chars: i64,
    pub pages: i64,
    pub empty_pages: i64,
}

#[derive(Clone, Debug)]
pub struct ExpandedPrompt {
    pub messages: Vec<crate::openrouter::ChatMessage>,
    pub pdf_text: usize,
    pub pdf_native: usize,
    pub pdf_omitted: usize,
}
impl ExpandedPrompt {
    pub fn needs_native_pdf_plugin(&self) -> bool {
        self.pdf_native > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadKind {
    Image,
    Pdf,
    Text,
}
impl UploadKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Pdf => "pdf",
            Self::Text => "text",
        }
    }
    fn limit(self, limits: &crate::settings::UploadLimits) -> u64 {
        match self {
            Self::Image => limits.image_bytes.min(40 * 1024 * 1024),
            Self::Pdf => limits.pdf_bytes.min(100 * 1024 * 1024),
            Self::Text => limits.text_bytes.min(5 * 1024 * 1024),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct UploadRecord {
    pub id: String,
    pub filename: String,
    pub kind: String,
    pub mime: String,
    pub size: i64,
    pub text: Option<PdfTextStats>,
}

#[derive(Debug, Error)]
pub enum PromptExpansionError {
    #[error("An attached file is no longer available")]
    Missing,
    #[error("text has not been extracted from PDF {0}")]
    PdfTextMissing(String),
    #[error("Attachments in this prompt exceed the configured limit")]
    TooLarge,
    #[error("Could not read an attached file")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Expand stored upload IDs into the content parts accepted by OpenRouter.
pub async fn expand_prompt(
    pool: &SqlitePool,
    data_dir: &FsPath,
    prompt: &[crate::messages::PromptMessage],
    image_support: bool,
    file_support: bool,
    max_total_bytes: u64,
) -> Result<ExpandedPrompt, PromptExpansionError> {
    let mut result = Vec::with_capacity(prompt.len());
    let mut total = 0_u64;
    let mut pdf_text = 0;
    let mut pdf_native = 0;
    let mut pdf_omitted = 0;
    for message in prompt {
        let mut parts = Vec::<Value>::new();
        if !message.content.is_empty() {
            parts.push(json!({"type":"text", "text":message.content}));
        }
        for upload_id in &message.attachment_ids {
            let upload = sqlx::query_as::<_, (String, String, String, i64, String, Option<i64>, Option<i64>, Option<i64>)>(
                "SELECT sha256, filename, mime, size, kind, text_chars, text_pages, text_empty_pages FROM uploads WHERE id = ?",
            )
            .bind(upload_id)
            .fetch_optional(pool)
            .await?
            .ok_or(PromptExpansionError::Missing)?;
            total = total.saturating_add(upload.3.max(0) as u64);
            if total > max_total_bytes.min(200 * 1024 * 1024) {
                return Err(PromptExpansionError::TooLarge);
            }
            if upload.4 == "image" && !image_support {
                parts.push(json!({"type":"text", "text":format!(
                    "[image omitted: {}. The current model can't view images]", upload.1
                )}));
                continue;
            }
            if upload.4 == "pdf" {
                let (Some(_chars), Some(pages), Some(empty_pages)) = (upload.5, upload.6, upload.7)
                else {
                    return Err(PromptExpansionError::PdfTextMissing(upload_id.clone()));
                };
                if pages > 0 && pages == empty_pages {
                    if file_support {
                        let bytes = tokio::fs::read(content_path(data_dir, &upload.0)).await?;
                        parts.push(json!({
                            "type":"document",
                            "source":{"type":"base64", "media_type":"application/pdf", "data":STANDARD.encode(bytes)},
                            "title":upload.1
                        }));
                        pdf_native += 1;
                    } else {
                        parts.push(json!({"type":"text", "text":format!(
                            "[PDF omitted: {}. No text could be extracted and the current model can't read PDFs]",
                            upload.1
                        )}));
                        pdf_omitted += 1;
                    }
                } else {
                    let text =
                        tokio::fs::read_to_string(pdf_text_path(data_dir, &upload.0)).await?;
                    let fence = text_fence(&text);
                    parts.push(json!({"type":"text", "text":format!(
                        "Attached PDF: {} ({} pages, text extracted)\n{}text\n{}\n{}",
                        upload.1, pages, fence, text, fence
                    )}));
                    pdf_text += 1;
                }
                continue;
            }
            let bytes = tokio::fs::read(content_path(data_dir, &upload.0)).await?;
            match upload.4.as_str() {
                "image" => parts.push(json!({
                    "type":"image",
                    "source":{"type":"base64", "media_type":upload.2, "data":STANDARD.encode(bytes)}
                })),
                "text" => {
                    let text = String::from_utf8_lossy(&bytes);
                    let fence = text_fence(&text);
                    let label = upload.1.rsplit('.').next().unwrap_or("");
                    parts.push(json!({"type":"text", "text":format!(
                        "Attached file: {}\n{}{}\n{}\n{}", upload.1, fence, label, text, fence
                    )}));
                }
                _ => return Err(PromptExpansionError::Missing),
            }
        }
        let content = if message.attachment_ids.is_empty() {
            Value::String(message.content.clone())
        } else {
            Value::Array(parts)
        };
        result.push(crate::openrouter::ChatMessage {
            role: message.role.clone(),
            content,
        });
    }
    Ok(ExpandedPrompt {
        messages: result,
        pdf_text,
        pdf_native,
        pdf_omitted,
    })
}

fn text_fence(text: &str) -> String {
    let fence_len = text
        .split('\n')
        .map(|line| line.chars().take_while(|c| *c == '`').count())
        .max()
        .unwrap_or(0)
        .max(2)
        + 1;
    "`".repeat(fence_len)
}

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("upload exceeds its size limit")]
    TooLarge,
    #[error("unsupported file type")]
    Unsupported,
    #[error("invalid filename")]
    Filename,
    #[error("upload not found")]
    NotFound,
    #[error("upload is already attached")]
    Attached,
    #[error("invalid upload id")]
    InvalidId,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/uploads", put(put_handler))
        .route("/api/uploads/{id}/text", put(put_text_handler))
        .route(
            "/api/uploads/{id}",
            axum::routing::get(get_handler).delete(delete_handler),
        )
        .route_layer(middleware::from_fn_with_state(state, auth::require_session))
}

async fn put_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, axum::Json<UploadRecord>), UploadApiError> {
    let filename = headers
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| decode_filename(v).ok())
        .ok_or_else(|| {
            UploadApiError::new(
                &headers,
                StatusCode::BAD_REQUEST,
                "invalid_filename",
                "Provide a URL-encoded X-Filename header",
            )
        })?;
    let limits = read_limits(&state.pool)
        .await
        .map_err(|_| UploadApiError::internal(&headers))?;
    let record = store_stream_with_limits(
        &state.pool,
        &state.config.data_dir,
        &filename,
        body.into_data_stream(),
        limits,
    )
    .await
    .map_err(|error| UploadApiError::from_upload(error, &headers))?;
    Ok((StatusCode::CREATED, axum::Json(record)))
}

async fn put_text_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<axum::Json<PdfTextStats>, UploadApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !content_type.to_ascii_lowercase().starts_with("text/plain") {
        return Err(UploadApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_pdf_text",
            "PDF text must use text/plain",
        ));
    }
    let pages = parse_pdf_count_header(&headers, "x-pdf-pages").ok_or_else(|| {
        UploadApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_pdf_text",
            "PDF page counts are invalid",
        )
    })?;
    let empty_pages = parse_pdf_count_header(&headers, "x-pdf-empty-pages").ok_or_else(|| {
        UploadApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_pdf_text",
            "PDF page counts are invalid",
        )
    })?;
    let extractor = headers
        .get("x-pdf-extractor")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(DEFAULT_PDF_EXTRACTOR);
    if extractor.is_empty()
        || extractor.len() > 64
        || !extractor
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
    {
        return Err(UploadApiError::new(
            &headers,
            StatusCode::BAD_REQUEST,
            "invalid_pdf_text",
            "PDF extractor version is invalid",
        ));
    }
    match store_pdf_text(
        &state.pool,
        &state.config.data_dir,
        &id,
        body.into_data_stream(),
        pages,
        empty_pages,
        extractor,
    )
    .await
    {
        Ok(stored) => {
            tracing::info!(
                upload = %id,
                chars = stored.stats.chars,
                pages = stored.stats.pages,
                empty_pages = stored.stats.empty_pages,
                bytes = stored.bytes,
                extractor,
                "PDF text stored"
            );
            Ok(axum::Json(stored.stats))
        }
        Err(error) => {
            let reason = match &error {
                PdfTextError::TooLarge => "too_large",
                PdfTextError::InvalidText => "not_utf8",
                PdfTextError::NotPdf => "not_pdf",
                PdfTextError::InvalidMetadata => "invalid_metadata",
                PdfTextError::NotFound => "not_found",
                PdfTextError::Io(_) | PdfTextError::Database(_) => "internal",
            };
            tracing::warn!(upload = %id, reason, "PDF text rejected");
            Err(UploadApiError::from_pdf_text(error, &headers))
        }
    }
}

fn parse_pdf_count_header(headers: &HeaderMap, name: &str) -> Option<i64> {
    let count = headers.get(name)?.to_str().ok()?.parse::<i64>().ok()?;
    (count >= 0).then_some(count)
}

#[derive(Debug, Error)]
enum PdfTextError {
    #[error("upload not found")]
    NotFound,
    #[error("upload is not a PDF")]
    NotPdf,
    #[error("PDF text exceeds its size limit")]
    TooLarge,
    #[error("PDF text must be valid UTF-8 without NUL bytes")]
    InvalidText,
    #[error("PDF page metadata is invalid")]
    InvalidMetadata,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

struct StoredPdfText {
    stats: PdfTextStats,
    bytes: u64,
}

async fn store_pdf_text<S, E>(
    pool: &SqlitePool,
    data_dir: &FsPath,
    upload_id: &str,
    mut stream: S,
    pages: i64,
    empty_pages: i64,
    extractor: &str,
) -> Result<StoredPdfText, PdfTextError>
where
    S: futures_util::Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    if pages <= 0 || empty_pages < 0 || empty_pages > pages {
        return Err(PdfTextError::InvalidMetadata);
    }
    let (kind, sha): (String, String) =
        sqlx::query_as("SELECT kind, sha256 FROM uploads WHERE id = ?")
            .bind(upload_id)
            .fetch_optional(pool)
            .await?
            .ok_or(PdfTextError::NotFound)?;
    if kind != "pdf" {
        return Err(PdfTextError::NotPdf);
    }

    let tmp_dir = data_dir.join("tmp");
    create_private_dir(&tmp_dir)?;
    let tmp_path = tmp_dir.join(Uuid::now_v7().to_string());
    let mut file = File::create(&tmp_path)?;
    let mut size = 0_u64;
    let mut utf8 = Utf8Validator::default();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                drop(file);
                let _ = fs::remove_file(&tmp_path);
                return Err(PdfTextError::Io(std::io::Error::other(
                    "request body stream failed",
                )));
            }
        };
        size = match size.checked_add(chunk.len() as u64) {
            Some(size) if size <= PDF_TEXT_MAX_BYTES => size,
            _ => {
                drop(file);
                let _ = fs::remove_file(&tmp_path);
                return Err(PdfTextError::TooLarge);
            }
        };
        if chunk.contains(&0) || !utf8.feed(&chunk) {
            drop(file);
            let _ = fs::remove_file(&tmp_path);
            return Err(PdfTextError::InvalidText);
        }
        if let Err(error) = file.write_all(&chunk) {
            drop(file);
            let _ = fs::remove_file(&tmp_path);
            return Err(PdfTextError::Io(error));
        }
    }
    if !utf8.finish() {
        drop(file);
        let _ = fs::remove_file(&tmp_path);
        return Err(PdfTextError::InvalidText);
    }
    if let Err(error) = file.sync_all() {
        drop(file);
        let _ = fs::remove_file(&tmp_path);
        return Err(PdfTextError::Io(error));
    }
    drop(file);

    let stats = PdfTextStats {
        chars: i64::try_from(utf8.chars).map_err(|_| PdfTextError::InvalidMetadata)?,
        pages,
        empty_pages,
    };
    let target = pdf_text_path(data_dir, &sha);
    let parent = target
        .parent()
        .ok_or_else(|| PdfTextError::Io(std::io::Error::other("invalid text path")))?;
    create_private_dir(&data_dir.join("uploads"))?;
    create_private_dir(parent)?;
    if let Err(error) = fs::rename(&tmp_path, &target) {
        let _ = fs::remove_file(&tmp_path);
        return Err(PdfTextError::Io(error));
    }
    sqlx::query(
        "UPDATE uploads SET text_chars = ?, text_pages = ?, text_empty_pages = ?, text_extractor = ? WHERE sha256 = ?",
    )
    .bind(stats.chars)
    .bind(stats.pages)
    .bind(stats.empty_pages)
    .bind(extractor)
    .bind(&sha)
    .execute(pool)
    .await?;
    Ok(StoredPdfText { stats, bytes: size })
}

async fn get_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, UploadApiError> {
    let row = sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT sha256, mime, kind, size FROM uploads WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| UploadApiError::internal(&headers))?
    .ok_or_else(|| {
        UploadApiError::new(
            &headers,
            StatusCode::NOT_FOUND,
            "upload_not_found",
            "Upload not found",
        )
    })?;
    let path = content_path(&state.config.data_dir, &row.0);
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| UploadApiError::internal(&headers))?;
    let disposition = if row.2 == "pdf" || row.2 == "text" {
        "inline"
    } else {
        "attachment"
    };
    let stream = async_stream::stream! {
        loop {
            let mut chunk = vec![0; 32 * 1024];
            match tokio::io::AsyncReadExt::read(&mut file, &mut chunk).await {
                Ok(0) => break,
                Ok(count) => {
                    chunk.truncate(count);
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(chunk));
                }
                Err(error) => {
                    yield Err(error);
                    break;
                }
            }
        }
    };
    let mut response = (StatusCode::OK, Body::from_stream(stream)).into_response();
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&row.1)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&row.3.to_string()).expect("integer header"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static(disposition),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    Ok(response)
}

async fn delete_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, UploadApiError> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| UploadApiError::internal(&headers))?;
    let hash: Option<String> = sqlx::query_scalar("SELECT sha256 FROM uploads WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| UploadApiError::internal(&headers))?;
    let Some(hash) = hash else {
        return Err(UploadApiError::new(
            &headers,
            StatusCode::NOT_FOUND,
            "upload_not_found",
            "Upload not found",
        ));
    };
    let attached: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM message_attachments WHERE upload_id = ?)")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| UploadApiError::internal(&headers))?;
    if attached {
        return Err(UploadApiError::new(
            &headers,
            StatusCode::CONFLICT,
            "upload_attached",
            "Attached uploads cannot be deleted",
        ));
    }
    sqlx::query("DELETE FROM uploads WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|_| UploadApiError::internal(&headers))?;
    let still_used: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM uploads WHERE sha256 = ?)")
            .bind(&hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| UploadApiError::internal(&headers))?;
    tx.commit()
        .await
        .map_err(|_| UploadApiError::internal(&headers))?;
    if !still_used {
        let _ = fs::remove_file(content_path(&state.config.data_dir, &hash));
        let _ = fs::remove_file(pdf_text_path(&state.config.data_dir, &hash));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Stream a request body to a private temporary file, enforce the per-kind limit as
/// soon as its magic bytes identify the kind, then atomically publish by digest.
pub async fn store_stream<S, E>(
    pool: &SqlitePool,
    data_dir: &FsPath,
    filename: &str,
    stream: S,
) -> Result<UploadRecord, UploadError>
where
    S: futures_util::Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    store_stream_with_limits(
        pool,
        data_dir,
        filename,
        stream,
        crate::settings::UploadLimits::default(),
    )
    .await
}

async fn store_stream_with_limits<S, E>(
    pool: &SqlitePool,
    data_dir: &FsPath,
    filename: &str,
    mut stream: S,
    limits: crate::settings::UploadLimits,
) -> Result<UploadRecord, UploadError>
where
    S: futures_util::Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    let filename = sanitize_filename(filename)?;
    let tmp_dir = data_dir.join("tmp");
    create_private_dir(&tmp_dir)?;
    let tmp_path = tmp_dir.join(Uuid::now_v7().to_string());
    let mut file = File::create(&tmp_path)?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut prefix = Vec::with_capacity(512);
    let mut kind = None;
    let mut utf8 = Utf8Validator::default();
    let mut svg_probe = Vec::new();
    let svg_probe_limit = limits.text_bytes.min(5 * 1024 * 1024) as usize;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|_| UploadError::Io(std::io::Error::other("request body stream failed")))?;
        size = size
            .checked_add(chunk.len() as u64)
            .ok_or(UploadError::TooLarge)?;
        if kind.is_none() {
            let to_add = (512usize.saturating_sub(prefix.len())).min(chunk.len());
            prefix.extend_from_slice(&chunk[..to_add]);
            kind = classify_prefix(&prefix);
        }
        if let Some(detected) = kind {
            if size > detected.limit(&limits) {
                drop(file);
                let _ = fs::remove_file(&tmp_path);
                return Err(UploadError::TooLarge);
            }
            if detected == UploadKind::Text {
                if chunk.contains(&0) || !utf8.feed(&chunk) {
                    drop(file);
                    let _ = fs::remove_file(&tmp_path);
                    return Err(UploadError::Unsupported);
                }
                if svg_probe.len() < svg_probe_limit {
                    let n = (svg_probe_limit - svg_probe.len()).min(chunk.len());
                    svg_probe.extend_from_slice(&chunk[..n]);
                }
            }
        } else if size > 100 * 1024 * 1024 {
            // Any supported kind is below this compiled ceiling.
            drop(file);
            let _ = fs::remove_file(&tmp_path);
            return Err(UploadError::TooLarge);
        }
        hasher.update(&chunk);
        file.write_all(&chunk)?;
    }
    file.sync_all()?;
    drop(file);
    let kind = kind
        .or_else(|| classify_prefix(&prefix))
        .ok_or(UploadError::Unsupported)?;
    if kind == UploadKind::Text {
        if !utf8.finish()
            || looks_like_svg(&svg_probe)
            || filename.to_ascii_lowercase().ends_with(".svg")
        {
            let _ = fs::remove_file(&tmp_path);
            return Err(UploadError::Unsupported);
        }
    }
    if size > kind.limit(&limits) {
        let _ = fs::remove_file(&tmp_path);
        return Err(UploadError::TooLarge);
    }
    let digest = hex::encode(hasher.finalize());
    let target = content_path(data_dir, &digest);
    let parent = target
        .parent()
        .ok_or_else(|| UploadError::Io(std::io::Error::other("invalid content path")))?;
    create_private_dir(&data_dir.join("uploads"))?;
    create_private_dir(parent)?;
    // hard_link is an atomic create-if-absent operation, unlike rename on Unix which
    // would replace a concurrently published object. Both paths are under DATA_DIR.
    match fs::hard_link(&tmp_path, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            return Err(UploadError::Io(e));
        }
    }
    fs::remove_file(&tmp_path)?;
    let id = Uuid::now_v7().to_string();
    let mime = sniffed_mime(kind, &filename, &prefix);
    let created_at = now_ms();
    if let Err(error) = sqlx::query("INSERT INTO uploads (id, sha256, filename, mime, kind, size, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&id).bind(&digest).bind(&filename).bind(&mime).bind(kind.as_str()).bind(size as i64).bind(created_at).execute(pool).await {
        let referenced: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM uploads WHERE sha256 = ?)")
            .bind(&digest).fetch_one(pool).await.unwrap_or(true);
        if !referenced { let _ = fs::remove_file(&target); }
        return Err(UploadError::Database(error));
    }
    let text_stats = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>)>(
        "SELECT text_chars, text_pages, text_empty_pages FROM uploads WHERE sha256 = ? LIMIT 1",
    )
    .bind(&digest)
    .fetch_one(pool)
    .await?;
    let text = match text_stats {
        (Some(chars), Some(pages), Some(empty_pages)) => Some(PdfTextStats {
            chars,
            pages,
            empty_pages,
        }),
        _ => None,
    };
    Ok(UploadRecord {
        id,
        filename,
        kind: kind.as_str().to_owned(),
        mime,
        size: size as i64,
        text,
    })
}

async fn read_limits(pool: &SqlitePool) -> Result<crate::settings::UploadLimits, sqlx::Error> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'upload_limits'")
            .fetch_optional(pool)
            .await?;
    Ok(value
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

/// Remove unattached uploads older than 24 hours, then remove content files no longer
/// referenced by any upload row. Also clears temporary files older than 24 hours.
pub async fn gc(pool: &SqlitePool, data_dir: &FsPath, now: i64) -> Result<usize, UploadError> {
    let cutoff = now.saturating_sub(ORPHAN_MAX_AGE_MS);
    let hashes: Vec<String> = sqlx::query_scalar("SELECT DISTINCT sha256 FROM uploads WHERE created_at < ? AND NOT EXISTS (SELECT 1 FROM message_attachments WHERE message_attachments.upload_id = uploads.id)")
        .bind(cutoff).fetch_all(pool).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM uploads WHERE created_at < ? AND NOT EXISTS (SELECT 1 FROM message_attachments WHERE message_attachments.upload_id = uploads.id)")
        .bind(cutoff).execute(&mut *tx).await?;
    tx.commit().await?;
    let mut removed = 0;
    for hash in hashes {
        let used: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM uploads WHERE sha256 = ?)")
                .bind(&hash)
                .fetch_one(pool)
                .await?;
        if !used {
            if fs::remove_file(content_path(data_dir, &hash)).is_ok() {
                removed += 1;
            }
            if fs::remove_file(pdf_text_path(data_dir, &hash)).is_ok() {
                removed += 1;
            }
        }
    }
    // Recover from a crash after publishing the content file but before inserting
    // its upload metadata row. Only collect old content with no live DB reference.
    let content_dir = data_dir.join("uploads");
    if let Ok(prefixes) = fs::read_dir(content_dir) {
        for prefix in prefixes.flatten() {
            if !prefix.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let Ok(files) = fs::read_dir(prefix.path()) else {
                continue;
            };
            for file in files.flatten() {
                let name = file.file_name().to_string_lossy().into_owned();
                let (hash, is_text) = if let Some(hash) = name.strip_suffix(".txt") {
                    (hash.to_owned(), true)
                } else {
                    (name, false)
                };
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    continue;
                }
                let old = file
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .is_some_and(|time| {
                        now.saturating_sub(time.as_millis() as i64) > ORPHAN_MAX_AGE_MS
                    });
                if !old {
                    continue;
                }
                let used: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM uploads WHERE sha256 = ?)")
                        .bind(&hash)
                        .fetch_one(pool)
                        .await?;
                if !used {
                    if is_text {
                        if fs::remove_file(file.path()).is_ok() {
                            removed += 1;
                        }
                    } else {
                        if fs::remove_file(file.path()).is_ok() {
                            removed += 1;
                        }
                        if fs::remove_file(pdf_text_path(data_dir, &hash)).is_ok() {
                            removed += 1;
                        }
                    }
                }
            }
        }
    }
    let tmp_dir = data_dir.join("tmp");
    if let Ok(entries) = fs::read_dir(tmp_dir) {
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .is_some_and(|time| now.saturating_sub(time.as_millis() as i64) > TMP_MAX_AGE_MS);
            if old && fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

pub async fn gc_loop(pool: SqlitePool, data_dir: PathBuf) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
    loop {
        interval.tick().await;
        match gc(&pool, &data_dir, now_ms()).await {
            Ok(removed) if removed > 0 => tracing::info!(removed, "cleaned stale uploads"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "upload garbage collection failed"),
        }
    }
}

fn content_path(data_dir: &FsPath, digest: &str) -> PathBuf {
    data_dir.join("uploads").join(&digest[..2]).join(digest)
}
fn pdf_text_path(data_dir: &FsPath, digest: &str) -> PathBuf {
    data_dir
        .join("uploads")
        .join(&digest[..2])
        .join(format!("{digest}.txt"))
}

fn create_private_dir(path: &FsPath) -> std::io::Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => {}
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o700)) {
            tracing::warn!(path = %path.display(), %error,
                "could not secure upload directory; relying on mount-managed permissions");
        }
    }
    Ok(())
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn sanitize_filename(name: &str) -> Result<String, UploadError> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if name.is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
        return Err(UploadError::Filename);
    }
    Ok(name.to_owned())
}
fn decode_filename(value: &str) -> Result<String, ()> {
    let input = value.as_bytes();
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            if i + 2 >= input.len() {
                return Err(());
            }
            let hi = (input[i + 1] as char).to_digit(16).ok_or(())?;
            let lo = (input[i + 2] as char).to_digit(16).ok_or(())?;
            out.push(((hi << 4) | lo) as u8);
            i += 3;
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

fn classify_prefix(b: &[u8]) -> Option<UploadKind> {
    let signatures: [(&[u8], UploadKind); 4] = [
        (b"\x89PNG\r\n\x1a\n", UploadKind::Image),
        (b"\xff\xd8\xff", UploadKind::Image),
        (b"GIF87a", UploadKind::Image),
        (b"GIF89a", UploadKind::Image),
    ];
    if signatures.iter().any(|(sig, _)| b.starts_with(sig)) {
        return Some(UploadKind::Image);
    }
    if b.starts_with(b"RIFF") && b.len() >= 12 && &b[8..12] == b"WEBP" {
        return Some(UploadKind::Image);
    }
    if b.starts_with(b"%PDF-") {
        return Some(UploadKind::Pdf);
    }
    if b.is_empty() {
        return None;
    }
    // Keep waiting while a short initial buffer could still become a magic signature.
    let magic: [&[u8]; 6] = [
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff",
        b"GIF87a",
        b"GIF89a",
        b"RIFF",
        b"%PDF-",
    ];
    if magic.iter().any(|sig| sig.starts_with(b)) && b.len() < 12 {
        return None;
    }
    Some(UploadKind::Text)
}
fn sniffed_mime(kind: UploadKind, filename: &str, prefix: &[u8]) -> String {
    match kind {
        UploadKind::Image => if prefix.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if prefix.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else if prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a") {
            "image/gif"
        } else if prefix.starts_with(b"RIFF") && prefix.len() >= 12 && &prefix[8..12] == b"WEBP" {
            "image/webp"
        } else {
            "application/octet-stream"
        }
        .to_owned(),
        UploadKind::Pdf => "application/pdf".to_owned(),
        UploadKind::Text => classify_extension(filename)
            .filter(|m| m.starts_with("text/") || m == "application/json" || m == "application/xml")
            .unwrap_or_else(|| "text/plain".to_owned()),
    }
}
fn classify_extension(filename: &str) -> Option<String> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    Some(
        match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "txt" => "text/plain",
            "md" | "markdown" => "text/markdown",
            "csv" => "text/csv",
            "json" => "application/json",
            "xml" => "application/xml",
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" => "text/javascript",
            "rs" => "text/plain",
            "py" => "text/x-python",
            "toml" => "application/toml",
            "yaml" | "yml" => "application/yaml",
            "svg" => "image/svg+xml",
            _ => return None,
        }
        .to_owned(),
    )
}
fn looks_like_svg(b: &[u8]) -> bool {
    let text = String::from_utf8_lossy(b)
        .trim_start_matches('\u{feff}')
        .trim_start()
        .to_ascii_lowercase();
    text.contains("<svg")
}
#[derive(Default)]
struct Utf8Validator {
    pending: Vec<u8>,
    chars: usize,
}
impl Utf8Validator {
    fn feed(&mut self, bytes: &[u8]) -> bool {
        self.pending.extend_from_slice(bytes);
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                self.chars += text.encode_utf16().count();
                self.pending.clear();
                true
            }
            Err(e) if e.error_len().is_none() => {
                let valid = std::str::from_utf8(&self.pending[..e.valid_up_to()])
                    .expect("valid prefix was checked by from_utf8");
                self.chars += valid.encode_utf16().count();
                let remainder = self.pending.split_off(e.valid_up_to());
                self.pending = remainder;
                true
            }
            Err(_) => false,
        }
    }
    fn finish(&self) -> bool {
        self.pending.is_empty()
    }
}

#[derive(Debug)]
struct UploadApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: String,
}
impl UploadApiError {
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
    fn internal(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred",
        )
    }
    fn from_upload(error: UploadError, headers: &HeaderMap) -> Self {
        match error {
            UploadError::TooLarge => Self::new(
                headers,
                StatusCode::PAYLOAD_TOO_LARGE,
                "upload_too_large",
                "The file exceeds its size limit",
            ),
            UploadError::Unsupported => Self::new(
                headers,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_file_type",
                "This file type is not supported",
            ),
            UploadError::Filename => Self::new(
                headers,
                StatusCode::BAD_REQUEST,
                "invalid_filename",
                "The filename is invalid",
            ),
            _ => Self::internal(headers),
        }
    }
    fn from_pdf_text(error: PdfTextError, headers: &HeaderMap) -> Self {
        match error {
            PdfTextError::NotFound => Self::new(
                headers,
                StatusCode::NOT_FOUND,
                "upload_not_found",
                "Upload not found",
            ),
            PdfTextError::NotPdf => Self::new(
                headers,
                StatusCode::CONFLICT,
                "upload_not_pdf",
                "Extracted PDF text can only be stored for PDF uploads",
            ),
            PdfTextError::TooLarge => Self::new(
                headers,
                StatusCode::PAYLOAD_TOO_LARGE,
                "pdf_text_too_large",
                "Extracted PDF text exceeds 16 MB",
            ),
            PdfTextError::InvalidText | PdfTextError::InvalidMetadata => Self::new(
                headers,
                StatusCode::BAD_REQUEST,
                "invalid_pdf_text",
                "Extracted PDF text or page metadata is invalid",
            ),
            PdfTextError::Io(_) | PdfTextError::Database(_) => Self::internal(headers),
        }
    }
}
impl IntoResponse for UploadApiError {
    fn into_response(self) -> Response {
        auth::api_error(&self.request_id, self.status, self.code, self.message, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::Arc,
    };

    async fn setup() -> (SqlitePool, PathBuf) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrate(&pool).await.unwrap();
        let root = std::env::temp_dir().join(format!("sprinter-upload-test-{}", Uuid::now_v7()));
        fs::create_dir_all(&root).unwrap();
        (pool, root)
    }

    fn app_state(pool: SqlitePool, root: &FsPath) -> AppState {
        AppState::new(
            pool,
            Arc::new(crate::config::Config {
                master_password: None,
                data_dir: root.to_owned(),
                bind_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                trusted_proxies: vec![],
                worker_threads: 1,
                log_filter: "off".to_owned(),
                log_keep_days: 30,
                insecure_cookies: true,
                openrouter_base_url: "http://localhost".to_owned(),
            }),
        )
    }

    fn chunks(
        bytes: Vec<u8>,
    ) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Unpin {
        stream::iter(
            bytes
                .chunks(4096)
                .map(|c| Ok(Bytes::copy_from_slice(c)))
                .collect::<Vec<_>>(),
        )
    }

    #[cfg(unix)]
    #[test]
    fn private_upload_directories_are_searchable_and_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root =
            std::env::temp_dir().join(format!("sprinter-private-dir-test-{}", Uuid::now_v7()));
        create_private_dir(&root).unwrap();
        let uploads = root.join("uploads");
        let shard = uploads.join("ab");
        create_private_dir(&uploads).unwrap();
        create_private_dir(&shard).unwrap();

        assert_eq!(
            fs::metadata(&uploads).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&shard).unwrap().permissions().mode() & 0o777,
            0o700
        );
        File::create(shard.join("content.txt")).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sniffing_uses_magic_bytes_and_same_content_deduplicates() {
        let (pool, root) = setup().await;
        let png = b"\x89PNG\r\n\x1a\nbody".to_vec();
        let first = store_stream(&pool, &root, "misnamed.txt", chunks(png.clone()))
            .await
            .unwrap();
        let second = store_stream(&pool, &root, "renamed.png", chunks(png.clone()))
            .await
            .unwrap();
        assert_eq!(first.kind, "image");
        assert_eq!(first.mime, "image/png");
        assert_ne!(first.id, second.id);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT sha256) FROM uploads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            fs::read(content_path(&root, &hex::encode(Sha256::digest(&png)))).unwrap(),
            png
        );
        pool.close().await;
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn per_kind_limit_is_enforced_and_svg_is_rejected() {
        let (pool, root) = setup().await;
        let too_big =
            vec![b'a'; (crate::settings::UploadLimits::default().text_bytes + 1) as usize];
        assert!(matches!(
            store_stream(&pool, &root, "large.txt", chunks(too_big)).await,
            Err(UploadError::TooLarge)
        ));
        assert!(matches!(
            store_stream(
                &pool,
                &root,
                "drawing.svg",
                chunks(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".to_vec())
            )
            .await,
            Err(UploadError::Unsupported)
        ));
        assert!(matches!(
            store_stream(
                &pool,
                &root,
                "renamed.txt",
                chunks(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".to_vec())
            )
            .await,
            Err(UploadError::Unsupported)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM uploads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        pool.close().await;
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn gc_removes_unattached_upload_rows_and_content() {
        let (pool, root) = setup().await;
        let rec = store_stream(
            &pool,
            &root,
            "report.pdf",
            chunks(b"%PDF-1.7 fake".to_vec()),
        )
        .await
        .unwrap();
        let hash: String = sqlx::query_scalar("SELECT sha256 FROM uploads WHERE id = ?")
            .bind(&rec.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let path = content_path(&root, &hash);
        let text_path = pdf_text_path(&root, &hash);
        store_pdf_text(
            &pool,
            &root,
            &rec.id,
            chunks(b"extracted".to_vec()),
            1,
            0,
            "pdfjs-test",
        )
        .await
        .unwrap();
        assert!(path.exists());
        assert!(text_path.exists());
        sqlx::query("UPDATE uploads SET created_at = 1 WHERE id = ?")
            .bind(&rec.id)
            .execute(&pool)
            .await
            .unwrap();
        gc(&pool, &root, TMP_MAX_AGE_MS + 2).await.unwrap();
        assert!(!path.exists());
        assert!(!text_path.exists());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM uploads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        pool.close().await;
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn get_sets_sandbox_headers_and_delete_removes_unattached_upload() {
        let (pool, root) = setup().await;
        let rec = store_stream(&pool, &root, "note.txt", chunks(b"hello".to_vec()))
            .await
            .unwrap();
        let state = app_state(pool.clone(), &root);
        let response = get_handler(State(state.clone()), Path(rec.id.clone()), HeaderMap::new())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/plain");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "sandbox"
        );
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, max-age=31536000, immutable"
        );
        assert_eq!(response.headers()[header::CONTENT_DISPOSITION], "inline");
        assert_eq!(
            delete_handler(State(state), Path(rec.id), HeaderMap::new())
                .await
                .unwrap(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM uploads")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        pool.close().await;
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn prompt_expansion_uses_extracted_text_and_gates_scan_fallback_by_model() {
        let (pool, root) = setup().await;
        let image = store_stream(
            &pool,
            &root,
            "photo.webp",
            chunks(b"RIFF\x00\x00\x00\x00WEBPpicture".to_vec()),
        )
        .await
        .unwrap();
        let pdf = store_stream(&pool, &root, "scan.pdf", chunks(b"%PDF-1.7 fake".to_vec()))
            .await
            .unwrap();
        let pdf_sha: String = sqlx::query_scalar("SELECT sha256 FROM uploads WHERE id = ?")
            .bind(&pdf.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let extracted = "## Page 1\nExtracted report content";
        fs::write(pdf_text_path(&root, &pdf_sha), extracted).unwrap();
        sqlx::query(
            "UPDATE uploads SET text_chars = ?, text_pages = 1, text_empty_pages = 0 WHERE id = ?",
        )
        .bind(extracted.len() as i64)
        .bind(&pdf.id)
        .execute(&pool)
        .await
        .unwrap();
        let text = store_stream(&pool, &root, "notes.md", chunks(b"hello `world`".to_vec()))
            .await
            .unwrap();
        let prompt = [crate::messages::PromptMessage {
            role: "user".into(),
            content: "Summarize these".into(),
            attachment_ids: vec![image.id.clone(), pdf.id.clone(), text.id.clone()],
        }];
        let expanded = expand_prompt(&pool, &root, &prompt, true, false, 1024 * 1024)
            .await
            .unwrap();
        let parts = expanded.messages[0].content.as_array().unwrap();
        assert_eq!(parts[0]["text"], "Summarize these");
        assert_eq!(parts[1]["type"], "image");
        assert_eq!(parts[1]["source"]["type"], "base64");
        assert_eq!(parts[1]["source"]["media_type"], "image/webp");
        assert!(parts[1]["source"]["data"].as_str().is_some());
        assert_eq!(parts[2]["type"], "text");
        assert!(
            parts[2]["text"]
                .as_str()
                .unwrap()
                .contains("Attached PDF: scan.pdf (1 pages, text extracted)")
        );
        assert!(
            parts[2]["text"]
                .as_str()
                .unwrap()
                .contains("```text\n## Page 1\nExtracted report content\n```")
        );
        assert!(
            parts[3]["text"]
                .as_str()
                .unwrap()
                .contains("```md\nhello `world`\n```")
        );
        assert_eq!(expanded.pdf_text, 1);
        assert_eq!(expanded.pdf_native, 0);

        let without_vision = expand_prompt(&pool, &root, &prompt, false, false, 1024 * 1024)
            .await
            .unwrap();
        let parts = without_vision.messages[0].content.as_array().unwrap();
        assert!(
            parts[1]["text"]
                .as_str()
                .unwrap()
                .contains("image omitted: photo.webp")
        );
        assert_eq!(parts[2]["type"], "text");

        let scan = store_stream(
            &pool,
            &root,
            "empty-scan.pdf",
            chunks(b"%PDF-1.7 scan".to_vec()),
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE uploads SET text_chars = 0, text_pages = 2, text_empty_pages = 2 WHERE id = ?",
        )
        .bind(&scan.id)
        .execute(&pool)
        .await
        .unwrap();
        let scan_prompt = [crate::messages::PromptMessage {
            role: "user".into(),
            content: "read the scan".into(),
            attachment_ids: vec![scan.id],
        }];
        let native = expand_prompt(&pool, &root, &scan_prompt, true, true, 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(native.messages[0].content[1]["type"], "document");
        assert_eq!(native.messages[0].content[1]["source"]["type"], "base64");
        assert_eq!(
            native.messages[0].content[1]["source"]["media_type"],
            "application/pdf"
        );
        assert_eq!(native.messages[0].content[1]["title"], "empty-scan.pdf");
        assert!(native.needs_native_pdf_plugin());
        let omitted = expand_prompt(&pool, &root, &scan_prompt, true, false, 1024 * 1024)
            .await
            .unwrap();
        assert!(
            omitted.messages[0].content[1]["text"]
                .as_str()
                .unwrap()
                .contains("PDF omitted: empty-scan.pdf")
        );
        assert_eq!(omitted.pdf_omitted, 1);

        assert!(matches!(
            expand_prompt(&pool, &root, &prompt, true, false, 2).await,
            Err(PromptExpansionError::TooLarge)
        ));
        pool.close().await;
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn pdf_text_storage_is_streamed_shared_and_utf8_validated() {
        let (pool, root) = setup().await;
        let pdf = store_stream(
            &pool,
            &root,
            "report.pdf",
            chunks(b"%PDF-1.7 fake".to_vec()),
        )
        .await
        .unwrap();
        let duplicate = store_stream(&pool, &root, "copy.pdf", chunks(b"%PDF-1.7 fake".to_vec()))
            .await
            .unwrap();
        let stream = stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(b"abc \xf0\x9f")),
            Ok(Bytes::from_static(b"\x8c\x8d")),
        ]);
        let stored = store_pdf_text(&pool, &root, &pdf.id, stream, 2, 1, "pdfjs-test")
            .await
            .unwrap();
        assert_eq!(stored.stats.chars, 6);
        assert_eq!(stored.bytes, 8);
        assert_eq!(
            fs::read_to_string(pdf_text_path(
                &root,
                &hex::encode(Sha256::digest(b"%PDF-1.7 fake"))
            ))
            .unwrap(),
            "abc 🌍"
        );
        let duplicate_stats = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, Option<String>)>(
            "SELECT text_chars, text_pages, text_empty_pages, text_extractor FROM uploads WHERE id = ?",
        )
        .bind(&duplicate.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            duplicate_stats,
            (Some(6), Some(2), Some(1), Some("pdfjs-test".into()))
        );

        let invalid = store_pdf_text(
            &pool,
            &root,
            &pdf.id,
            chunks(b"bad\0text".to_vec()),
            1,
            0,
            "pdfjs-test",
        )
        .await;
        assert!(matches!(invalid, Err(PdfTextError::InvalidText)));
        let scan_prompt = [crate::messages::PromptMessage {
            role: "user".into(),
            content: "read this".into(),
            attachment_ids: vec!["old-pdf".into()],
        }];
        sqlx::query("INSERT INTO uploads(id, sha256, filename, mime, kind, size, created_at) VALUES('old-pdf', 'old-pdf-hash', 'old.pdf', 'application/pdf', 'pdf', 100, 1)")
            .execute(&pool).await.unwrap();
        assert!(matches!(
            expand_prompt(&pool, &root, &scan_prompt, true, false, 1024).await,
            Err(PromptExpansionError::PdfTextMissing(id)) if id == "old-pdf"
        ));
        let prompt = [crate::messages::PromptMessage {
            role: "user".into(),
            content: "read this".into(),
            attachment_ids: vec![pdf.id.clone()],
        }];
        let expanded = expand_prompt(&pool, &root, &prompt, true, false, 1024)
            .await
            .unwrap();
        assert_eq!(expanded.pdf_text, 1);
        assert!(
            expanded.messages[0].content[1]["text"]
                .as_str()
                .unwrap()
                .contains("abc 🌍")
        );
        pool.close().await;
        fs::remove_dir_all(root).unwrap();
    }
}
