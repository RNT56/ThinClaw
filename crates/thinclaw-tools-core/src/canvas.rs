//! Shared canvas / A2UI payload types.

use serde::{Deserialize, Serialize};

/// A canvas action produced by the tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CanvasAction {
    /// Display a UI panel.
    Show {
        /// Unique panel ID (agent-chosen, used for update/dismiss).
        panel_id: String,
        /// Panel title.
        title: String,
        /// UI components to render.
        components: Vec<UiComponent>,
        /// Panel position hint.
        #[serde(default)]
        position: PanelPosition,
        /// Whether the panel is modal (blocks interaction with chat).
        #[serde(default)]
        modal: bool,
    },
    /// Update an existing panel.
    Update {
        /// Panel ID to update.
        panel_id: String,
        /// Updated components (replaces all).
        components: Vec<UiComponent>,
    },
    /// Dismiss/close a panel.
    Dismiss {
        /// Panel ID to close.
        panel_id: String,
    },
    /// Show a toast notification.
    Notify {
        /// Notification message.
        message: String,
        /// Severity level.
        #[serde(default)]
        level: NotifyLevel,
        /// Auto-dismiss duration in seconds (0 = persistent).
        #[serde(default = "default_toast_duration")]
        duration_secs: u64,
    },
}

fn default_toast_duration() -> u64 {
    5
}

/// UI component that can be rendered in a panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiComponent {
    /// Markdown-formatted text block.
    Text { content: String },
    /// A heading.
    Heading {
        text: String,
        #[serde(default = "default_heading_level")]
        level: u8,
    },
    /// A data table.
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// A code block with syntax highlighting.
    Code { language: String, content: String },
    /// An image (base64 or URL).
    Image {
        src: String,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        width: Option<u32>,
    },
    /// A progress bar.
    Progress {
        #[serde(default)]
        label: String,
        value: f64,
        max: f64,
    },
    /// A key-value list.
    KeyValue { items: Vec<KvItem> },
    /// A separator / divider.
    Divider,
    /// A button (user interaction sends a message back to the agent).
    Button {
        label: String,
        /// Message sent back when clicked.
        action: String,
        #[serde(default)]
        style: ButtonStyle,
    },
    /// A form with input fields.
    Form {
        /// Form ID for response routing.
        form_id: String,
        fields: Vec<FormField>,
        submit_label: String,
    },
    /// JSON data rendered as a collapsible tree.
    Json {
        data: serde_json::Value,
        #[serde(default)]
        collapsed: bool,
    },
}

fn default_heading_level() -> u8 {
    2
}

/// Key-value item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvItem {
    pub key: String,
    pub value: String,
}

/// Form field definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FormField {
    Text {
        name: String,
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        required: bool,
    },
    Number {
        name: String,
        label: String,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
    },
    Select {
        name: String,
        label: String,
        options: Vec<String>,
    },
    Checkbox {
        name: String,
        label: String,
        #[serde(default)]
        checked: bool,
    },
    Textarea {
        name: String,
        label: String,
        #[serde(default)]
        rows: Option<u32>,
    },
}

/// Panel position hint.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PanelPosition {
    #[default]
    Right,
    Bottom,
    Center,
    Floating,
}

/// Notification severity.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

/// Button style.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Primary,
    Secondary,
    Danger,
    Ghost,
}
