# Multimodal Capabilities (Images & Audio)

When building an AI agent that operates in chat applications like iMessage, Telegram, or Discord, it must be able to "see" and "hear" attachments sent by users.

In OpenClaw, this is handled by the `src/media-understanding` subsystem.

Because we are porting this to a local Rust backend, our approach to multimodal inputs changes depending on whether the user has chosen a **Cloud LLM (gpt-4o)** or a **Local LLM**.

## 1. The Interception Pipeline

When a user dragging-and-drops an image into the Tauri UI, or uploads a photo to a Telegram chat, your Rust Orchestrator must handle it _before_ the prompt hits the LLM.

**Step 1: Download & Cache**
When the channel adapter (e.g., Discord) receives a message with an attachment, download the bytes to `~/.thinclaw/temp/` and grab the MIME type (e.g., `image/jpeg`).

**Step 2: Format for the LLM**
_If the user is using `gpt-4o` or `claude-3-5-sonnet`:_
Instead of sending a standard string prompt to `rig-core`, you format the prompt as a multimodal array.

```rust
// Pseudocode for RIG multimodal prompt
let user_message = vec![
    MessageContent::Text("What is in this image?".to_string()),
    MessageContent::Image {
        format: "jpeg",
        base64_data: "...bytes...",
    }
];
agent.chat_multimodal(user_message).await;
```

## 2. Local-First Vision (MLX & Candle)

If the user is running ThinClaw entirely offline with a local model (like Llama-3-8B), standard LLMs _cannot_ see images.

If you want the agent to use computer vision offline, you must include a **Vision-Language Model (VLM)** in your Rust backend, such as `Llava` or `Moondream`.

**The Local Vision Flow:**

1. User uploads an image via Tauri UI while offline.
2. The Rust Orchestrator detects the local LLM cannot read images natively.
3. The Orchestrator spins up a secondary, lightweight VLM in the background using the `mlx-rs` (Apple Silicon) or `candle-core` framework.
4. The background VLM "looks" at the image and generates a text caption (e.g., _"This is a photograph of a red car parked near a tree."_).
5. The Orchestrator converts the image into text, and injects it into the prompt for the main LLM:
   `[User uploaded an image. Vision Assistant describes it as: "This is a photograph of a red car parked near a tree."] What's in this image?`

This matches OpenClaw's implementation, allowing graceful fallback when multimodal models are unavailable.

## 3. Audio Transcriptions (Whisper)

Users on iMessage or WhatsApp frequently send Voice Memos.

To process these:

1. The channel adapter downloads the `.m4a` or `.ogg` file.
2. The Rust Orchestrator uses a local Whisper model (via a crate like `whisper-rs` or `candle-transformers`) to instantly transcribe the audio into text _on-device_.
3. The transcribed text is sent to the LLM agent as if the user typed it:
   `[User sent an audio message]: "Hey agent, remind me to buy milk tomorrow."`

## 4. Text-to-Speech (TTS)

OpenClaw ships a full TTS pipeline (`src/tts/`, 66KB) that converts agent responses into spoken audio. This is essential for voice channels (voice calls, Discord voice, iMessage audio replies) and can be enabled per-channel.

### Providers

| Provider | Crate | Quality | Cost | Offline? |
|---|---|---|---|---|
| **Edge TTS** | `edge-tts` | Good | Free | No (Microsoft API) |
| **ElevenLabs** | `reqwest` (REST API) | Excellent | Paid | No |
| **OpenAI TTS** | `reqwest` (REST API) | Excellent | Paid | No |

### Auto Mode

TTS has four trigger modes, configurable per-agent and overridable per-channel:

- `"off"` — TTS disabled (default for text channels)
- `"always"` — Every agent response is converted to audio
- `"inbound"` — TTS activates only when the user sent an audio message (voice memo → voice reply)
- `"tagged"` — TTS activates only when the agent's text contains a `[tts]` directive tag

### Config

```toml
[tts]
auto = "inbound"         # Trigger mode
provider = "edge-tts"    # Default provider
max_length = 1500         # Max characters before summarizing
summarize = true          # Use a fast model to condense long responses before speaking

[tts.edge_tts]
voice = "en-US-AriaNeural"

[tts.elevenlabs]
api_key_ref = "keychain:elevenlabs_key"
voice_id = "pMsXgVXv3BLzUgSXRplE"
model_id = "eleven_multilingual_v2"

[tts.openai]
model = "gpt-4o-mini-tts"
voice = "alloy"
```

### Rust Implementation

```rust
pub enum TtsProvider { EdgeTts, ElevenLabs, OpenAi }

pub struct TtsService {
    config: ResolvedTtsConfig,
    client: reqwest::Client,
}

impl TtsService {
    /// Generate audio from text, returning the path to the output file
    pub async fn synthesize(&self, text: &str) -> Result<TtsResult> {
        // 1. Optionally summarize if text > max_length
        let speak_text = if text.len() > self.config.max_length && self.config.summarize {
            self.summarize_for_speech(text).await?
        } else {
            text.to_string()
        };

        // 2. Generate audio via selected provider
        let audio_bytes = match self.config.provider {
            TtsProvider::EdgeTts => {
                edge_tts::synthesize(&speak_text, &self.config.edge_tts.voice).await?
            },
            TtsProvider::ElevenLabs => {
                self.eleven_labs_synthesize(&speak_text).await?
            },
            TtsProvider::OpenAi => {
                self.openai_tts(&speak_text).await?
            },
        };

        // 3. Save to temp file and return path
        let path = temp_dir().join(format!("tts_{}.mp3", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, &audio_bytes).await?;
        Ok(TtsResult { audio_path: path, provider: self.config.provider })
    }
}
```

### Voice Selection & Telephony

- **Per-agent voices:** Each agent can have its own voice (configured in identity or config), creating distinct personalities across sub-agents.
- **Telephony output:** For voice call channels, audio is output in PCM format at 22050/24000 Hz (provider-dependent) instead of MP3.
- **Model override directives:** The LLM can include inline TTS directives in its response text to override voice/provider for a specific reply (e.g., changing accent for a joke).

### User Preferences

TTS preferences are stored per-user in a JSON file (`~/.thinclaw/tts-prefs.json`). Users can toggle TTS on/off via `/tts on|off` chat command or the Tauri Settings UI.

---

## Summary: The Media Pipeline

By handling Image parsing, Audio transcription, and Text-to-Speech centrally inside the Rust Orchestrator _before and after_ the RIG Agent runs, you guarantee that your agent can seamlessly operate across all 6 chat channels while fully supporting both Cloud and Local-Offline models.
