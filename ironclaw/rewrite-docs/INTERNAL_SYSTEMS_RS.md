# Internal Agent Systems: Heartbeats & Memory Documents

You have discovered the elements that give OpenClaw its "personality" and true autonomy.

Concepts like `SOUL.md`, `BOOTSTRAP.md`, `MEMORY.md`, and "Heartbeats" are not actually complex code. They are highly specific **prompting strategies** and **scheduling loops**.

Here is exactly what they do in OpenClaw, and how you should recreate them in your Rust `ThinClaw` agent.

---

## 1. The Core Documents (SOUL, BOOTSTRAP, MEMORY)

If you look inside the OpenClaw codebase, these are literally just `.md` (Markdown) files.

### `BOOTSTRAP.md`

When you install OpenClaw for the first time, it has a "First-Run Ritual." It places `BOOTSTRAP.md` in the agent's folder.

- **The Purpose:** It tells the LLM: _"You just woke up. You have no memory. Talk to the user, figure out your name, your personality, and what your purpose is. When you are done, write that info to `SOUL.md` and delete this file."_

### `SOUL.md` & `IDENTITY.md`

These are the agent's persistent personality files. In OpenClaw, before the agent is allowed to answer any chat message, the Orchestrator reads `SOUL.md` from the hard drive and pastes the entire contents into the hidden `System Prompt`.

- **The Purpose:** This provides the LLM continuous context on who it is, so it doesn't revert back to the default OpenAI "As an AI language model..." personality.

### How to Recreate This in Rust

You do not need to hardcode any complex logic for this. It is a simple file-reading loop in your Rust Orchestrator:

```rust
// 1. When the Rust Agent boots, check if SOUL.md exists
let soul_path = std::path::Path::new("~/.thinclaw/config/SOUL.md");

let system_prompt = if soul_path.exists() {
    // 2. If it exists, read it into the System Prompt
    std::fs::read_to_string(soul_path).unwrap()
} else {
    // 3. If it doesn't exist, read BOOTSTRAP.md and start the First-Run Ritual
    std::fs::read_to_string("~/.thinclaw/config/BOOTSTRAP.md").unwrap()
};

// 4. Build the RIG agent with that prompt
let agent = openai_client.agent("gpt-4o")
    .preamble(&system_prompt)
    .build();
```

---

## 2. Heartbeats (Proactive AI)

Most AI agents are **Reactive**: They just sit there asleep until you type a message to them.

OpenClaw is **Proactive**. It uses a system called "Heartbeats," which allows the agent to wake up, think, and take action _even when you aren't talking to it_.

### How OpenClaw Heartbeats Work

In the OpenClaw Node.js backend, there is a Cron scheduler (a timer). By default, every 15 minutes, the Orchestrator silently pings the LLM in the background.

It sends the LLM a hidden prompt that looks like this:

> _"SYSTEM HEARTBEAT: It is 2:00 PM. You have no new messages from the user. Review your memory, check if you have any scheduled tasks to execute, and reply quietly. If you need to alert the user, use the `send_message` tool."_

This is how the OpenClaw agent is able to suddenly message you: _"Hey, I noticed you have a meeting in 10 minutes, I prepared these notes for you."_ It didn't magically predict the future; it was woken up by a 15-minute cron job, noticed the meeting in its calendar context, and decided to send a message.

### How to Recreate This in Rust (`tokio-cron-scheduler`)

This is actually much cleaner to implement in Rust using the asynchronous runtime (`tokio`).

You use the `tokio-cron-scheduler` crate to run a background thread in your Tauri app.

```rust
use tokio_cron_scheduler::{Job, JobScheduler};

async fn start_heartbeat(agent: rig::Agent) {
    let sched = JobScheduler::new().await.unwrap();

    // Run this block of code every 15 minutes
    sched.add(Job::new_async("0 15 * * * *", move |_, _| {
        let agent_clone = agent.clone();
        Box::pin(async move {
            println!("Triggering Heartbeat...");

            // Send the hidden heartbeat prompt
            let response = agent_clone.chat(
                "[SYSTEM_HEARTBEAT] Review your context. If you need to work, use your tools silently."
            ).await.unwrap();

            // The agent might output a tool call to read emails, write code, or do nothing.
        })
    }).unwrap()).await.unwrap();

    sched.start().await.unwrap();
}
```

## 3. The Continuous Memory Loop

The final "magic" behavior of OpenClaw is how it manages `MEMORY.md`.

Because you give the agent a `write_file` tool (as we discussed in `AUTONOMY_RS.md`), the agent effectively manages its own long-term memory.

If you tell the agent: _"My dog's name is Buster."_

1. The agent's LLM reasons: _"I should remember that."_
2. The agent outputs a tool call: `{"tool": "write_file", "path": "MEMORY.md", "content": "- User's dog is named Buster (Added today)"}`
3. The Rust Orchestrator writes it.
4. On the next chat (or the next 15-minute Heartbeat), the Orchestrator automatically reads `MEMORY.md` from the disk and injects it into the System Prompt.

## Summary

You do not need to write complex AI logic to recreate OpenClaw's "Soul" or "Memory." You just need:

1. **File Tracking:** Make your Rust Orchestrator read `SOUL.md` and `MEMORY.md` into the System Prompt every time it builds an agent context.
2. **Tools:** Give the agent the ability to write to those files via your Rust tools.
3. **Heartbeat Loop:** Set up a 15-minute `tokio` background timer to quietly wake the LLM so it can process information and act without your prompting.

---

## 4. Backing Up Personality & Memory in Remote Mode (G7)

In **Local Mode**, `SOUL.md` and `MEMORY.md` live in the user's home directory (`~/.thinclaw/config/`). They are backed up naturally by Time Machine.

In **Remote Mode** (headless VPS), these files live on the *server's* filesystem. If the user wipes the VPS, migrates to a new provider, or the disk fails, the agent's entire personality and memory history is permanently lost.

### Recommended Strategy: Mirror Docs in the Encrypted Knowledge Base

Every time the Orchestrator writes to `SOUL.md` or `MEMORY.md` (because the agent used the `write_file` tool), it should simultaneously upsert those documents into the encrypted SQLite Knowledge Base (described in `KNOWLEDGE_BASE_RS.md`):

```rust
// After the agent writes SOUL.md to disk:
async fn sync_personality_to_db(content: &str, db: &KnowledgeBase) -> Result<()> {
    db.upsert_document(KbDocument {
        id:       "system:soul",
        content:  content.to_string(),
        metadata: json!({ "type": "personality", "updated_at": Utc::now() }),
    }).await
}
```

On boot, if `SOUL.md` is missing (e.g., after a server rebuild), the Orchestrator can restore it from the DB:

```rust
if !soul_path.exists() {
    if let Some(doc) = db.get_document("system:soul").await? {
        fs::write(&soul_path, doc.content)?;
        tracing::info!("Restored SOUL.md from encrypted knowledge base.");
    }
}
```

### Optional: Cloud Sync via `rclone`

For users who run multiple Orchestrators (e.g., home Mac Mini + cloud VPS), the `~/.thinclaw/` directory can be synced using `rclone` to an encrypted cloud bucket (B2, S3, etc.). The Orchestrator can invoke `rclone sync` as a post-write hook after memory updates. This ensures all instances share the same up-to-date personality while keeping data encrypted in transit and at rest.
