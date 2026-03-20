> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# Architecture Comparison: OpenClaw vs. Your ThinClaw (Tauri)

It is completely understandable why the architecture differences are confusing! You are bridging two fundamentally different ways of building a desktop application.

Here is a plain-English breakdown of how OpenClaw is designed, how your app is currently designed, and where the agent should actually live.

---

## The OpenClaw Architecture (Client-Server Model)

OpenClaw is built like a traditional web application, even though it runs locally on your Mac.

1. **The Backend (Node.js):** This is the massive `openclaw` folder full of TypeScript. It is the true "Brain". It hosts the Pi Agent, talks to the APIs, connects to Discord/Telegram, and runs the web server. It runs as a detached background process (a daemon).
2. **The Frontend (Swift UI macOS App):** This is a completely "dumb" client. It knows _nothing_ about AI. It is literally just a WebSocket client that connects to `localhost:18789` (the Node.js backend). When you type a chat message, it sends it to Node.js. When it needs to take a photo, Node.js tells it to open the macOS camera.

**Why did they do this?** Because they wanted the exact same Node.js backend code to power the macOS app, the iOS app, the Android app, and headless web servers.

---

## Your ThinClaw Architecture (The "Local-First" Tauri Model)

You are building a **Tauri** application. Tauri is uniquely powerful because it merges the Frontend and the Backend into a **single, unified application binary**.

Here is what your architecture looks like right now:

1. **The Frontend (HTML/JS/React/Vue):** This is your gorgeous GUI. It runs in a WebView (like a lightweight Chrome browser) managed by Tauri.
2. **The Backend (Rust):** This is the core of your Tauri app (`src-tauri/src/main.rs`). It acts as the bridge between your web frontend and the actual Mac operating system.
3. **The Sidecars (C++ Binaries):** You have bundled `llama.cpp` (for text), `sd.cpp` (for images), and `MLX` as separate executable files that your Rust backend launches in the background so you can run AI locally without internet.

---

## Where Should the Agent Live?

**The short answer: The Agent should live _inside your Tauri Rust backend_.**

You do **NOT** need a separate Rust project, and you absolutely do **NOT** need to keep the OpenClaw Node.js sidecar once you finish the rewrite.

Here is the exact flow of how it should work in your Tauri app:

### 1. The User Types a Message

- The user types "Generate an image of a cat" in your React/Vue frontend.
- The frontend uses Tauri's IPC (Inter-Process Communication) to call a Rust function: `invoke("chat_message", { text: "Generate..." })`.

### 2. The Rust Backend (The Brains)

- Your Tauri Rust backend receives the message string.
- **Your RIG Agent lives right here, in the same Rust process.**
- The RIG agent looks at the prompt and says, "Aha! I need to use the Image Generation Tool."

### 3. The Tool Execution (The Sidecars)

- The RIG Agent (which is just Rust code) triggers its `image_gen_tool`.
- That tool uses Rust's `std::process::Command` to talk to the `sd.cpp` sidecar you already have running in the background.
- `sd.cpp` generates the image and saves it to the hard drive.

### 4. The Response

- The RIG Agent gets the file path of the new image.
- The Rust backend streams the response back to your React/Vue frontend over Tauri's IPC channel.
- The frontend displays the cat picture.

---

## Why this is vastly superior to OpenClaw

By putting the RIG Agent directly inside the Tauri Rust backend, you eliminate incredible amounts of complexity:

1. **No Websockets Needed:** You don't have to manage a fragile WebSocket connection between a dumb UI and a Node.js backend. Tauri's IPC handles communication between your frontend and your Rust backend frictionlessly.
2. **Direct OS Access:** When the agent needs to read a file or run a terminal command, its native Rust tools can execute it instantly. OpenClaw has to serialize that command, send it over a WebSocket to the Swift app, have Swift execute it, and send the result back to Node.js.
3. **True Local Processing:** Because you are distributing `llama.cpp` and `sd.cpp` as sidecars managed by your Tauri app, the user downloads _one_ installer (the `.dmg`), opens it, and they instantly have a fully fledged, local AI agent. No Node.js required.

### Next Steps

To implement this, you will simply add the `rig-core` library directly to your Tauri app's `Cargo.toml`. Your Agent will be a standard struct instantiated in your Tauri `main.rs` setup function, perfectly bridging your UI, your APIs, and your local C++ sidecars.
