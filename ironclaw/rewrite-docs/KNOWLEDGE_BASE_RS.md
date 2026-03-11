# Building a Secure Knowledge Base (RAG) in ThinClaw

In a local-first AI application like ThinClaw, the **Knowledge Base** (often called the Agent Memory or RAG - Retrieval-Augmented Generation system) must be highly secure, performant, and completely contained on the user's hard drive.

A "Secure Knowledge Base" means the data never leaves the user's machine unless explicitly authorized, and the AI agent can privately query thousands of personal documents in milliseconds.

## 1. The Architecture of Local Memory

To build a knowledge base, you do not just drop text files into a folder. You need a **Vector Database**.
When the user adds a PDF or writes a note, the system converts that text into an "Embedding" (a long list of numbers representing the meaning of the text) and stores it in the database. When the user asks a question, the agent searches that database for the most relevant context and injects it into the prompt.

### The Problem with Cloud Knowledge Bases

Most tutorials tell you to use Pinecone or Supabase for vector storage, and OpenAI's `text-embedding-3-small` API to generate embeddings.

- **Security Risk:** This means every PDF, chat history, and private journal entry the user gives the agent is sent to OpenAI's servers to be embedded, and then sent to Pinecone's servers to be stored.

### The "ThinClaw" Secure Approach

For a privacy-first Rust/Tauri app, you must embed the database directly into the app binary and generate embeddings completely offline.

---

## 2. Choosing an Embedded Vector Database

Since ThinClaw is a Rust application, you have three fantastic options for an embedded database that requires zero setup from the user (no Docker, no background servers).

### Option A: SQLite + `sqlite-vec` (Highest Recommendation)

SQLite is the most robust, battle-tested database on earth. It runs as a single local `.db` file on your Mac. Using the `sqlite-vec` extension (or `rusqlite` in Rust), you can give SQLite native vector search capabilities.

- **Pros:** Extremely reliable, tiny file size, highly portable, easy to back up, ubiquitous.
- **Cons:** Slower than specialized vector DBs at tens of millions of rows, but plenty fast for a personal agent.

### Option B: LanceDB

LanceDB is an open-source vector database built explicitly for AI and written in Rust/C++. It runs entirely embedded inside your app (serverless) and saves data locally.

- **Pros:** Incredibly fast vector search designed from the ground up for RAG; natively built in Rust.
- **Cons:** Newer than SQLite, slightly steeper learning curve.

### Option C: SurrealDB

SurrealDB is a massive, multi-model database written entirely in Rust. It can run embedded in your Rust binary (RocksDB/TiKV backend). It supports vector search, graph relations, and document storage all at once.

- **Pros:** Hugely powerful; you can model complex relationships (e.g., "This Note belongs to this Project which is linked to this Chat Session").
- **Cons:** Overkill if you just need a simple place to dump paragraphs of text for the agent to read.

---

## 3. Local, Private Embeddings

To store data in your Secure Knowledge Base, you need to turn text into embeddings. **Do not use OpenAI for this.**

Since you already have `MLX` and `llama.cpp` sidecars, you should use a **local embedding model**.

- **The Model:** `nomic-embed-text` or `bge-small-en` (both are tiny, very fast, and run locally).
- **The Rust Implementation:** You can use the `candle-core` crate (by Hugging Face) or `mlx-rs` to load the embedding model directly inside your Rust backend.
- Whenever a user drops a file into ThinClaw, your Rust backend reads the file, passes it to the local `candle` model (which runs on the Mac's M-series GPU), and saves the resulting vector into SQLite. **Zero data leaves the machine.**

---

## 4. Encryption at Rest (The Final Security Layer)

If a user's laptop is stolen, someone could extract the SQLite `.db` file and read all their agent's chat histories and indexed documents.

To make the knowledge base truly "secure," you must implement **Encryption at Rest**.

1. **SQLCipher:** Instead of standard SQLite, use `sqlcipher` (via the `rusqlite` crate with the `sqlcipher` feature).
2. **The Key:** When the Tauri app boots, your Rust backend grabs a high-entropy encryption key from the macOS Keychain (using the `keyring` crate we discussed in `SECRETS_RS.md`).
3. **The Unlock:** Rust uses that secure key to silently decrypt and mount the SQLite database purely in memory.

If a hacker steals the `.db` file from the hard drive, it is mathematically impossible to read without the macOS Keychain encryption key (which requires the user's Touch ID or Mac password to unlock).

## Summary: The Ultimate Secure ThinClaw RAG

1. **Storage:** `rusqlite` + `sqlite-vec` (A single local file).
2. **Encryption:** `sqlcipher` (Encrypted at rest, key stored in macOS Keychain).
3. **Embeddings:** `candle-core` + `nomic-embed-text` (Zero-cloud data processing).
4. **Agent Access:** The RIG Agent queries the SQLite DB in Rust, formats the context strictly into the prompt, and handles requests without ever exposing the raw database to the Sandbox.
