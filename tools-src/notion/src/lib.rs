//! Notion WASM Tool for ThinClaw.
//!
//! Provides full Notion integration: pages, databases, blocks, search,
//! and user management via the Notion API v2022-06-28.
//!
//! # Authentication
//!
//! Uses a Notion Internal Integration Token (bearer auth).
//! Create one at: https://www.notion.so/my-integrations
//!
//! Store it with: `thinclaw secret set notion_token <token>`
//!
//! The token must have the following capabilities enabled:
//! - Read content
//! - Update content
//! - Insert content
//! - Read comments (optional, for comment features)
//!
//! Remember to **share pages/databases** with your integration
//! in Notion's "Connections" menu — integrations can only access
//! content explicitly shared with them.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const API_VERSION: &str = "2022-06-28";
const MAX_TEXT_LENGTH: usize = 65536;
const MAX_PAGE_SIZE: u32 = 100;

/// Validate input length to prevent oversized payloads.
fn validate_input_length(s: &str, field_name: &str) -> Result<(), String> {
    if s.len() > MAX_TEXT_LENGTH {
        return Err(format!(
            "Input '{}' exceeds maximum length of {} characters",
            field_name, MAX_TEXT_LENGTH
        ));
    }
    Ok(())
}

/// Percent-encode a string for safe use in URL path segments.
fn url_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

struct NotionTool;

// ── Actions ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum NotionAction {
    // ── Search ──────────────────────────────────────────────────────
    #[serde(rename = "search")]
    Search {
        query: Option<String>,
        filter: Option<SearchFilter>,
        page_size: Option<u32>,
        start_cursor: Option<String>,
    },

    // ── Pages ───────────────────────────────────────────────────────
    #[serde(rename = "get_page")]
    GetPage { page_id: String },

    #[serde(rename = "create_page")]
    CreatePage {
        parent_id: String,
        parent_type: Option<String>,
        title: String,
        properties: Option<serde_json::Value>,
        children: Option<Vec<serde_json::Value>>,
    },

    #[serde(rename = "update_page")]
    UpdatePage {
        page_id: String,
        properties: serde_json::Value,
        archived: Option<bool>,
    },

    #[serde(rename = "archive_page")]
    ArchivePage { page_id: String },

    // ── Databases ───────────────────────────────────────────────────
    #[serde(rename = "get_database")]
    GetDatabase { database_id: String },

    #[serde(rename = "query_database")]
    QueryDatabase {
        database_id: String,
        filter: Option<serde_json::Value>,
        sorts: Option<Vec<serde_json::Value>>,
        page_size: Option<u32>,
        start_cursor: Option<String>,
    },

    #[serde(rename = "create_database")]
    CreateDatabase {
        parent_id: String,
        title: String,
        properties: serde_json::Value,
    },

    #[serde(rename = "update_database")]
    UpdateDatabase {
        database_id: String,
        title: Option<String>,
        properties: Option<serde_json::Value>,
    },

    // ── Blocks ──────────────────────────────────────────────────────
    #[serde(rename = "get_block")]
    GetBlock { block_id: String },

    #[serde(rename = "get_block_children")]
    GetBlockChildren {
        block_id: String,
        page_size: Option<u32>,
        start_cursor: Option<String>,
    },

    #[serde(rename = "append_block_children")]
    AppendBlockChildren {
        block_id: String,
        children: Vec<serde_json::Value>,
    },

    #[serde(rename = "update_block")]
    UpdateBlock {
        block_id: String,
        #[serde(flatten)]
        content: serde_json::Value,
    },

    #[serde(rename = "delete_block")]
    DeleteBlock { block_id: String },

    // ── Users ───────────────────────────────────────────────────────
    #[serde(rename = "list_users")]
    ListUsers {
        page_size: Option<u32>,
        start_cursor: Option<String>,
    },

    #[serde(rename = "get_user")]
    GetUser { user_id: String },

    #[serde(rename = "get_me")]
    GetMe,

    // ── Comments ────────────────────────────────────────────────────
    #[serde(rename = "get_comments")]
    GetComments {
        block_id: String,
        page_size: Option<u32>,
        start_cursor: Option<String>,
    },

    #[serde(rename = "create_comment")]
    CreateComment {
        parent_id: String,
        rich_text: Vec<serde_json::Value>,
        discussion_id: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
pub struct SearchFilter {
    value: String,
    property: String,
}

// ── WIT export ─────────────────────────────────────────────────────────

impl exports::near::agent::tool::Guest for NotionTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Notion workspace integration for managing pages, databases, blocks, \
         and searching content. Supports creating, reading, updating, and \
         archiving pages, querying databases with filters and sorts, \
         manipulating page content blocks, and managing comments. \
         Authentication is handled via the 'notion_token' secret (Internal Integration Token) \
         injected by the host."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: NotionAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    // Pre-flight: verify token exists
    let _ = check_token()?;

    match action {
        // Search
        NotionAction::Search { query, filter, page_size, start_cursor } => {
            search(query.as_deref(), filter, page_size, start_cursor.as_deref())
        }

        // Pages
        NotionAction::GetPage { page_id } => get_page(&page_id),
        NotionAction::CreatePage { parent_id, parent_type, title, properties, children } => {
            create_page(&parent_id, parent_type.as_deref(), &title, properties, children)
        }
        NotionAction::UpdatePage { page_id, properties, archived } => {
            update_page(&page_id, properties, archived)
        }
        NotionAction::ArchivePage { page_id } => archive_page(&page_id),

        // Databases
        NotionAction::GetDatabase { database_id } => get_database(&database_id),
        NotionAction::QueryDatabase { database_id, filter, sorts, page_size, start_cursor } => {
            query_database(&database_id, filter, sorts, page_size, start_cursor.as_deref())
        }
        NotionAction::CreateDatabase { parent_id, title, properties } => {
            create_database(&parent_id, &title, properties)
        }
        NotionAction::UpdateDatabase { database_id, title, properties } => {
            update_database(&database_id, title.as_deref(), properties)
        }

        // Blocks
        NotionAction::GetBlock { block_id } => get_block(&block_id),
        NotionAction::GetBlockChildren { block_id, page_size, start_cursor } => {
            get_block_children(&block_id, page_size, start_cursor.as_deref())
        }
        NotionAction::AppendBlockChildren { block_id, children } => {
            append_block_children(&block_id, children)
        }
        NotionAction::UpdateBlock { block_id, content } => update_block(&block_id, content),
        NotionAction::DeleteBlock { block_id } => delete_block(&block_id),

        // Users
        NotionAction::ListUsers { page_size, start_cursor } => {
            list_users(page_size, start_cursor.as_deref())
        }
        NotionAction::GetUser { user_id } => get_user(&user_id),
        NotionAction::GetMe => get_me(),

        // Comments
        NotionAction::GetComments { block_id, page_size, start_cursor } => {
            get_comments(&block_id, page_size, start_cursor.as_deref())
        }
        NotionAction::CreateComment { parent_id, rich_text, discussion_id } => {
            create_comment(&parent_id, rich_text, discussion_id.as_deref())
        }
    }
}

// ── Authentication ─────────────────────────────────────────────────────

fn check_token() -> Result<String, String> {
    if near::agent::host::secret_exists("notion_token") {
        return Ok("present".to_string());
    }
    Err("Notion token not found in secret store. Set it with: thinclaw secret set notion_token <token>. \
         Create an integration at https://www.notion.so/my-integrations".into())
}

// ── HTTP helpers ───────────────────────────────────────────────────────

fn notion_get(path: &str) -> Result<String, String> {
    notion_request("GET", path, None)
}

fn notion_post(path: &str, body: serde_json::Value) -> Result<String, String> {
    notion_request("POST", path, Some(body.to_string()))
}

fn notion_patch(path: &str, body: serde_json::Value) -> Result<String, String> {
    notion_request("PATCH", path, Some(body.to_string()))
}

fn notion_delete(path: &str) -> Result<String, String> {
    notion_request("DELETE", path, None)
}

fn notion_request(method: &str, path: &str, body: Option<String>) -> Result<String, String> {
    let url = format!("https://api.notion.com/v1{}", path);

    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Notion-Version": API_VERSION,
        "User-Agent": "ThinClaw-Notion-Tool"
    });

    let body_bytes = body.map(|b| b.into_bytes());

    let max_retries = 3;
    let mut attempt = 0;

    loop {
        attempt += 1;

        let response = near::agent::host::http_request(
            method,
            &url,
            &headers.to_string(),
            body_bytes.as_deref(),
            None,
        );

        match response {
            Ok(resp) => {
                if resp.status >= 200 && resp.status < 300 {
                    return String::from_utf8(resp.body)
                        .map_err(|e| format!("Invalid UTF-8: {}", e));
                } else if attempt < max_retries && (resp.status == 429 || resp.status >= 500) {
                    near::agent::host::log(
                        near::agent::host::LogLevel::Warn,
                        &format!(
                            "Notion API error {} (attempt {}/{}). Retrying...",
                            resp.status, attempt, max_retries
                        ),
                    );
                    continue;
                } else {
                    let body_str = String::from_utf8_lossy(&resp.body);
                    return Err(format!("Notion API error {}: {}", resp.status, body_str));
                }
            }
            Err(e) => {
                if attempt < max_retries {
                    near::agent::host::log(
                        near::agent::host::LogLevel::Warn,
                        &format!(
                            "HTTP request failed: {} (attempt {}/{}). Retrying...",
                            e, attempt, max_retries
                        ),
                    );
                    continue;
                }
                return Err(format!(
                    "HTTP request failed after {} attempts: {}",
                    max_retries, e
                ));
            }
        }
    }
}

// ── Search ─────────────────────────────────────────────────────────────

fn search(
    query: Option<&str>,
    filter: Option<SearchFilter>,
    page_size: Option<u32>,
    start_cursor: Option<&str>,
) -> Result<String, String> {
    if let Some(q) = query {
        validate_input_length(q, "query")?;
    }

    let mut body = serde_json::json!({});

    if let Some(q) = query {
        body["query"] = serde_json::json!(q);
    }
    if let Some(f) = filter {
        body["filter"] = serde_json::json!({
            "value": f.value,
            "property": f.property,
        });
    }
    if let Some(ps) = page_size {
        body["page_size"] = serde_json::json!(ps.min(MAX_PAGE_SIZE));
    }
    if let Some(cursor) = start_cursor {
        body["start_cursor"] = serde_json::json!(cursor);
    }

    notion_post("/search", body)
}

// ── Pages ──────────────────────────────────────────────────────────────

fn get_page(page_id: &str) -> Result<String, String> {
    let encoded = url_encode_path(page_id);
    notion_get(&format!("/pages/{}", encoded))
}

fn create_page(
    parent_id: &str,
    parent_type: Option<&str>,
    title: &str,
    properties: Option<serde_json::Value>,
    children: Option<Vec<serde_json::Value>>,
) -> Result<String, String> {
    validate_input_length(title, "title")?;

    let parent_type = parent_type.unwrap_or("database_id");
    let parent = serde_json::json!({ parent_type: parent_id });

    let mut props = properties.unwrap_or_else(|| serde_json::json!({}));

    // If creating in a database, set the "Name" or "title" property
    if parent_type == "database_id" {
        if !props.as_object().map_or(false, |o| o.contains_key("Name") || o.contains_key("title")) {
            props["Name"] = serde_json::json!({
                "title": [{ "text": { "content": title } }]
            });
        }
    }
    // If creating as a child page, set the title property
    if parent_type == "page_id" {
        if !props.as_object().map_or(false, |o| o.contains_key("title")) {
            props["title"] = serde_json::json!([
                { "text": { "content": title } }
            ]);
        }
    }

    let mut body = serde_json::json!({
        "parent": parent,
        "properties": props,
    });

    if let Some(children) = children {
        body["children"] = serde_json::json!(children);
    }

    notion_post("/pages", body)
}

fn update_page(
    page_id: &str,
    properties: serde_json::Value,
    archived: Option<bool>,
) -> Result<String, String> {
    let encoded = url_encode_path(page_id);
    let mut body = serde_json::json!({ "properties": properties });
    if let Some(archived) = archived {
        body["archived"] = serde_json::json!(archived);
    }
    notion_patch(&format!("/pages/{}", encoded), body)
}

fn archive_page(page_id: &str) -> Result<String, String> {
    let encoded = url_encode_path(page_id);
    notion_patch(
        &format!("/pages/{}", encoded),
        serde_json::json!({ "archived": true }),
    )
}

// ── Databases ──────────────────────────────────────────────────────────

fn get_database(database_id: &str) -> Result<String, String> {
    let encoded = url_encode_path(database_id);
    notion_get(&format!("/databases/{}", encoded))
}

fn query_database(
    database_id: &str,
    filter: Option<serde_json::Value>,
    sorts: Option<Vec<serde_json::Value>>,
    page_size: Option<u32>,
    start_cursor: Option<&str>,
) -> Result<String, String> {
    let encoded = url_encode_path(database_id);
    let mut body = serde_json::json!({});

    if let Some(f) = filter {
        body["filter"] = f;
    }
    if let Some(s) = sorts {
        body["sorts"] = serde_json::json!(s);
    }
    if let Some(ps) = page_size {
        body["page_size"] = serde_json::json!(ps.min(MAX_PAGE_SIZE));
    }
    if let Some(cursor) = start_cursor {
        body["start_cursor"] = serde_json::json!(cursor);
    }

    notion_post(&format!("/databases/{}/query", encoded), body)
}

fn create_database(
    parent_id: &str,
    title: &str,
    properties: serde_json::Value,
) -> Result<String, String> {
    validate_input_length(title, "title")?;

    let body = serde_json::json!({
        "parent": { "page_id": parent_id },
        "title": [{ "text": { "content": title } }],
        "properties": properties,
    });

    notion_post("/databases", body)
}

fn update_database(
    database_id: &str,
    title: Option<&str>,
    properties: Option<serde_json::Value>,
) -> Result<String, String> {
    let encoded = url_encode_path(database_id);
    let mut body = serde_json::json!({});

    if let Some(t) = title {
        validate_input_length(t, "title")?;
        body["title"] = serde_json::json!([{ "text": { "content": t } }]);
    }
    if let Some(p) = properties {
        body["properties"] = p;
    }

    notion_patch(&format!("/databases/{}", encoded), body)
}

// ── Blocks ─────────────────────────────────────────────────────────────

fn get_block(block_id: &str) -> Result<String, String> {
    let encoded = url_encode_path(block_id);
    notion_get(&format!("/blocks/{}", encoded))
}

fn get_block_children(
    block_id: &str,
    page_size: Option<u32>,
    start_cursor: Option<&str>,
) -> Result<String, String> {
    let encoded = url_encode_path(block_id);
    let ps = page_size.unwrap_or(25).min(MAX_PAGE_SIZE);
    let mut path = format!("/blocks/{}/children?page_size={}", encoded, ps);
    if let Some(cursor) = start_cursor {
        path.push_str(&format!("&start_cursor={}", url_encode_path(cursor)));
    }
    notion_get(&path)
}

fn append_block_children(
    block_id: &str,
    children: Vec<serde_json::Value>,
) -> Result<String, String> {
    if children.is_empty() {
        return Err("children array cannot be empty".into());
    }
    let encoded = url_encode_path(block_id);
    let body = serde_json::json!({ "children": children });
    notion_patch(&format!("/blocks/{}/children", encoded), body)
}

fn update_block(block_id: &str, content: serde_json::Value) -> Result<String, String> {
    let encoded = url_encode_path(block_id);
    notion_patch(&format!("/blocks/{}", encoded), content)
}

fn delete_block(block_id: &str) -> Result<String, String> {
    let encoded = url_encode_path(block_id);
    notion_delete(&format!("/blocks/{}", encoded))
}

// ── Users ──────────────────────────────────────────────────────────────

fn list_users(page_size: Option<u32>, start_cursor: Option<&str>) -> Result<String, String> {
    let ps = page_size.unwrap_or(25).min(MAX_PAGE_SIZE);
    let mut path = format!("/users?page_size={}", ps);
    if let Some(cursor) = start_cursor {
        path.push_str(&format!("&start_cursor={}", url_encode_path(cursor)));
    }
    notion_get(&path)
}

fn get_user(user_id: &str) -> Result<String, String> {
    let encoded = url_encode_path(user_id);
    notion_get(&format!("/users/{}", encoded))
}

fn get_me() -> Result<String, String> {
    notion_get("/users/me")
}

// ── Comments ───────────────────────────────────────────────────────────

fn get_comments(
    block_id: &str,
    page_size: Option<u32>,
    start_cursor: Option<&str>,
) -> Result<String, String> {
    let encoded = url_encode_path(block_id);
    let ps = page_size.unwrap_or(25).min(MAX_PAGE_SIZE);
    let mut path = format!("/comments?block_id={}&page_size={}", encoded, ps);
    if let Some(cursor) = start_cursor {
        path.push_str(&format!("&start_cursor={}", url_encode_path(cursor)));
    }
    notion_get(&path)
}

fn create_comment(
    parent_id: &str,
    rich_text: Vec<serde_json::Value>,
    discussion_id: Option<&str>,
) -> Result<String, String> {
    if rich_text.is_empty() {
        return Err("rich_text array cannot be empty".into());
    }

    let mut body = serde_json::json!({
        "rich_text": rich_text,
    });

    if let Some(disc_id) = discussion_id {
        body["discussion_id"] = serde_json::json!(disc_id);
    } else {
        body["parent"] = serde_json::json!({ "page_id": parent_id });
    }

    notion_post("/comments", body)
}

// ── Schema ─────────────────────────────────────────────────────────────

const SCHEMA: &str = r#"{
    "type": "object",
    "required": ["action"],
    "oneOf": [
        {
            "properties": {
                "action": { "const": "search" },
                "query": { "type": "string", "description": "Search query text" },
                "filter": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "string", "enum": ["page", "database"] },
                        "property": { "type": "string", "const": "object" }
                    },
                    "description": "Filter by object type"
                },
                "page_size": { "type": "integer", "description": "Max results (default 25, max 100)" },
                "start_cursor": { "type": "string", "description": "Pagination cursor" }
            },
            "required": ["action"]
        },
        {
            "properties": {
                "action": { "const": "get_page" },
                "page_id": { "type": "string", "description": "Page ID (UUID or dashed UUID)" }
            },
            "required": ["action", "page_id"]
        },
        {
            "properties": {
                "action": { "const": "create_page" },
                "parent_id": { "type": "string", "description": "Parent database or page ID" },
                "parent_type": { "type": "string", "enum": ["database_id", "page_id"], "default": "database_id" },
                "title": { "type": "string", "description": "Page title" },
                "properties": { "type": "object", "description": "Notion page properties (database columns)" },
                "children": { "type": "array", "description": "Block content to add to the page" }
            },
            "required": ["action", "parent_id", "title"]
        },
        {
            "properties": {
                "action": { "const": "update_page" },
                "page_id": { "type": "string" },
                "properties": { "type": "object", "description": "Properties to update" },
                "archived": { "type": "boolean", "description": "Set true to archive" }
            },
            "required": ["action", "page_id", "properties"]
        },
        {
            "properties": {
                "action": { "const": "archive_page" },
                "page_id": { "type": "string", "description": "Page ID to archive" }
            },
            "required": ["action", "page_id"]
        },
        {
            "properties": {
                "action": { "const": "get_database" },
                "database_id": { "type": "string", "description": "Database ID" }
            },
            "required": ["action", "database_id"]
        },
        {
            "properties": {
                "action": { "const": "query_database" },
                "database_id": { "type": "string", "description": "Database ID" },
                "filter": { "type": "object", "description": "Notion filter object" },
                "sorts": { "type": "array", "description": "Sort conditions" },
                "page_size": { "type": "integer", "default": 25 },
                "start_cursor": { "type": "string" }
            },
            "required": ["action", "database_id"]
        },
        {
            "properties": {
                "action": { "const": "create_database" },
                "parent_id": { "type": "string", "description": "Parent page ID" },
                "title": { "type": "string", "description": "Database title" },
                "properties": { "type": "object", "description": "Database schema (property definitions)" }
            },
            "required": ["action", "parent_id", "title", "properties"]
        },
        {
            "properties": {
                "action": { "const": "update_database" },
                "database_id": { "type": "string" },
                "title": { "type": "string", "description": "New title" },
                "properties": { "type": "object", "description": "Schema changes" }
            },
            "required": ["action", "database_id"]
        },
        {
            "properties": {
                "action": { "const": "get_block" },
                "block_id": { "type": "string", "description": "Block ID" }
            },
            "required": ["action", "block_id"]
        },
        {
            "properties": {
                "action": { "const": "get_block_children" },
                "block_id": { "type": "string", "description": "Block or page ID (pages are also blocks)" },
                "page_size": { "type": "integer", "default": 25 },
                "start_cursor": { "type": "string" }
            },
            "required": ["action", "block_id"]
        },
        {
            "properties": {
                "action": { "const": "append_block_children" },
                "block_id": { "type": "string", "description": "Block or page ID to append to" },
                "children": {
                    "type": "array",
                    "description": "Block objects to append (paragraph, heading, to_do, bulleted_list_item, etc.)",
                    "items": { "type": "object" }
                }
            },
            "required": ["action", "block_id", "children"]
        },
        {
            "properties": {
                "action": { "const": "update_block" },
                "block_id": { "type": "string" },
                "paragraph": { "type": "object", "description": "Updated paragraph content" },
                "heading_1": { "type": "object" },
                "heading_2": { "type": "object" },
                "heading_3": { "type": "object" },
                "to_do": { "type": "object" },
                "bulleted_list_item": { "type": "object" },
                "numbered_list_item": { "type": "object" },
                "toggle": { "type": "object" },
                "archived": { "type": "boolean" }
            },
            "required": ["action", "block_id"]
        },
        {
            "properties": {
                "action": { "const": "delete_block" },
                "block_id": { "type": "string", "description": "Block ID to delete" }
            },
            "required": ["action", "block_id"]
        },
        {
            "properties": {
                "action": { "const": "list_users" },
                "page_size": { "type": "integer", "default": 25 },
                "start_cursor": { "type": "string" }
            },
            "required": ["action"]
        },
        {
            "properties": {
                "action": { "const": "get_user" },
                "user_id": { "type": "string", "description": "User ID" }
            },
            "required": ["action", "user_id"]
        },
        {
            "properties": {
                "action": { "const": "get_me" }
            },
            "required": ["action"]
        },
        {
            "properties": {
                "action": { "const": "get_comments" },
                "block_id": { "type": "string", "description": "Page or block ID to get comments for" },
                "page_size": { "type": "integer", "default": 25 },
                "start_cursor": { "type": "string" }
            },
            "required": ["action", "block_id"]
        },
        {
            "properties": {
                "action": { "const": "create_comment" },
                "parent_id": { "type": "string", "description": "Page ID (required if no discussion_id)" },
                "rich_text": {
                    "type": "array",
                    "description": "Rich text content. Minimum: [{\"text\":{\"content\":\"your message\"}}]",
                    "items": { "type": "object" }
                },
                "discussion_id": { "type": "string", "description": "Reply to existing discussion (optional)" }
            },
            "required": ["action", "parent_id", "rich_text"]
        }
    ]
}"#;

export!(NotionTool);
