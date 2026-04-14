//! In-session task planner tool (todo).
//!
//! Provides an in-memory task list that survives context compaction by
//! injecting active items back into the conversation. The agent uses
//! this for multi-step plans where tracking completion state matters.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Status of a single todo item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoStatus::Pending => write!(f, "pending"),
            TodoStatus::InProgress => write!(f, "in_progress"),
            TodoStatus::Completed => write!(f, "completed"),
            TodoStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl TodoStatus {
    fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => TodoStatus::InProgress,
            "completed" => TodoStatus::Completed,
            "cancelled" => TodoStatus::Cancelled,
            _ => TodoStatus::Pending,
        }
    }

    fn is_active(&self) -> bool {
        matches!(self, TodoStatus::Pending | TodoStatus::InProgress)
    }
}

/// A single task item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    /// Unique numeric index (1-based).
    pub id: usize,
    /// Task description.
    pub content: String,
    /// Current status.
    pub status: TodoStatus,
}

/// In-memory todo store for a single session.
#[derive(Debug, Default)]
pub struct TodoStore {
    items: Vec<TodoItem>,
    next_id: usize,
}

impl TodoStore {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
        }
    }

    /// Replace the entire list with new items.
    fn write_replace(&mut self, items: Vec<(String, String)>) {
        self.items.clear();
        self.next_id = 1;
        for (content, status) in items {
            self.items.push(TodoItem {
                id: self.next_id,
                content,
                status: TodoStatus::from_str(&status),
            });
            self.next_id += 1;
        }
    }

    /// Merge items: update existing by ID, append new ones.
    fn write_merge(&mut self, items: Vec<(Option<usize>, String, String)>) {
        for (id_opt, content, status) in items {
            if let Some(id) = id_opt {
                // Update existing item
                if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                    item.content = content;
                    item.status = TodoStatus::from_str(&status);
                    continue;
                }
            }
            // Append new item
            self.items.push(TodoItem {
                id: self.next_id,
                content,
                status: TodoStatus::from_str(&status),
            });
            self.next_id += 1;
        }
    }

    /// Get summary counts.
    fn summary(&self) -> serde_json::Value {
        let total = self.items.len();
        let pending = self
            .items
            .iter()
            .filter(|i| i.status == TodoStatus::Pending)
            .count();
        let in_progress = self
            .items
            .iter()
            .filter(|i| i.status == TodoStatus::InProgress)
            .count();
        let completed = self
            .items
            .iter()
            .filter(|i| i.status == TodoStatus::Completed)
            .count();
        let cancelled = self
            .items
            .iter()
            .filter(|i| i.status == TodoStatus::Cancelled)
            .count();

        serde_json::json!({
            "total": total,
            "pending": pending,
            "in_progress": in_progress,
            "completed": completed,
            "cancelled": cancelled,
        })
    }

    /// Format active items for post-compaction injection.
    pub fn format_for_injection(&self) -> Option<String> {
        let active: Vec<&TodoItem> = self.items.iter().filter(|i| i.status.is_active()).collect();

        if active.is_empty() {
            return None;
        }

        let mut out = String::from("[Active task list preserved across context compression]\n");
        for item in &active {
            let marker = match item.status {
                TodoStatus::InProgress => "🔄",
                _ => "⬜",
            };
            out.push_str(&format!("{} {}. {}\n", marker, item.id, item.content));
        }
        Some(out)
    }

    /// Get all items as JSON.
    fn to_json(&self) -> Vec<serde_json::Value> {
        self.items
            .iter()
            .map(|item| {
                serde_json::json!({
                    "id": item.id,
                    "content": item.content,
                    "status": item.status.to_string(),
                })
            })
            .collect()
    }
}

/// Shared todo store reference.
pub type SharedTodoStore = Arc<RwLock<TodoStore>>;

/// Create a new shared todo store.
pub fn new_shared_todo_store() -> SharedTodoStore {
    Arc::new(RwLock::new(TodoStore::new()))
}

/// In-session task planner tool.
pub struct TodoTool {
    store: SharedTodoStore,
}

impl TodoTool {
    pub fn new(store: SharedTodoStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "In-session task planner. Use for multi-step plans (3+ tasks). \
         Call with no parameters to read the current list. \
         Call with 'todos' array to write/update tasks. \
         Items survive context compression. \
         Keep only ONE item 'in_progress' at a time. \
         Mark items 'completed' or 'cancelled' when done."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "Array of tasks to write. Omit to read current list.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "integer",
                                "description": "Existing item ID to update (omit for new items)"
                            },
                            "content": {
                                "type": "string",
                                "description": "Task description"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "cancelled"],
                                "description": "Task status (default: 'pending')"
                            }
                        },
                        "required": ["content"]
                    }
                },
                "merge": {
                    "type": "boolean",
                    "description": "If true, merge with existing items (update by ID, append new). If false, replace entire list (default: true)."
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let todos_param = params.get("todos").and_then(|v| v.as_array());

        if let Some(todos) = todos_param {
            let merge = params
                .get("merge")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let mut store = self.store.write().await;

            if merge {
                let items: Vec<(Option<usize>, String, String)> = todos
                    .iter()
                    .map(|item| {
                        let id = item.get("id").and_then(|v| v.as_u64()).map(|v| v as usize);
                        let content = item
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let status = item
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("pending")
                            .to_string();
                        (id, content, status)
                    })
                    .collect();
                store.write_merge(items);
            } else {
                let items: Vec<(String, String)> = todos
                    .iter()
                    .map(|item| {
                        let content = item
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let status = item
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("pending")
                            .to_string();
                        (content, status)
                    })
                    .collect();
                store.write_replace(items);
            }

            let result = serde_json::json!({
                "todos": store.to_json(),
                "summary": store.summary(),
                "action": "updated",
            });

            Ok(ToolOutput::success(result, start.elapsed()))
        } else {
            // Read mode
            let store = self.store.read().await;
            let result = serde_json::json!({
                "todos": store.to_json(),
                "summary": store.summary(),
                "action": "read",
            });

            Ok(ToolOutput::success(result, start.elapsed()))
        }
    }

    fn requires_sanitization(&self) -> bool {
        false // In-memory, no external data
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo_store_write_replace() {
        let mut store = TodoStore::new();
        store.write_replace(vec![
            ("Task 1".into(), "pending".into()),
            ("Task 2".into(), "in_progress".into()),
        ]);
        assert_eq!(store.items.len(), 2);
        assert_eq!(store.items[0].id, 1);
        assert_eq!(store.items[1].status, TodoStatus::InProgress);
    }

    #[test]
    fn test_todo_store_write_merge() {
        let mut store = TodoStore::new();
        store.write_replace(vec![
            ("Task 1".into(), "pending".into()),
            ("Task 2".into(), "pending".into()),
        ]);
        // Update item 1, add new item 3
        store.write_merge(vec![
            (Some(1), "Task 1 updated".into(), "completed".into()),
            (None, "Task 3".into(), "pending".into()),
        ]);
        assert_eq!(store.items.len(), 3);
        assert_eq!(store.items[0].content, "Task 1 updated");
        assert_eq!(store.items[0].status, TodoStatus::Completed);
        assert_eq!(store.items[2].content, "Task 3");
    }

    #[test]
    fn test_todo_store_summary() {
        let mut store = TodoStore::new();
        store.write_replace(vec![
            ("A".into(), "pending".into()),
            ("B".into(), "in_progress".into()),
            ("C".into(), "completed".into()),
            ("D".into(), "cancelled".into()),
        ]);
        let summary = store.summary();
        assert_eq!(summary["total"], 4);
        assert_eq!(summary["pending"], 1);
        assert_eq!(summary["in_progress"], 1);
        assert_eq!(summary["completed"], 1);
        assert_eq!(summary["cancelled"], 1);
    }

    #[test]
    fn test_format_for_injection_active_only() {
        let mut store = TodoStore::new();
        store.write_replace(vec![
            ("Active task".into(), "in_progress".into()),
            ("Done task".into(), "completed".into()),
            ("Pending task".into(), "pending".into()),
        ]);
        let injection = store.format_for_injection().unwrap();
        assert!(injection.contains("Active task"));
        assert!(injection.contains("Pending task"));
        assert!(!injection.contains("Done task"));
    }

    #[test]
    fn test_format_for_injection_none_when_empty() {
        let store = TodoStore::new();
        assert!(store.format_for_injection().is_none());
    }

    #[test]
    fn test_format_for_injection_none_when_all_done() {
        let mut store = TodoStore::new();
        store.write_replace(vec![
            ("Done".into(), "completed".into()),
            ("Cancelled".into(), "cancelled".into()),
        ]);
        assert!(store.format_for_injection().is_none());
    }

    #[tokio::test]
    async fn test_todo_tool_read_empty() {
        let store = new_shared_todo_store();
        let tool = TodoTool::new(store);
        let ctx = JobContext::default();

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let todos = result.result.get("todos").unwrap().as_array().unwrap();
        assert!(todos.is_empty());
    }

    #[tokio::test]
    async fn test_todo_tool_write_and_read() {
        let store = new_shared_todo_store();
        let tool = TodoTool::new(store);
        let ctx = JobContext::default();

        // Write
        tool.execute(
            serde_json::json!({
                "todos": [
                    {"content": "Step 1", "status": "in_progress"},
                    {"content": "Step 2"}
                ],
                "merge": false
            }),
            &ctx,
        )
        .await
        .unwrap();

        // Read
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let todos = result.result.get("todos").unwrap().as_array().unwrap();
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0]["status"], "in_progress");
        assert_eq!(todos[1]["status"], "pending");
    }

    #[tokio::test]
    async fn test_todo_tool_merge() {
        let store = new_shared_todo_store();
        let tool = TodoTool::new(store);
        let ctx = JobContext::default();

        // Initial write
        tool.execute(
            serde_json::json!({
                "todos": [{"content": "Task A"}],
                "merge": false
            }),
            &ctx,
        )
        .await
        .unwrap();

        // Merge update
        let result = tool
            .execute(
                serde_json::json!({
                    "todos": [
                        {"id": 1, "content": "Task A done", "status": "completed"},
                        {"content": "Task B"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();

        let todos = result.result.get("todos").unwrap().as_array().unwrap();
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0]["status"], "completed");
        assert_eq!(todos[1]["content"], "Task B");
    }
}
