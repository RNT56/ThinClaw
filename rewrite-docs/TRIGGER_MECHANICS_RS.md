# Channel Trigger Mechanics (When to Reply)

If you connect an AI Agent directly to a bustling Slack or Discord workspace, disaster will strike if you do not implement proper **Trigger Mechanics**. Without them, the agent will attempt to reply to every single message sent by every human, rapidly exhausting API credits and annoying everyone.

If you explore the `src/auto-reply` and `src/routing` directories in OpenClaw, you will see exactly how to solve this. When rewriting ThinClaw in Rust, you must implement a "Gatekeeper" that runs _before_ the LLM is invoked.

## 1. The Three Rules of Engagement

For Headless Channels (Slack, Discord, Telegram), the Rust Orchestrator must intercept incoming messages and apply the following logic:

### A. Direct Messages (DMs)

- **Rule:** If the message is a 1-on-1 direct message between a human user and the Agent, the agent **always** replies.
- **Implementation:** The channel adapter flags the message as `ChatType::Direct`. The Orchestrator automatically routes it to the LLM.

### B. Group Chat Mentions

- **Rule:** If the message is in a group or public channel, the agent **only** replies if it is explicitly @mentioned or if its name is invoked.
- **Implementation:** You must pass the bot's username or user ID down to the channel adapters. Every incoming group message string is checked for `<@BOT_ID>` or regex variations of the agent's name. If found, the message is routed to the LLM. If not, it is silently dropped or cached in short-term history.

### C. Thread Bindings (Contextual Continunation)

- **Rule:** If a human replies directly to a message the agent previously sent in a group chat, the agent should reply back, _even if the human forgot to @mention the agent_.
- **Implementation:** When the agent sends a message to Discord or Slack, it receives a `message_id`. Your Rust backend must temporarily cache these `message_ids` in an LRU Cache or SQLite table (e.g., `active_threads`). When a new message arrives from the channel adapter, check if its `reply_to_id` matches an ID in your `active_threads` table. If it does, the agent replies.

## 2. Shared Group History (The "Eavesdropping" Buffer)

One of OpenClaw's best features is that the agent feels aware of the conversation even when it isn't speaking.

If humans are talking in a Slack channel for 20 messages, and then someone says:
_"@ThinBot, what do you think about what John just said?"_

The bot needs to know what John just said.

**How to implement this in Rust:**

1. Create a lightweight circular buffer (e.g., a `VecDeque<String>` with a maximum size of 50) in your Rust state, keyed by the `channel_id`.
2. When the Gatekeeper blocks a group message because the agent wasn't mentioned, **do not throw the message away**. Instead, append it to the circular buffer: `[John]: I think we should rewrite in Rust.`.
3. When the agent is finally @mentioned, grab the last 20 messages from the buffer, format them into a single string, and inject them into the LLM prompt as `<context>Recent Chat History...</context>`.

## 3. Session Isolation (Memory Spillage Prevention)

If you are using the same core RIG Agent for Telegram, Discord, and Local Tauri chats, you must prevent the agent from accidentally leaking a Discord conversation into a Telegram chat.

As seen in OpenClaw's `src/routing/session-key.ts`, every interaction must be aggressively bucketed by a **Session Key**.

Your Rust `Agent` implementation should dynamically load its memory database using a composite string:
`agent:{agent_id}:{channel}:{peer_id}`
(e.g., `agent:main:discord:user1234`)

When replying, the Orchestrator pulls _only_ the memory rows matching that exact Session Key.

## Summary Checklist for Rust Migration

- [ ] Implement `is_direct_message()` flag in your channel adapters.
- [ ] Implement `contains_bot_mention()` regex parsing.
- [ ] Implement `active_threads` LRU Cache for contextual replies without mentions.
- [ ] Implement `VecDeque` unmentioned history eavesdropping buffer per-channel.
- [ ] Enforce strict `SessionKey` generation to guarantee memory isolation between platforms and users.
