# LLM Provider Configuration

ThinClaw defaults to `openai_compatible` so it works with any OpenAI-compatible
endpoint out of the box. It also supports Anthropic, OpenAI, Ollama, and Tinfoil
for private inference. This guide covers the most common configurations.

## Provider Overview

| Provider | Backend value | Requires API key | Notes |
|---|---|---|---|
| OpenRouter | `openai_compatible` | `LLM_API_KEY` | **Default**; 300+ models |
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Claude models |
| OpenAI | `openai` | `OPENAI_API_KEY` | GPT models |
| Ollama | `ollama` | No | Local inference |
| Tinfoil | `tinfoil` | `TINFOIL_API_KEY` | Private/confidential inference |
| Together AI | `openai_compatible` | `LLM_API_KEY` | Fast inference |
| Fireworks AI | `openai_compatible` | `LLM_API_KEY` | Fast inference |
| vLLM / LiteLLM | `openai_compatible` | Optional | Self-hosted |
| LM Studio | `openai_compatible` | No | Local GUI |

---

---

## Anthropic (Claude)

```env
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-...
```

Popular models: `claude-sonnet-4-20250514`, `claude-3-5-sonnet-20241022`, `claude-3-5-haiku-20241022`

---

## OpenAI (GPT)

```env
LLM_BACKEND=openai
OPENAI_API_KEY=sk-...
```

Popular models: `gpt-4o`, `gpt-4o-mini`, `o3-mini`

---

## Ollama (local)

Install Ollama from [ollama.com](https://ollama.com), pull a model, then:

```env
LLM_BACKEND=ollama
OLLAMA_MODEL=llama3.2
# OLLAMA_BASE_URL=http://localhost:11434   # default
```

Pull a model first: `ollama pull llama3.2`

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

### Fireworks AI

[Fireworks AI](https://fireworks.ai) offers fast inference with compound AI system support.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.fireworks.ai/inference/v1
LLM_API_KEY=fw_...
LLM_MODEL=accounts/fireworks/models/llama4-maverick-instruct-basic
```

### vLLM / LiteLLM (self-hosted)

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

### LM Studio (local GUI)

Start LM Studio's local server, then:

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://localhost:1234/v1
LLM_MODEL=llama-3.2-3b-instruct-q4_K_M
# LLM_API_KEY is not required for LM Studio
```

---

## Using the Setup Wizard

Instead of editing `.env` manually, run the onboarding wizard:

```bash
thinclaw onboard
```

Select **"OpenAI-compatible"** for OpenRouter, Together AI, Fireworks, vLLM, LiteLLM,
or LM Studio. You will be prompted for the base URL and (optionally) an API key.
The model name is configured in the following step.
