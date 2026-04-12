# LLM Provider Configuration

ThinClaw supports **20+ inference providers** out of the box. It defaults to
`openai_compatible` so it works with any OpenAI-compatible endpoint, and ships
a built-in provider catalog that auto-resolves base URLs, API keys, and model
defaults for popular cloud providers.

## Provider Overview

### Native Backends

These providers have dedicated `LLM_BACKEND` values with first-class support:

| Provider | `LLM_BACKEND` | API Key Env | Default Model |
|---|---|---|---|
| [Anthropic](#anthropic) | `anthropic` | `ANTHROPIC_API_KEY` | `claude-sonnet-4-20250514` |
| [OpenAI](#openai) | `openai` | `OPENAI_API_KEY` | `gpt-4o` |
| [Google Gemini](#google-gemini) | `gemini` | `GEMINI_API_KEY` | `gemini-3.1-flash` |
| [Tinfoil](#tinfoil) | `tinfoil` | `TINFOIL_API_KEY` | `kimi-k2-5` |
| [Ollama](#ollama) | `ollama` | — | `llama3` |
| [AWS Bedrock](#aws-bedrock) | `bedrock` | `BEDROCK_API_KEY` | `anthropic.claude-3-sonnet-20240229-v1:0` |
| [llama.cpp](#llamacpp) | `llama_cpp` | — | `llama-local` |
| [OpenAI-Compatible](#openai-compatible-endpoints) | `openai_compatible` | `LLM_API_KEY` | — |

### Cloud Catalog Providers

These providers are registered in the built-in catalog. ThinClaw auto-resolves
their base URL and defaults when you enable them via the setup wizard or
`providers.enabled` settings. They all use the OpenAI-compatible backend
internally.

| Provider | Slug | API Key Env | Default Model |
|---|---|---|---|
| [OpenRouter](#openrouter) | `openrouter` | `OPENROUTER_API_KEY` | `anthropic/claude-sonnet-4-20250514` |
| [Groq](#groq) | `groq` | `GROQ_API_KEY` | `llama-3.3-70b-versatile` |
| [Mistral AI](#mistral-ai) | `mistral` | `MISTRAL_API_KEY` | `mistral-large-latest` |
| [xAI (Grok)](#xai-grok) | `xai` | `XAI_API_KEY` | `grok-3` |
| [DeepSeek](#deepseek) | `deepseek` | `DEEPSEEK_API_KEY` | `deepseek-chat` |
| [Together AI](#together-ai) | `together` | `TOGETHER_API_KEY` | `meta-llama/Llama-3.3-70B-Instruct-Turbo` |
| [Cerebras](#cerebras) | `cerebras` | `CEREBRAS_API_KEY` | `llama-3.3-70b` |
| [Cohere](#cohere) | `cohere` | `COHERE_API_KEY` | `command-a-03-2025` |
| [NVIDIA NIM](#nvidia-nim) | `nvidia` | `NVIDIA_API_KEY` | `meta/llama-3.3-70b-instruct` |
| [Venice AI](#venice-ai) | `venice` | `VENICE_API_KEY` | `llama-3.3-70b` |
| [MiniMax](#minimax) | `minimax` | `MINIMAX_API_KEY` | `MiniMax-M2.7` |
| [Moonshot (Kimi)](#moonshot-kimi) | `moonshot` | `MOONSHOT_API_KEY` | `moonshot-v1-auto` |
| [Perplexity](#perplexity) | `perplexity` | `PERPLEXITY_API_KEY` | `sonar-pro` |
| [GLM (Zhipu)](#glm-zhipu) | `glm` | `GLM_API_KEY` | `glm-4-plus` |

### Self-Hosted / Local Proxies

These use `LLM_BACKEND=openai_compatible` with a local base URL:

| Provider | Notes |
|---|---|
| [vLLM](#vllm--litelllm) | High-performance local inference server |
| [LiteLLM](#vllm--litelllm) | Proxy to Bedrock, Vertex, Azure, etc. |
| [LM Studio](#lm-studio) | Local GUI with built-in server |
| [Fireworks AI](#fireworks-ai) | Fast cloud inference |

---

## Native Backends

### Anthropic

```env
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-...
```

Popular models: `claude-sonnet-4-20250514`, `claude-opus-4-5`, `claude-haiku-4-5`

Optional overrides:

| Variable | Default | Notes |
|---|---|---|
| `ANTHROPIC_MODEL` | `claude-sonnet-4-20250514` | Model override |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com/v1` | For proxies |

---

### OpenAI

```env
LLM_BACKEND=openai
OPENAI_API_KEY=sk-...
```

Popular models: `gpt-4o`, `gpt-4o-mini`, `o3`, `o4-mini`

Optional overrides:

| Variable | Default | Notes |
|---|---|---|
| `OPENAI_MODEL` | `gpt-4o` | Model override |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | For proxies |

---

### Google Gemini

```env
LLM_BACKEND=gemini
GEMINI_API_KEY=...
```

Also accepts `GOOGLE_AI_API_KEY`. Routes through Google's OpenAI-compatible
gateway by default.

Popular models: `gemini-3.1-flash`, `gemini-2.5-flash`, `gemini-2.5-pro`

Optional overrides:

| Variable | Default | Notes |
|---|---|---|
| `GEMINI_MODEL` | `gemini-3.1-flash` | Model override |
| `GEMINI_BASE_URL` | `https://generativelanguage.googleapis.com/v1beta/openai` | Custom endpoint |

---

### Tinfoil

```env
LLM_BACKEND=tinfoil
TINFOIL_API_KEY=...
```

Private/confidential inference. Default model: `kimi-k2-5`.

| Variable | Default | Notes |
|---|---|---|
| `TINFOIL_MODEL` | `kimi-k2-5` | Model override |

---

### Ollama

Install Ollama from [ollama.com](https://ollama.com), pull a model, then:

```env
LLM_BACKEND=ollama
OLLAMA_MODEL=llama3.2
# OLLAMA_BASE_URL=http://localhost:11434   # default
```

Pull a model first: `ollama pull llama3.2`

---

### AWS Bedrock

ThinClaw supports AWS Bedrock via native Mantle endpoints and legacy proxy mode.

```env
LLM_BACKEND=bedrock
AWS_REGION=us-east-1
BEDROCK_API_KEY=...
BEDROCK_MODEL_ID=anthropic.claude-3-sonnet-20240229-v1:0
```

Optional overrides:

| Variable | Default | Notes |
|---|---|---|
| `AWS_REGION` | `us-east-1` | AWS region |
| `AWS_ACCESS_KEY_ID` | — | IAM access key (alternative to `BEDROCK_API_KEY`) |
| `AWS_SECRET_ACCESS_KEY` | — | IAM secret key |
| `BEDROCK_PROXY_URL` | — | Legacy proxy mode URL |
| `BEDROCK_PROXY_API_KEY` | — | Legacy proxy auth token |
| `BEDROCK_MAX_TOKENS` | `4096` | Max output tokens |

---

### llama.cpp

Local GGUF inference via a `llama.cpp` server (OpenAI-compatible mode).

```env
LLM_BACKEND=llama_cpp
LLAMA_SERVER_URL=http://localhost:8080
LLAMA_MODEL=llama-local
```

Optional overrides:

| Variable | Default | Notes |
|---|---|---|
| `LLAMA_MODEL_PATH` | — | Path to `.gguf` model file |
| `LLAMA_CONTEXT_LENGTH` | `4096` | Context window |
| `LLAMA_GPU_LAYERS` | `0` | GPU layers to offload (`-1` = all) |

> **Note:** Requires the `llama-cpp` feature at compile time, or use the
> server's OpenAI-compatible endpoint with `LLM_BACKEND=openai_compatible`.

---

## OpenAI-Compatible Endpoints

All providers below use `LLM_BACKEND=openai_compatible`. Set `LLM_BASE_URL` to the
provider's OpenAI-compatible endpoint and `LLM_API_KEY` to your API key.
ThinClaw also accepts provider-specific aliases such as `OPENROUTER_API_KEY`
for catalog-driven routing and provider selection.

### OpenRouter

[OpenRouter](https://openrouter.ai) routes to 300+ models from a single API key.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

Popular OpenRouter model IDs:

| Model | ID |
|---|---|
| Claude Sonnet 4 | `anthropic/claude-sonnet-4` |
| GPT-4o | `openai/gpt-4o` |
| Llama 4 Maverick | `meta-llama/llama-4-maverick` |
| Gemini 2.0 Flash | `google/gemini-2.0-flash-001` |
| Mistral Small | `mistralai/mistral-small-3.1-24b-instruct` |

Browse all models at [openrouter.ai/models](https://openrouter.ai/models).

---

### Groq

[Groq](https://groq.com) provides ultra-fast inference on custom LPU hardware.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.groq.com/openai/v1
LLM_API_KEY=gsk_...
LLM_MODEL=llama-3.3-70b-versatile
```

Popular Groq models: `llama-3.3-70b-versatile`, `llama-3.1-8b-instant`, `gemma2-9b-it`

---

### Mistral AI

[Mistral AI](https://mistral.ai) offers frontier models and fast open-weight inference.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.mistral.ai/v1
LLM_API_KEY=...
LLM_MODEL=mistral-large-latest
```

Popular models: `mistral-large-latest`, `mistral-small-latest`, `codestral-latest`

---

### xAI (Grok)

[xAI](https://x.ai) provides inference for Grok models.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.x.ai/v1
LLM_API_KEY=xai-...
LLM_MODEL=grok-3
```

Popular models: `grok-3`, `grok-3-mini`

---

### DeepSeek

[DeepSeek](https://deepseek.com) offers high-performance reasoning models.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.deepseek.com/v1
LLM_API_KEY=sk-...
LLM_MODEL=deepseek-chat
```

Popular models: `deepseek-chat`, `deepseek-reasoner`

---

### Together AI

[Together AI](https://www.together.ai) provides fast inference for open-source models.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.together.xyz/v1
LLM_API_KEY=...
LLM_MODEL=meta-llama/Llama-3.3-70B-Instruct-Turbo
```

Popular Together AI model IDs:

| Model | ID |
|---|---|
| Llama 3.3 70B | `meta-llama/Llama-3.3-70B-Instruct-Turbo` |
| DeepSeek R1 | `deepseek-ai/DeepSeek-R1` |
| Qwen 2.5 72B | `Qwen/Qwen2.5-72B-Instruct-Turbo` |

---

### Cerebras

[Cerebras](https://cerebras.ai) provides fast wafer-scale inference.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.cerebras.ai/v1
LLM_API_KEY=...
LLM_MODEL=llama-3.3-70b
```

---

### Cohere

[Cohere](https://cohere.com) offers enterprise-focused models, including
Command A with 256K context.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.cohere.ai/compatibility/v1
LLM_API_KEY=...
LLM_MODEL=command-a-03-2025
```

Popular models: `command-a-03-2025`, `command-r-plus-08-2024`, `command-r7b-12-2024`

---

### NVIDIA NIM

[NVIDIA](https://build.nvidia.com) offers inference via its NIM platform.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://integrate.api.nvidia.com/v1
LLM_API_KEY=nvapi-...
LLM_MODEL=meta/llama-3.3-70b-instruct
```

---

### Venice AI

[Venice AI](https://venice.ai) offers private, uncensored inference.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.venice.ai/api/v1
LLM_API_KEY=...
LLM_MODEL=llama-3.3-70b
```

---

### MiniMax

[MiniMax](https://www.minimax.io) provides large-context models (up to 1M tokens).

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.minimax.io/v1
LLM_API_KEY=...
LLM_MODEL=MiniMax-M2.7
```

Popular models: `MiniMax-M2.7`, `MiniMax-M2.5`, `MiniMax-M2.5-highspeed`

---

### Moonshot (Kimi)

[Moonshot](https://moonshot.ai) provides long-context inference with Kimi models.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.moonshot.ai/v1
LLM_API_KEY=...
LLM_MODEL=moonshot-v1-auto
```

---

### Perplexity

[Perplexity](https://perplexity.ai) provides search-augmented inference.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.perplexity.ai
LLM_API_KEY=pplx-...
LLM_MODEL=sonar-pro
```

> **Note:** Perplexity has limited tool/function calling support.

---

### GLM (Zhipu)

[Zhipu GLM](https://open.bigmodel.cn) provides Chinese-optimized LLMs.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://open.bigmodel.cn/api/paas/v4
LLM_API_KEY=...
LLM_MODEL=glm-4-plus
```

---

### Fireworks AI

[Fireworks AI](https://fireworks.ai) offers fast inference with compound AI system support.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.fireworks.ai/inference/v1
LLM_API_KEY=fw_...
LLM_MODEL=accounts/fireworks/models/llama4-maverick-instruct-basic
```

---

### vLLM / LiteLLM

For self-hosted inference servers:

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://localhost:8000/v1
LLM_API_KEY=token-abc123        # set to any string if auth is not configured
LLM_MODEL=meta-llama/Llama-3.1-8B-Instruct
```

LiteLLM proxy (forwards to any backend, including Bedrock, Vertex, Azure):

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://localhost:4000/v1
LLM_API_KEY=sk-...
LLM_MODEL=gpt-4o                 # as configured in litellm config.yaml
```

---

### LM Studio

Start LM Studio's local server, then:

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://localhost:1234/v1
LLM_MODEL=llama-3.2-3b-instruct-q4_K_M
# LLM_API_KEY is not required for LM Studio
```

---

## Extra Headers

Some providers require custom HTTP headers. Use `LLM_EXTRA_HEADERS` to inject
them into every LLM request:

```env
LLM_EXTRA_HEADERS=HTTP-Referer:https://myapp.com,X-Title:MyApp
```

Format: `Key:Value` pairs separated by commas.

---

## Reliability & Routing

These settings apply to **all providers**:

| Variable | Default | Notes |
|---|---|---|
| `LLM_CHEAP_MODEL` | — | Cheap/fast model for heartbeat, routing, evaluation |
| `LLM_FALLBACK_MODEL` | — | Fallback model for automatic failover |
| `LLM_MAX_RETRIES` | `3` | Max retries for transient errors |
| `CIRCUIT_BREAKER_THRESHOLD` | — | Consecutive failures before circuit opens |
| `CIRCUIT_BREAKER_RECOVERY_SECS` | `30` | Recovery timeout for circuit breaker |
| `RESPONSE_CACHE_ENABLED` | `false` | In-memory response caching |
| `RESPONSE_CACHE_TTL_SECS` | `3600` | Cache TTL |
| `LLM_FAILOVER_COOLDOWN_SECS` | `300` | Provider cooldown after failures |
| `SMART_ROUTING_CASCADE` | `true` | Re-send uncertain cheap responses to primary |

---

## Using the Setup Wizard

Instead of editing `.env` manually, run the onboarding wizard:

```bash
thinclaw onboard
```

The wizard covers all native backends (Anthropic, OpenAI, Gemini, Tinfoil,
Ollama, AWS Bedrock, llama.cpp, OpenRouter, OpenAI-compatible) and lets you
select models interactively with live discovery from the provider's API.

For multi-provider routing and failover, use the gateway settings page or
configure `providers.enabled` and `providers.fallback_chain` in the settings
database.
