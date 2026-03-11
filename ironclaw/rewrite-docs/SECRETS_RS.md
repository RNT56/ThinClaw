# Secrets & API Key Management (Node.js vs. Rust)

One of the most important aspects of an AI agent is handling the highly sensitive API keys (OpenAI, Anthropic, etc.) without leaking them or storing them insecurely.

Users **never** enter API keys directly into the source code. Doing so would mean you couldn't distribute your app (since your keys would be embedded in it) and every user would have to recompile the app just to use it.

## How OpenClaw (Node.js) Does It

OpenClaw evaluates secrets in a specific fallback order:

1. **Environment Variables:** Developers export `OPENAI_API_KEY` in their terminal.
2. **`openclaw.json5` Config:** Users can paste their keys in plaintext into the main configuration file located at `~/.openclaw/openclaw.json5`.
3. **`auth-profiles.json` (The Stateful DB):** OpenClaw maintains a dedicated JSON database in its state directory to keep track of multiple keys for the same provider, allowing it to mark keys that have hit rate limits and automatically switch to backup keys.

_The Problem:_ Storing API keys in plaintext JSON files on a user's hard drive is a security risk. If a malicious script reads `~/.openclaw/openclaw.json5`, the user's expensive AI API keys are compromised.

## How ThinClaw (Rust/Tauri) Should Do It

Because ThinClaw is a native macOS Rust application, you have access to vastly superior, OS-level security.

### 1. The Core Strategy: macOS Keychain (`SecretStore`)

Instead of saving keys in a `config.toml`, your Rust backend should utilize the macOS Keychain. This means the keys are encrypted by the operating system, and no rogue script can read them without triggering a "ThinClaw wants to access your keychain" macOS prompt.

You will use a crate like **`keyring`** to build a `SecretStore`.

```rust
use keyring::Entry;

pub struct SecretStore;

impl SecretStore {
    // Save a key securely to the macOS Keychain
    pub fn set_key(provider: &str, secret: &str) -> Result<(), keyring::Error> {
        let entry = Entry::new("thinclaw_api_keys", provider)?;
        entry.set_password(secret)
    }

    // Retrieve a key securely from the macOS Keychain
    pub fn get_key(provider: &str) -> Result<String, keyring::Error> {
        let entry = Entry::new("thinclaw_api_keys", provider)?;
        entry.get_password()
    }
}
```

### 2. The User Flow (Tauri UI to Rust Backend)

Since the user cannot put the key in code, how do they give it to the agent?

1. **The UI:** You build a "Settings" pane in your Tauri frontend (React/Vue/HTML). The user pastes their `sk-ant-api03...` key into an input field and clicks "Save".
2. **The IPC Command:** The frontend calls a Tauri command to pass the key to Rust.
   ```javascript
   // Frontend JS
   await invoke("save_api_key", { provider: "anthropic", key: "sk-ant-123..." });
   ```
3. **The Backend:** Your Rust Tauri command receives the plaintext key in memory and immediately hands it to `SecretStore`, which safely tucks it away in the macOS Keychain.
   ```rust
   #[tauri::command]
   fn save_api_key(provider: String, key: String) -> Result<(), String> {
       SecretStore::set_key(&provider, &key).map_err(|e| e.to_string())
   }
   ```
4. **The Agent Execution:** When your RIG `.agent("gpt-4o")` needs to make a web request, it asks `SecretStore::get_key("openai")` for the key. If it exists, the agent proceeds. If it doesn't, the agent throws an error, and the UI tells the user "Please add an OpenAI key in Settings."

### 3. Fallback to Environment Variables (For Devs)

To make local development easy, your agent should check environment variables _before_ checking the Keychain.

```rust
pub fn resolve_provider_key(provider: &str) -> Option<String> {
    // 1. Try environment variable first (e.g., OPENAI_API_KEY)
    let env_name = format!("{}_API_KEY", provider.to_uppercase());
    if let Ok(val) = std::env::var(&env_name) {
         return Some(val);
    }

    // 2. Try the secure SecretStore (macOS Keychain)
    if let Ok(val) = SecretStore::get_key(provider) {
         return Some(val);
    }

    None
}
```

## 4. Why OpenClaw did this differently (The Sandbox Leak Problem)

You might have noticed that in the original OpenClaw Node.js repository, there is a complex file called `src/agents/sandbox/sanitize-env-vars.ts`. Why did they need this?

In Node.js, when you spawn a child process to run a bash script, Node automatically passes _every single environment variable_ from the host (`process.env`) down into the child script.

Because OpenClaw users often put their `OPENAI_API_KEY` in their `.zshrc` or environment, the OpenClaw Node server would inherit it, and then accidentally pass it down to the Sandbox when the agent wrote a Python script!
If the agent wrote `print(os.environ)`, it could accidentally leak the user's secret keys into the chat window, or worse, send them to the internet.

To fix this massive security hole, the OpenClaw team had to write a giant `Regex` blacklist (`/^OPENAI_API_KEY$/i`, `/^ANTHROPIC.*/`, etc.) to manually scrub secrets out of the environment before launching the Docker container. This is called **"Insecure by Default"**.

**The Rust Fix (Secure by Default):**
When you launch a process in Rust using `std::process::Command`, you have complete control. If you map your `bollard` (Docker) or `wasmtime` (WASM) sandbox correctly, it starts completely empty. You do not have to write a massive regex blacklist because secrets (like the macOS Keychain) are never injected into the sandbox environment to begin with.

## Summary

- **Configuration (`config.toml`)** is for non-sensitive data: which model to use by default, system prompts, colors, and feature flags.
- **SecretStore (`macOS Keychain`)** is strictly for API Keys, Bearer tokens, and passwords.
- **Environment Variables** are a developer override.

---

## 5. Storing Multiple Keys Per Provider (Auth Key Rotation — G6)

The `AuthKeychain` in `AGENT_RS.md` needs to store multiple API keys for a single provider (e.g., two OpenAI keys for rotation). The macOS Keychain (and `keyring` crate) maps one `(service, account)` pair to one password value.

**The Solution: Indexed Keychain Entries**

Store multiple keys using indexed account names:

```rust
// Store key 1
SecretStore::set_key("openai_key_1", "sk-proj-key1...");
// Store key 2
SecretStore::set_key("openai_key_2", "sk-proj-key2...");
// Store count
SecretStore::set_key("openai_key_count", "2");
```

The `AuthKeychain` at boot reads all indexed keys into memory through a simple loading loop:

```rust
pub fn load_keys_for_provider(provider: &str) -> Vec<AuthProfile> {
    let count_key = format!("{}_key_count", provider);
    let count: usize = SecretStore::get_key(&count_key)
        .unwrap_or("0".into())
        .parse()
        .unwrap_or(0);

    (1..=count)
        .filter_map(|i| {
            let key_name = format!("{}_key_{}", provider, i);
            SecretStore::get_key(&key_name).ok().map(|secret| AuthProfile {
                id: key_name,
                secret,
                cooldown_until: None,
            })
        })
        .collect()
}
```

---

## 6. Headless Linux Key Storage (Q3 — Remote VPS / Server)

On a headless Linux VPS running `thinclaw-server`, **there is no macOS Keychain**. The `keyring` crate on Linux backends uses `libsecret` / `gnome-keyring`, which requires a running D-Bus session — not available on a minimal headless server.

**The Headless Key Storage Strategy (Tiered):**

**Tier 1: Linux Kernel Keyring (Best for session-level security)**
On modern Linux (>= 3.8), the `keyutils` system call provides an in-kernel encrypted keyring tied to the user's login session. Keys stored here are inaccessible between reboots and never written to disk.

```bash
# The thinclaw-server process uses keyctl to store secrets at setup time
keyctl add user thinclaw:openai_key_1 "sk-proj-..." @u
```

The Rust `keyring` crate can use this backend transparently on Linux via the `secret-service` provider.

**Tier 2: Argon2-Derived File Encryption (Best for persistent storage)**
If the user wants secrets to survive a reboot (common for unattended VPS deployment), ThinClaw generates an encrypted secrets file:

1. On first-run setup (`thinclaw-server --setup`), a passphrase is entered interactively.
2. An encryption key is derived: `let key = Argon2::default().hash(passphrase, &salt)?;`
3. Secrets are encrypted with AES-GCM and saved to `~/.thinclaw/secrets.enc`.
4. On each boot, the passphrase is prompted once (or provided via `systemd-creds` for fully automated deployments).

**Tier 3: Environment Variables (Developer / Container fallback)**
For Docker deployments or CI environments, secrets can always be provided as environment variables (`OPENAI_API_KEY`, etc.) and override Keychain/file storage. This is the same fallback described in Section 3 above.

**Recommended VPS Setup Flow:**
```bash
# During thinclaw-server initial setup:
> Enter a master passphrase for secret storage (leave blank to use env vars only): ****
> Passphrase confirmed. Secrets will be encrypted at ~/.thinclaw/secrets.enc
> To add API keys, run: thinclaw-server secret set openai sk-proj-...
# Or send them from the Tauri UI over the secure WebSocket (see NETWORKING_RS.md)
```

---

## 7. Transmitting API Keys from the Tauri UI to a Remote Orchestrator (Q4)

In Remote Mode, the user configures API keys via the Tauri Settings UI — but the keys must be stored on the *remote Orchestrator's* machine, not locally.

**The Flow:**
1. User opens Settings → API Keys in the Tauri Thin Client.
2. User types `sk-proj-...` into the OpenAI key field and clicks Save.
3. The Tauri frontend **never writes the key to local storage**. It immediately calls:

```rust
// Tauri frontend (JS side)
await invoke("transmit_remote_secret", {
    provider: "openai",
    keyIndex: 1,
    secret: "sk-proj-..."
});
```

4. The Rust Tauri backend sends a `secret.set` WebSocket message to the Remote Orchestrator:

```json
{
  "id": "...",
  "type": "secret.set",
  "payload": { "provider": "openai", "key_index": 1, "secret": "sk-proj-..." }
}
```

5. The Remote Orchestrator receives this message, calls `SecretStore::set_key("openai_key_1", secret)`, and responds with `secret.set.ack`.
6. The key is **now stored exclusively on the remote server's keychain**. If the user inspects the Tauri app's local storage, no API keys are present.

**Security:** The transmission travels over the Tailscale WireGuard-encrypted tunnel (see `NETWORKING_RS.md`). The connection is private and end-to-end encrypted at the network layer, providing equivalent security to HTTPS without requiring TLS certificate management.
