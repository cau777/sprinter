use crate::{auth, chats, state::AppState};
use axum::{
    Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashMap;

#[derive(Deserialize)]
struct ExportQuery {
    format: Option<String>,
}

#[derive(Serialize)]
struct AttachmentMetadata {
    upload_id: String,
    position: i64,
    filename: String,
    mime: String,
    kind: String,
    size: i64,
    text_chars: Option<i64>,
    text_pages: Option<i64>,
    text_empty_pages: Option<i64>,
}

#[derive(Serialize)]
struct JsonExport<'a> {
    chat: JsonChat<'a>,
    messages: Vec<JsonMessage<'a>>,
}

#[derive(Serialize)]
struct JsonChat<'a> {
    id: &'a str,
    title: &'a Option<String>,
    title_source: &'a str,
    model: &'a str,
    tools: &'a [String],
    current_leaf_id: &'a Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize)]
struct JsonMessage<'a> {
    id: &'a str,
    chat_id: &'a str,
    parent_id: &'a Option<String>,
    role: &'a str,
    content: &'a str,
    status: &'a str,
    error: &'a Option<String>,
    model: &'a Option<String>,
    generation_id: &'a Option<String>,
    finish_reason: &'a Option<String>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    cost: Option<f64>,
    tools: Option<Vec<String>>,
    citations: Option<Vec<crate::tools::Citation>>,
    tool_steps: Option<Vec<crate::tools::ToolStep>>,
    web_search_requests: Option<i64>,
    tool_cost: Option<f64>,
    tool_fallback: bool,
    created_at: i64,
    updated_at: i64,
    attachments: &'a [AttachmentMetadata],
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/chats/{id}/export", get(export_handler))
        .route_layer(middleware::from_fn_with_state(state, auth::require_session))
}

async fn export_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ExportQuery>,
    headers: HeaderMap,
) -> Result<Response, ExportError> {
    let format = query.format.as_deref().unwrap_or("md");
    if format != "md" && format != "json" {
        return Err(ExportError::bad_request(&headers));
    }
    let Some(chat) = chats::get_chat_detail(&state.pool, &id)
        .await
        .map_err(|_| ExportError::internal(&headers))?
    else {
        return Err(ExportError::not_found(&headers));
    };
    let mut attachment_map = HashMap::<String, Vec<AttachmentMetadata>>::new();
    for row in sqlx::query_as::<_, AttachmentRow>(
        "SELECT ma.message_id, u.id AS upload_id, ma.position, u.filename, u.mime, u.kind, u.size, u.text_chars, u.text_pages, u.text_empty_pages \
         FROM message_attachments ma JOIN uploads u ON u.id = ma.upload_id \
         JOIN messages m ON m.id = ma.message_id WHERE m.chat_id = ? ORDER BY ma.message_id, ma.position",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await
    .map_err(|_| ExportError::internal(&headers))?
    {
        // The first field is selected specifically to group the metadata by message.
        let message_id = row.message_id.clone();
        attachment_map.entry(message_id).or_default().push(row.into_metadata());
    }

    let content = if format == "md" {
        markdown(&chat, &attachment_map)
    } else {
        json_export(&chat, &attachment_map).map_err(|_| ExportError::internal(&headers))?
    };
    let filename = format!("{}-{}.{}", slug(&chat.title), utc_date(now_ms()), format);
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
        .expect("slug and date are valid header characters");
    tracing::info!(chat = %id, format, bytes = content.len(), "exported");
    let mut response = Response::new(Body::from(content));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(if format == "md" {
            "text/markdown; charset=utf-8"
        } else {
            "application/json; charset=utf-8"
        }),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_DISPOSITION, disposition);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn json_export(
    chat: &chats::ChatDetail,
    attachments: &HashMap<String, Vec<AttachmentMetadata>>,
) -> Result<String, serde_json::Error> {
    let messages = chat
        .messages
        .iter()
        .map(|message| JsonMessage {
            id: &message.id,
            chat_id: &message.chat_id,
            parent_id: &message.parent_id,
            role: &message.role,
            content: &message.content,
            status: &message.status,
            error: &message.error,
            model: &message.model,
            generation_id: &message.generation_id,
            finish_reason: &message.finish_reason,
            prompt_tokens: message.prompt_tokens,
            completion_tokens: message.completion_tokens,
            reasoning_tokens: message.reasoning_tokens,
            cost: message.cost,
            tools: decode_column(message.tools.as_deref()),
            citations: decode_column(message.citations.as_deref()),
            tool_steps: decode_column(message.tool_steps.as_deref()),
            web_search_requests: message.web_search_requests,
            tool_cost: message.tool_cost,
            tool_fallback: message.tool_fallback,
            created_at: message.created_at,
            updated_at: message.updated_at,
            attachments: attachments
                .get(&message.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        })
        .collect();
    serde_json::to_string_pretty(&JsonExport {
        chat: JsonChat {
            id: &chat.id,
            title: &chat.title,
            title_source: &chat.title_source,
            model: &chat.model,
            tools: &chat.tools,
            current_leaf_id: &chat.current_leaf_id,
            created_at: chat.created_at,
            updated_at: chat.updated_at,
        },
        messages,
    })
}

// SQLx's row derive needs the grouping key, but it is not part of exported metadata.
#[derive(FromRow)]
struct AttachmentRow {
    message_id: String,
    upload_id: String,
    position: i64,
    filename: String,
    mime: String,
    kind: String,
    size: i64,
    text_chars: Option<i64>,
    text_pages: Option<i64>,
    text_empty_pages: Option<i64>,
}

impl AttachmentRow {
    fn into_metadata(self) -> AttachmentMetadata {
        AttachmentMetadata {
            upload_id: self.upload_id,
            position: self.position,
            filename: self.filename,
            mime: self.mime,
            kind: self.kind,
            size: self.size,
            text_chars: self.text_chars,
            text_pages: self.text_pages,
            text_empty_pages: self.text_empty_pages,
        }
    }
}

fn markdown(
    chat: &chats::ChatDetail,
    attachments: &HashMap<String, Vec<AttachmentMetadata>>,
) -> String {
    let by_id = chat
        .messages
        .iter()
        .map(|message| (message.id.as_str(), message))
        .collect::<HashMap<_, _>>();
    let mut path = Vec::new();
    let mut current = chat.current_leaf_id.as_deref();
    while let Some(id) = current {
        let Some(message) = by_id.get(id) else { break };
        path.push(*message);
        current = message.parent_id.as_deref();
    }
    path.reverse();

    let mut output = format!("# {}\n\n", chat.title.as_deref().unwrap_or("Untitled chat"));
    for message in path {
        output.push_str(&format!("## {}", title_case(&message.role)));
        if let Some(model) = &message.model {
            output.push_str(&format!(" · {model}"));
        }
        output.push_str(&format!(" · {}\n\n", iso_utc(message.created_at)));
        append_content_with_bash_steps(&mut output, message);
        output.push('\n');
        if let Some(items) = attachments.get(&message.id) {
            for attachment in items {
                output.push_str(&format!(
                    "\n_Attachment: {} ({}, {})_\n",
                    attachment.filename, attachment.mime, attachment.size
                ));
            }
        }
        if let Some(citations) =
            decode_column::<Vec<crate::tools::Citation>>(message.citations.as_deref())
                .filter(|citations| !citations.is_empty())
        {
            output.push_str("\nSources:\n");
            for citation in citations {
                output.push_str(&format!(
                    "- [{}](<{}>) · {}\n",
                    markdown_escape(&citation.title),
                    citation.url,
                    citation_domain(&citation.url)
                ));
            }
        }
        output.push('\n');
    }
    output
}

fn append_content_with_bash_steps(output: &mut String, message: &crate::messages::MessageRecord) {
    let steps = decode_column::<Vec<crate::tools::ToolStep>>(message.tool_steps.as_deref())
        .unwrap_or_default()
        .into_iter()
        .filter(|step| step.tool == crate::tools::BASH)
        .collect::<Vec<_>>();
    if steps.is_empty() {
        output.push_str(&message.content);
        output.push('\n');
        return;
    }
    let chars = message.content.chars().collect::<Vec<_>>();
    let mut cursor = 0;
    for step in steps {
        let offset = step.offset.min(chars.len()).max(cursor);
        output.extend(chars[cursor..offset].iter());
        if let Some(command) = step
            .input
            .as_ref()
            .and_then(|input| input.get("command"))
            .and_then(serde_json::Value::as_str)
        {
            if let Some(result) = step.output.as_ref() {
                append_bash_block(
                    output,
                    command,
                    &result.stdout,
                    &result.stderr,
                    result.exit_code,
                );
            } else {
                append_bash_block(output, command, "", "", None);
            }
        }
        cursor = offset;
    }
    output.extend(chars[cursor..].iter());
    output.push('\n');
}

fn append_bash_block(
    output: &mut String,
    command: &str,
    stdout: &str,
    stderr: &str,
    exit_code: Option<i64>,
) {
    let content = format!("{command}\n{stdout}\n{stderr}");
    let fence_size = longest_backtick_run(&content).saturating_add(1).max(3);
    let fence = "`".repeat(fence_size);
    output.push_str(&format!("\n\n{fence}bash\n$ {command}\n"));
    if !stdout.is_empty() {
        output.push_str(stdout);
        if !stdout.ends_with('\n') {
            output.push('\n');
        }
    }
    if !stderr.is_empty() {
        output.push_str("stderr:\n");
        output.push_str(stderr);
        if !stderr.ends_with('\n') {
            output.push('\n');
        }
    }
    if let Some(exit_code) = exit_code {
        output.push_str(&format!("exit code: {exit_code}\n"));
    }
    output.push_str(&format!("{fence}\n\n"));
}

fn longest_backtick_run(value: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in value.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn markdown_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('\r', " ")
        .replace('\n', " ")
}

fn citation_domain(url: &str) -> &str {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest.split('/').next().unwrap_or(rest))
}

fn decode_column<T: for<'de> serde::Deserialize<'de>>(value: Option<&str>) -> Option<T> {
    value.and_then(|value| serde_json::from_str(value).ok())
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn slug(title: &Option<String>) -> String {
    let source = title.as_deref().unwrap_or("chat").to_ascii_lowercase();
    let mut result = String::new();
    let mut separator = false;
    for ch in source.chars() {
        if ch.is_ascii_alphanumeric() {
            if separator && !result.is_empty() {
                result.push('-');
            }
            result.push(ch);
            separator = false;
        } else {
            separator = true;
        }
    }
    if result.is_empty() {
        "chat".into()
    } else {
        result
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn iso_utc(timestamp: i64) -> String {
    let seconds = timestamp.div_euclid(1000);
    let millis = timestamp.rem_euclid(1000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let within_day = seconds.rem_euclid(86_400);
    let hour = within_day / 3_600;
    let minute = (within_day % 3_600) / 60;
    let second = within_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03} UTC")
}

fn utc_date(timestamp: i64) -> String {
    let (year, month, day) = civil_from_days(timestamp.div_euclid(86_400_000));
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

struct ExportError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: String,
}

impl ExportError {
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
    fn bad_request(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::BAD_REQUEST,
            "invalid_export_format",
            "Format must be md or json",
        )
    }
    fn not_found(headers: &HeaderMap) -> Self {
        Self::new(
            headers,
            StatusCode::NOT_FOUND,
            "not_found",
            "Chat not found",
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

impl IntoResponse for ExportError {
    fn into_response(self) -> Response {
        auth::api_error(&self.request_id, self.status, self.code, self.message, None)
    }
}

#[cfg(test)]
mod tests {
    use super::{AttachmentMetadata, iso_utc, json_export, markdown, slug, utc_date};
    use crate::{chats::ChatDetail, messages::MessageRecord};
    use std::collections::HashMap;

    #[test]
    fn formats_utc_timestamp_and_safe_filename_slug() {
        assert_eq!(iso_utc(0), "1970-01-01 00:00:00.000 UTC");
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(
            slug(&Some("A Chat: With punctuation!".into())),
            "a-chat-with-punctuation"
        );
    }

    #[test]
    fn markdown_export_contains_only_the_visible_branch_and_lists_attachments() {
        let chat = ChatDetail {
            id: "chat".into(),
            title: Some("Branch chat".into()),
            title_source: "auto".into(),
            model: "test/model".into(),
            tools: Vec::new(),
            current_leaf_id: Some("visible-leaf".into()),
            created_at: 0,
            updated_at: 0,
            messages: vec![
                message("root", None, "user", "Question"),
                message("visible-leaf", Some("root"), "assistant", "Visible answer"),
                message("other-leaf", Some("root"), "assistant", "Other answer"),
            ],
        };
        let attachments = HashMap::from([(
            "root".to_string(),
            vec![AttachmentMetadata {
                upload_id: "upload".into(),
                position: 0,
                filename: "notes.txt".into(),
                mime: "text/plain".into(),
                kind: "text".into(),
                size: 12,
                text_chars: None,
                text_pages: None,
                text_empty_pages: None,
            }],
        )]);
        let output = markdown(&chat, &attachments);
        assert!(output.contains("Question"));
        assert!(output.contains("Visible answer"));
        assert!(output.contains("notes.txt"));
        assert!(!output.contains("Other answer"));

        let json = json_export(&chat, &attachments).unwrap();
        assert!(json.contains("Other answer"));
        assert!(json.contains("\"cost\": null"));
        assert!(json.contains("\"filename\": \"notes.txt\""));
    }

    fn message(id: &str, parent_id: Option<&str>, role: &str, content: &str) -> MessageRecord {
        MessageRecord {
            id: id.into(),
            chat_id: "chat".into(),
            parent_id: parent_id.map(str::to_owned),
            role: role.into(),
            content: content.into(),
            status: "complete".into(),
            error: None,
            model: (role == "assistant").then(|| "test/model".into()),
            generation_id: None,
            finish_reason: None,
            prompt_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cost: None,
            tools: None,
            citations: None,
            tool_steps: None,
            web_search_requests: None,
            tool_cost: None,
            tool_fallback: false,
            created_at: 0,
            updated_at: 0,
            attachments: Vec::new(),
        }
    }
}
