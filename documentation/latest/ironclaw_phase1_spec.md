# Phase 1: Preparation — Detailed Implementation Spec

> **Timeline:** Day 1 (no IronClaw dependency required)
> **Branch:** `feature/ironclaw-integration`
> **Goal:** Extract 2 modules from `normalizer.rs` before it gets deleted in Phase 4.
> After this phase, the codebase builds and passes all tests — zero behavior change.

---

## Why Phase 1 Exists

`normalizer.rs` (838 LOC) is being deleted in Phase 4 because it converts WsFrame
events to UiEvent — a layer that becomes unnecessary when IronClaw emits events
directly through TauriChannel. But it contains two things we **must keep**:

1. **Token sanitizer** (lines 1-54) — strips leaked ChatML/Jinja tokens from LLM
   output. This stays in Scrappy forever (IronClaw emits raw text, Scrappy
   sanitizes before rendering).

2. **UiEvent types** (lines 56-189) — the enum + supporting structs that define
   the frontend event contract. TauriChannel needs these types.

Extracting them now (before any IronClaw changes) means:
- Clean git history — extraction is a pure refactor commit, easy to review
- Safe — no risk of losing code during the destructive Phase 4 cleanup
- Zero conflicts — the new modules exist before any command rewrites

---

## Task 1.1: Create `sanitizer.rs`

### Goal

Extract the LLM token sanitizer into a standalone module with no dependencies
on WsFrame, normalizer, or any other OpenClaw internals.

### File to Create

**`backend/src/openclaw/sanitizer.rs`**

```rust
//! LLM token sanitizer — strips leaked ChatML / Jinja template tokens
//!
//! Local models (Qwen, Mistral, Llama, etc.) sometimes emit control tokens
//! in their output. This module provides a function to strip them before
//! the text reaches the UI.
//!
//! IronClaw emits raw LLM output — Scrappy applies this sanitizer before
//! rendering in the frontend.

use regex::Regex;
use std::sync::LazyLock;

/// Compiled regexes for stripping LLM control tokens from output text.
/// These patterns catch ChatML (Qwen, Mistral, etc.), Llama, and common
/// template artifacts that local models sometimes emit.
static LLM_TOKEN_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // ChatML block markers: <|im_start|> optionally followed by a role word (assistant, user, etc.)
        Regex::new(r"<\|im_start\|>\w*").unwrap(),
        Regex::new(r"<\|im_end\|>").unwrap(),
        // Generic special tokens
        Regex::new(r"<\|end\|>").unwrap(),
        Regex::new(r"<\|endoftext\|>").unwrap(),
        Regex::new(r"<\|eot_id\|>").unwrap(),
        // Llama header blocks: <|start_header_id|>role<|end_header_id|> as a single unit
        Regex::new(r"<\|start_header_id\|>\w*<\|end_header_id\|>").unwrap(),
        // Fallback: catch orphaned header tokens that appear without the other half
        Regex::new(r"<\|start_header_id\|>").unwrap(),
        Regex::new(r"<\|end_header_id\|>").unwrap(),
        // Thinking blocks: <think>...</think>
        Regex::new(r"(?s)<think>.*?</think>").unwrap(),
        // Bare role markers that sometimes leak mid-text
        Regex::new(r"(?m)^(user|assistant|system|tool)>\s*$").unwrap(),
    ]
});

/// Strip leaked LLM template tokens from text before it reaches the UI.
///
/// Applied to all assistant text (deltas, snapshots, finals) before rendering.
/// IronClaw emits raw LLM output; this function cleans it for display.
pub fn strip_llm_tokens(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in LLM_TOKEN_PATTERNS.iter() {
        result = pattern.replace_all(&result, "").to_string();
    }
    // Collapse runs of 3+ newlines into 2
    let collapse = Regex::new(r"\n{3,}").unwrap();
    result = collapse.replace_all(&result, "\n\n").to_string();
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_chatml_tokens() {
        let input = "Hello<|im_end|>\n<|im_start|>assistant\nI'm fine<|im_end|>";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Hello\n\nI'm fine");
    }

    #[test]
    fn test_strip_thinking_blocks() {
        let input =
            "Let me help. <think>I should check the weather first...</think>Here's the plan:";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Let me help. Here's the plan:");
    }

    #[test]
    fn test_strip_llama_tokens() {
        let input = "Hello<|eot_id|><|start_header_id|>assistant<|end_header_id|>World";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "HelloWorld");
    }

    #[test]
    fn test_strip_orphaned_header_tokens() {
        let input = "Hello<|start_header_id|>World";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "HelloWorld");
    }

    #[test]
    fn test_strip_preserves_normal_text() {
        let input = "This is a normal response with **markdown** and `code`.";
        let result = strip_llm_tokens(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_strip_collapses_newlines() {
        let input = "Part 1\n\n\n\n\nPart 2";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Part 1\n\nPart 2");
    }
}
```

### What Changed vs Original

| Aspect | `normalizer.rs` (original) | `sanitizer.rs` (new) |
|---|---|---|
| Visibility of `strip_llm_tokens` | `fn` (private) | `pub fn` (public — TauriChannel and future consumers need it) |
| Dependencies | `regex`, `serde`, `serde_json`, `LazyLock`, `tracing`, `WsFrame` | `regex`, `LazyLock` only |
| Tests | Mixed with normalizer tests | Self-contained — 6 sanitizer-only tests |
| Module doc | Part of normalizer doc | Standalone doc explaining IronClaw context |

---

## Task 1.2: Create `ui_types.rs`

### Goal

Extract `UiEvent`, `UiSession`, `UiMessage`, `UiUsage` into a standalone module.
These types are the **frontend event contract** — they define the shape of every
`"openclaw-event"` emission and are consumed by TauriChannel.

### File to Create

**`backend/src/openclaw/ui_types.rs`**

```rust
//! Stable UI event types — the frontend event contract for OpenClaw
//!
//! These types define the shape of every `"openclaw-event"` emission.
//! The frontend's `OpenClawChatView.tsx` pattern-matches on `kind` to
//! decide how to render each event.
//!
//! After IronClaw integration, these are emitted by `TauriChannel`
//! instead of the old WS normalizer. The types themselves don't change.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable UI event contract — what the OpenClaw chat UI consumes.
///
/// Tagged with `#[serde(tag = "kind")]` so JSON looks like:
/// `{ "kind": "AssistantDelta", "session_key": "...", "delta": "..." }`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum UiEvent {
    /// Successfully connected to engine
    Connected { protocol: u32 },

    /// Disconnected from engine
    Disconnected { reason: String },

    /// List of available sessions
    SessionList { sessions: Vec<UiSession> },

    /// Chat history response
    History {
        session_key: String,
        messages: Vec<UiMessage>,
        has_more: bool,
        before: Option<String>,
    },

    /// Streaming assistant delta (append to current text)
    AssistantDelta {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        delta: String,
    },

    /// Internal assistant thinking (renders 🧠 indicator)
    AssistantInternal {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
    },

    /// Streaming assistant snapshot (replace current text)
    AssistantSnapshot {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
    },

    /// Final assistant message (replace, includes usage stats)
    AssistantFinal {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
        usage: Option<UiUsage>,
    },

    /// Tool execution update
    ToolUpdate {
        session_key: String,
        run_id: Option<String>,
        tool_name: String,
        status: String, // started|stream|ok|error
        input: Value,
        output: Value,
    },

    /// Run status change
    RunStatus {
        session_key: String,
        run_id: Option<String>,
        status: String, // started|in_flight|ok|error|aborted
        error: Option<String>,
    },

    /// Approval requested for tool execution
    ApprovalRequested {
        approval_id: String,
        session_key: String,
        tool_name: String,
        input: Value,
    },

    /// Approval has been resolved (approved/denied)
    ApprovalResolved {
        approval_id: String,
        session_key: String,
        approved: bool,
    },

    /// Engine error
    Error {
        code: String,
        message: String,
        details: Value,
    },

    /// Web login event (QR code, status)
    WebLogin {
        provider: String,
        qr_code: Option<String>,
        status: String,
    },

    /// Canvas update
    CanvasUpdate {
        session_key: String,
        run_id: Option<String>,
        content: String,
        content_type: String, // "html" | "json"
        url: Option<String>,
    },
}

/// Session metadata for session list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSession {
    pub session_key: String,
    pub title: Option<String>,
    pub updated_at_ms: Option<u64>,
    pub source: Option<String>, // slack|telegram|webchat|...
}

/// Message in chat history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiMessage {
    pub id: String,
    pub role: String, // user|assistant|tool|system
    pub ts_ms: u64,
    pub text: String,
    pub source: Option<String>,
}

/// Token usage stats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}
```

### What Changed vs Original

These types are **identical** to the originals in `normalizer.rs`. The only
differences are:
- Module doc comment updated to reference IronClaw/TauriChannel
- `Connected` variant doc says "engine" instead of "gateway"
- Each variant has a doc comment (some were missing)

---

## Task 1.3: Update `openclaw/mod.rs`

### Goal

Wire the new modules into the module tree. **Keep `normalizer.rs` alive** for
now — `ws_client.rs` still calls `normalizer::normalize_event()`. It will be
deleted in Phase 4 along with `ws_client.rs`.

### Current File

```rust
// backend/src/openclaw/mod.rs (CURRENT — 23 lines)
pub mod commands;
pub mod config;
pub mod deploy;
pub mod extra_commands;
pub mod fleet;
mod frames;
pub mod ipc;
mod normalizer;
mod ws_client;

pub use commands::OpenClawManager;
pub use config::OpenClawConfig;
pub use frames::{WsError, WsFrame};
pub use normalizer::{UiEvent, UiMessage, UiSession, UiUsage};
```

### New File

```rust
// backend/src/openclaw/mod.rs (AFTER Phase 1)
//! OpenClaw module — agent engine integration
//!
//! After IronClaw integration (Phase 4), the following will be deleted:
//! - frames, normalizer, ws_client (WS bridge — replaced by TauriChannel)
//! - OpenClawManager (process lifecycle — replaced by IronClawState)

pub mod commands;
pub mod config;
pub mod deploy;
pub mod extra_commands;
pub mod fleet;
pub mod sanitizer;   // NEW: extracted from normalizer
pub mod ui_types;    // NEW: extracted from normalizer

// --- Legacy WS bridge (removed in Phase 4) ---
mod frames;
pub mod ipc;
mod normalizer;
mod ws_client;

pub use commands::OpenClawManager;
pub use config::OpenClawConfig;
pub use frames::{WsError, WsFrame};

// Re-export UI types from new location (consumers don't change)
pub use ui_types::{UiEvent, UiMessage, UiSession, UiUsage};
```

### Key Decision: Why Keep `normalizer.rs` Alive?

`ws_client.rs` line 20 imports `normalizer::normalize_event()` and line 668
calls it. Until `ws_client.rs` is deleted in Phase 4, `normalizer.rs` must
exist. The normalizer still uses its **own private copy** of `strip_llm_tokens`
and `UiEvent` — we don't change normalizer.rs at all in Phase 1.

This means there are temporarily **two copies** of `UiEvent` and
`strip_llm_tokens` in the codebase:
- `normalizer.rs` — the old copies (used by `ws_client.rs`)
- `ui_types.rs` + `sanitizer.rs` — the new copies (used by everything else)

This duplication is **intentional and safe**:
- The old copies are deleted en masse in Phase 4
- The new copies are the canonical versions going forward
- Zero risk of divergence — nobody edits `normalizer.rs` between now and deletion

---

## Task 1.4: Verification

### Commands to Run

```bash
# 1. Build — must pass with zero errors
cargo build -p Scrappy

# 2. Tests — all 6 sanitizer tests must pass
cargo test -p Scrappy sanitizer

# 3. Full test suite — nothing regressed
cargo test -p Scrappy

# 4. Verify new modules are properly exported
cargo doc -p Scrappy --no-deps 2>&1 | grep -E "sanitizer|ui_types"

# 5. Verify old normalizer still works (ws_client still uses it)
cargo test -p Scrappy normalizer
```

### Expected Results

- `cargo build` — ✅ zero errors, zero warnings on new files
- `cargo test sanitizer` — ✅ 6 tests pass
- `cargo test` — ✅ same test count as before + 6 new (sanitizer tests that
  moved from normalizer — normalizer keeps its copies too, so total is +6)
- `normalizer` tests still pass (old normalizer is untouched)

### Git Commit

```bash
git add backend/src/openclaw/sanitizer.rs \
        backend/src/openclaw/ui_types.rs \
        backend/src/openclaw/mod.rs

git commit -m "refactor: extract sanitizer and ui_types from normalizer

Preparation for IronClaw integration Phase 2+.
- sanitizer.rs: LLM token stripping (strip_llm_tokens + 6 tests)
- ui_types.rs: UiEvent enum + UiSession/UiMessage/UiUsage structs
- normalizer.rs kept alive temporarily (ws_client still uses it)
- Zero behavior change — pure extraction refactor"
```

---

## File Change Summary

| Action | File | LOC |
|---|---|---|
| **CREATE** | `backend/src/openclaw/sanitizer.rs` | ~95 |
| **CREATE** | `backend/src/openclaw/ui_types.rs` | ~130 |
| **MODIFY** | `backend/src/openclaw/mod.rs` | 23 → 27 |
| **NO CHANGE** | `backend/src/openclaw/normalizer.rs` | 838 (untouched) |

**Net: +225 new lines, 0 deleted, 4 lines modified**

---

## What Phase 1 Does NOT Include

- ❌ No Cargo workspace creation — not needed yet (IronClaw doesn't exist locally)
- ❌ No IronClaw dependency — that's Phase 2
- ❌ No command rewrites — that's Phase 3
- ❌ No deletions — that's Phase 4
- ❌ No changes to `normalizer.rs` — it stays untouched until deletion

Phase 1 is a **pure, safe extraction refactor** with zero behavior change.
