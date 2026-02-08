Scrappy – Phase 2 Enhancements Specification
Version: 1.0 (Draft) Prerequisites: Canonical Architecture (Phase 1) Implementation Goal: Elevate Scrappy from a functional tool to a production-grade, highly performant, and secure desktop application.

1. Global Quick-Access & Background Lifecycle
Objective: Transform Scrappy into an "always-available" utility (like Spotlight or Raycast) rather than a standard windowed app, while maintaining VRAM state for instant responses.
1.1 Architecture
The application must decouple the Window Lifecycle from the Process Lifecycle.
Process: Stays alive in the background (System Tray) to keep llama-server loaded in VRAM.
Window: Toggles visibility on demand.
1.2 Implementation Details
A. System Tray (Keep-Alive)
Use tauri-plugin-tray-icon (Tauri v2) to manage background persistence.
Logic:
On Close Requested (User clicks 'X'): Do not exit. calling window.hide() instead.
On Tray Icon Click: Toggle window visibility.
On Tray Menu -> Quit: Actually terminate the application and kill the sidecar.
B. Global Hotkey (Toggle)
Use tauri-plugin-global-shortcut.
Config: Register Alt+Space (Windows/Linux) or Cmd+Space (macOS - user configurable recommended due to conflicts).
Behavior: Rust  // Rust Pseudo-code (Tauri Command)
if window.is_visible() {
    window.hide();
} else {
    window.show();
    window.set_focus();
    // Trigger frontend focus on input field
    window.emit("focus_input", {});
}
   

2. Persistent Context (Disk-Based Prompt Caching)
Objective: eliminate the 10–30 second "processing" delay when resuming long conversations after an app restart.
2.1 The Mechanic
llama-server supports saving the evaluated prompt state (KV cache) to disk.
2.2 Implementation Strategy
A. Cache Directory Management
Location: %APPDATA%/Scrappy/cache/
Naming Convention: {conversation_id}.bin
Lifecycle Rules:
Create/Update: On chat exit or periodic auto-save.
Load: When opening a chat, check for existence.
Cleanup: Implement an LRU (Least Recently Used) policy. If cache folder > 5GB, delete oldest files.
B. Sidecar Flag Injection
When the Rust backend spawns a llama-server instance for a specific chat session:
Rust

// In Rust Sidecar Manager
let cache_path = format!(".../{}.bin", conversation_id);
let args = vec![
    // ... basic flags ...
    "--prompt-cache", &cache_path,
    "--prompt-cache-all", // Save user inputs too, not just system prompt
];
 Note on Citations: While the foundational docs mention using --keep N to retain cache in RAM, this enhancement extends that capability to disk persistence using standard llama.cpp features.  

3. Security: Streaming Sanitization Pipeline
Objective: Prevent Cross-Site Scripting (XSS) attacks where a malicious prompt injection or model hallucination causes the execution of arbitrary JavaScript via the chat interface.
3.1 The Vulnerability
The frontend receives Markdown chunks. If a chunk contains <img src=x onerror=alert(1)>, rendering it unsafe could compromise the host system via WebView privileges.
3.2 The Pipeline
Strict Rule: Never sanitize chunks in isolation. Always sanitize the rendered HTML buffer.
Stream Accumulator: Collect raw tokens into a buffer.
Markdown Parse: Convert buffer to HTML (e.g., using marked or markdown-it).
Sanitization (Critical Step):
Library: DOMPurify (configured for strictly allowing only safe tags like p, b, code, pre, table).
Hook: DOMPurify.sanitize(rawHtml, { FORBID_TAGS: ['script', 'iframe', 'object', 'embed'] }).
Render: Inject result into the DOM.
3.3 Backend Hardening (Rust)
 Input Validation: As noted in best practices, validate JSON payloads before sending to the sidecar.  
CSP: Enforce a strict Content Security Policy in tauri.conf.json preventing external script loading.

4. Advanced RAG: Dynamic Alpha Fusion
Objective: Automatically adjust the search strategy based on the user's intent, solving the "Keyword vs. Concept" problem.
4.1 The Algorithm: Linear Weighted Fusion
Instead of a static list, we calculate a final score for every document chunk:
Scorefinal =(1−α)⋅ScoreBM25 +(α)⋅ScoreVector
α (Alpha): A value between 0.0 and 1.0.
0.0: Pure Keyword Search (FTS5).
1.0: Pure Vector Search (Semantic).
0.5: Balanced Hybrid (Default).
4.2 Dynamic Tuning Logic (Rust)
Before executing the search, analyze the query string q:
Code Detection: If q matches regex for code patterns (e.g., error 0x, snake_case_func, camelCaseVar), set α=0.2 (Favor Exact Match).
Quote Detection: If q contains "quoted phrases", set α=0.1 (Strongly Favor Exact Match).
Natural Language: If q > 5 words and no quotes, set α=0.6 (Favor Semantic).
Fallback: Default to α=0.5.
 Citation Context: This enhances the base hybrid search architecture described in the design document.   

5. Developer Experience: Mock Mode
Objective: Allow UI/Feature development on laptops without GPUs or battery drain by simulating the AI backend.
5.1 Rust Trait Architecture
Define a trait that abstracts the AI server interactions.
Rust

#[async_trait]
pub trait ModelClient: Send + Sync {
    async fn stream_completion(&self, request: ChatRequest) -> Result<Receiver<String>>;
    async fn health_check(&self) -> bool;
}
5.2 Implementations
A. RealClient (Production)
Connects to 127.0.0.1:<port>.  
Sends authentic HTTP requests.
B. MockClient (Dev)
Activated by: cargo run --features mock
Behavior:
Ignores input prompt.
Returns hardcoded "Lorem Ipsum" or specific markdown test cases (tables, code blocks).
Simulation: Adds a sleep(30ms) between chunks to mimic token streaming generation speed.
5.3 Frontend Indicator
When in Mock Mode, the UI Top Bar should display a prominent "MOCK MODE" badge to prevent confusion during testing.

6. Implementation Checklist (Priority Order)
[ ] Global Hotkey & Tray: Essential for "daily driver" usability.
[ ] Mock Mode: Implement immediately to speed up all subsequent UI work.
[ ] Streaming Sanitization: Critical security blocker before first release.
[ ] Dynamic Alpha RAG: Implement during the RAG tuning phase.
[ ] Prompt Caching: Optimization to be added once core stability is achieved.
