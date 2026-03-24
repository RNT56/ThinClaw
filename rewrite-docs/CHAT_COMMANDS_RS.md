> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# Chat Commands & Control (Slash Commands)

There is one final internal system that is crucial to rewrite if you want the agent to be controllable when you are away from your Mac.

In your Tauri UI, if the user wants to change from `gpt-4o` to `claude-3-5-sonnet`, or wipe the agent's short-term memory, they just click a button.

But what happens when the user is chatting with ThinClaw through **Telegram**, **Discord**, or **Slack**? They don't have your React UI buttons!

To solve this, OpenClaw implements a unified **Chat Commands Registry** (found in `src/auto-reply/commands-registry.ts`).

---

## 1. The Core Slash Commands

In Rust, you must implement an interception layer that catches messages starting with `/` before they are sent to the LLM.

Here are the critical commands that must be ported:

- **`/reset`**: Wipes the agent's short-term context window (the recent chat history). Extremely important if the LLM gets confused or stuck in a loop.
- **`/status`**: The agent replies with a diagnostic block showing its current model, uptime, temperature, and enabled skills.
- **`/model [name]`**: Dynamically overrides the `config.toml` model choice for the current session. (e.g., `/model claude-3-5-sonnet`).
- **`/system [prompt]`**: Dynamically overrides the temporary system instructions for the current session.
- **`/retry`**: Re-generates the last response.
- **`/mute` / `/unmute`**: Prevents the agent from proactively replying in noisy group chats.

## 2. Implementing the Interception Layer in Rust

In your Rust `channel` traits (which we defined for Discord, Slack, etc.), when a new `String` message arrives from the chat provider, it must pass through a **Command Router** _before_ being forwarded to the RIG `Agent::chat()` function.

```rust
// ⚠️ NOTE: This is PSEUDOCODE. `agent.clear_history()` and `agent.set_model()`
// do not exist as methods in rig-core's standard Agent struct.
// These operations require custom session wrapper logic around rig-core.
// See AGENT_RS.md for the ModelRouter and session state implementation pattern.
pub async fn handle_incoming_message(msg: String, session: &mut AgentSession) {
    // 1. Check if it's a command
    if msg.starts_with('/') {
        let parts: Vec<&str> = msg.split_whitespace().collect();
        match parts[0] {
            "/reset" => {
                session.clear_history().await;  // custom wrapper method
                send_to_user("Memory reset. 🧠");
            },
            "/model" if parts.len() > 1 => {
                let new_model = parts[1];
                session.set_model(new_model).await;  // custom wrapper method
                send_to_user(&format!("Switched to {}", new_model));
            },
            "/status" => {
                send_to_user("Status: Online | Model: GPT-4o | DB: Encrypted");
            },
            "/skills" if parts.len() > 1 => {
                // Handled by Skills registry (see SKILLS_RS.md)
                handle_skills_command(parts, session).await;
            },
            _ => {
                send_to_user("Unknown command.");
            }
        }
        return; // Don't send the command to the LLM!
    }

    // 2. If it's a normal message, let the LLM answer it
    let response = session.chat(&msg).await.unwrap();
    send_to_user(&response);
}
```

## 3. Native Integration (Discord & Slack)

While typing `/slash` commands works on every platform (even SMS and WhatsApp), platforms like Discord and Slack actually have **Native Command Menus** built into their UI.

If you are using the `serenity` crate for Discord, or the `reqwest` API for Slack, your Rust backend should make a one-time API call on boot to register these commands natively. This allows the user to type `/` in Discord and see a beautiful popup menu with autocomplete for `/reset` and `/model` that routes directly to your Rust backend.

## Summary

The Rust Orchestrator needs a **Command Router** that intercepts messages starting with `/`. This is completely independent of the LLM. It gives the user a "remote control" to configure and debug the agent directly from external messaging apps without needing the Tauri desktop UI.
