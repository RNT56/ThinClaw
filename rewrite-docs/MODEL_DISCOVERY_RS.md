> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# Model & Provider Discovery in Rust

To provide a seamless experience, `ThinClaw` must dynamically discover available AI models rather than relying on a hardcoded list. This applies to both **Local Inference Engines** (fetching weights from Hugging Face) and **Cloud Providers** (OpenAI, Anthropic, OpenRouter).

Because the Rust Orchestrator acts as the "Gateway," it is responsible for querying these endpoints, standardizing the model lists, and providing a clean API to the Tauri Frontend.

---

## 1. Local Model Discovery (Hugging Face Hub)

For local execution, ThinClaw relies on underlying inference engines like **MLX** (for Apple Silicon) or **llama.cpp** (for broader hardware support via GGUF).

### The Rust Implementation

The Rust Orchestrator will implement a Hugging Face Hub client to search for compatible models using the `reqwest` HTTP crate.

```rust
use reqwest::Client;
use serde::Deserialize;

#[derive(Deserialize)]
struct HfModel {
    id: String,
    downloads: u32,
    tags: Vec<String>,
}

pub async fn discover_local_models(engine: &str, is_multimodal: bool) -> Result<Vec<HfModel>, reqwest::Error> {
    let client = Client::new();
    
    // Build the query. Example: searching for 'mlx' or 'gguf'
    let mut query = format!("https://huggingface.co/api/models?sort=downloads&direction=-1&limit=50");
    
    if engine == "mlx" {
        query.push_str("&search=mlx");
    } else if engine == "llama.cpp" {
        query.push_str("&search=gguf");
    }

    // Filter for multimodal vision models if requested
    if is_multimodal {
        query.push_str("&filter=image-text-to-text");
    } else {
        query.push_str("&filter=text-generation");
    }

    let response = client.get(&query).send().await?;
    let models: Vec<HfModel> = response.json().await?;
    
    Ok(models)
}
```

**Key Features to Replicate:**
- **Engine Tagging:** The query filters results based on the active local inference engine (e.g., ensuring MLX models are only shown on Apple Silicon).
- **Capability Filtering:** Separating standard text models from multimodal (`image-text-to-text`) models.
- **Sorting:** Prioritizing models by download count ensures the user sees the most popular, community-validated quantizations first.

---

## 2. Cloud Provider Auto-Discovery

Cloud APIs frequently release new model versions (e.g., `gpt-4o`, `claude-3.5-sonnet`). Instead of updating the ThinClaw application every time a provider releases a model, the Rust backend will dynamically query the provider's `/v1/models` endpoint (or equivalent).

### The Rust Implementation

Most major providers (OpenAI, OpenRouter, Groq, Together) adhere to the OpenAI API specification for model discovery. Anthropic and Google (Gemini) have their own specific endpoints, requiring a trait-based approach in Rust.

```rust
use async_trait::async_trait;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredModel {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub context_length: Option<u32>,
}

#[async_trait]
pub trait ModelProvider {
    async fn fetch_available_models(&self, api_key: &str) -> Result<Vec<DiscoveredModel>, String>;
}

// Example: OpenAI / OpenRouter Standard Payload
pub struct OpenAiCompatibleProvider {
    pub base_url: String,
    pub provider_name: String,
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    async fn fetch_available_models(&self, api_key: &str) -> Result<Vec<DiscoveredModel>, String> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/models", self.base_url);
        
        // Fetch JSON and map to standard DiscoveredModel format
        // ... (HTTP request logic using Bearer token)
        Ok(vec![]) // Return parsed models
    }
}
```

### Supported Providers
- **OpenAI Compatible:** OpenAI, OpenRouter, Groq, local API servers (LM Studio, Ollama).
- **Custom Rest Mappings:** Anthropic, Google Gemini API, AWS Bedrock.

---

## 3. Serving the Tauri Frontend

The Tauri app needs a unified list of all available models (both downloaded locally, available on Hugging Face, and available via configured Cloud API keys).

The Orchestrator provides a Tauri Command (or a WebSocket RPC event in Remote Mode):

```rust
#[tauri::command]
pub async fn get_all_available_models(
    state: tauri::State<'_, AppState>
) -> Result<Vec<DiscoveredModel>, String> {
    // 1. Check local disk for downloaded weights
    // 2. Fetch configured API keys from SecretStore (Keychain)
    // 3. Concurrently fetch cloud models (OpenRouter, OpenAI)
    // 4. Return unified list to the Svelte/React UI
    
    Ok(unified_list)
}
```

### Remote Mode Considerations
When the Tauri app is acting as a "Thin Client" connected to a remote headless Ubuntu server:
1. The Tauri UI requests the model list via WebSocket.
2. The **Remote Host** executes the Hugging Face and Cloud API discovery queries.
3. The Remote Host returns the list back down the WebSocket.
_This ensures the UI never needs direct internet access to Hugging Face or Cloud APIs, perfectly honoring the secure "Gateway" architecture._
