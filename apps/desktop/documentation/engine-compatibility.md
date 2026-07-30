# Local engine compatibility and provenance

Last verified: **2026-07-30**

ThinClaw treats native engines and their bootstrap tools as a tested matrix. A
version change must update this page, the matching setup script/runtime pin, and
the focused tests in the same change.

## Validated matrix

| Engine | Validated version | Supported host | Model contract | Provisioning |
|---|---:|---|---|---|
| llama.cpp | `b9988` | macOS arm64/x64, Linux x64, Windows x64 | GGUF v2/v3; quant types listed below | Bundled `llama-server`; staged archive verification, runtime inventory manifest, target gate, and cross-platform loader path |
| MLX service stack | `mlx-openai-server==1.8.1`, `mlx==0.31.2`, `mlx-lm==0.31.3`, `mlx-vlm==0.4.4`, `mlx-embeddings==0.0.5`, `mflux==0.17.5`, `mlx-whisper==0.4.3` | macOS 14+ on Apple Silicon | Task-specific MLX, Whisper, or MFlux directory layout | Managed Python `3.12.13`; hash-locked wheel graph; import self-test; lock-hash marker; bundled `uv` only |
| vLLM | `vllm==0.25.0`, `torch==2.11.0+cu129` | Linux x64, glibc 2.31+, NVIDIA compute capability 7.5+ | Hugging Face/AWQ safetensors directory | Managed Python `3.12.13`; hash-locked CUDA 12.9 wheel graph; host preflight and CUDA allocation self-test |
| Ollama | External; API validated with `0.31.2` | Ollama-supported hosts | Ollama model name | User-managed daemon; ThinClaw probes `/api/tags` and reports `/api/version` |
| uv | `0.11.28` | macOS arm64/x64, Linux x64 | MLX/vLLM bootstrap tool | Bundled or runtime-downloaded asset with per-platform SHA-256, size limit, timeout, version smoke, and atomic install |

`engine-manifest.json` is the authority for versions, supported targets, host
defaults, and minimum platform versions. Both Python graphs are committed locks
with artifact hashes and resolution cutoffs. A bootstrap marker is written only
after the complete environment passes validation; its lock digest makes any
dependency change trigger repair.

## GGUF and quantization contract

The bundled llama.cpp build accepts GGUF v2/v3 with a non-empty tensor set,
`general.architecture`, and `general.file_type`. ThinClaw validates metadata
counts and lengths before launch and fails before spawning the sidecar when the
file is truncated or its quantization is outside the pinned engine matrix.

Validated `general.file_type` families:

- Full precision: `F32`, `F16`, `BF16`
- Legacy quants: `Q4_0`, `Q4_1`, `Q5_0`, `Q5_1`, `Q8_0`
- K-quants: `Q2_K`, `Q2_K_S`, `Q3_K_S/M/L`, `Q4_K_S/M`, `Q5_K_S/M`, `Q6_K`
- I-quants: `IQ1_S/M`, `IQ2_XXS/XS/S/M`, `IQ3_XXS/XS/S/M`, `IQ4_NL/XS`
- Current low-bit formats: `TQ1_0`, `TQ2_0`, `MXFP4_MOE`, `NVFP4`, `Q1_0`, `Q2_0`

Hugging Face discovery also recognizes mixed-tensor `UD-*` filenames. Removed
llama.cpp file-type values (`4-6`, `33-35`) and unknown future values fail
closed until the bundled sidecar is deliberately upgraded and revalidated.

## Hugging Face discovery and installation contract

The backend, not the renderer, owns the searchable task matrix and Hub tags:

| Runtime | Searchable tasks | Accepted artifact layout |
|---|---|---|
| llama.cpp | Chat, vision, embedding | One complete GGUF quantization; split GGUF variants include every numbered shard. Vision may add one explicit mmproj artifact. |
| MLX | Chat, vision, embedding, Whisper STT, FLUX.1 dev/schnell diffusion | Runtime-specific directory layouts. Transformer models use root `config.json`, tokenizer assets, and MLX weights; Whisper uses root `config.json` plus exact `weights.npz` or `weights.safetensors`; MFlux requires its complete transformer, VAE, dual-text-encoder, and dual-tokenizer component layout. |
| vLLM | Chat, vision | AWQ directory with root `config.json` and supported weights. |
| Ollama / cloud-only | None | Raw Hub files are not presented as Ollama imports or cloud models. |

MLX embedding compatibility is pinned to the text-vector behavior of
`mlx-embeddings==0.0.5` as served by `mlx-openai-server==1.8.1`; an importable
Python model module alone is not sufficient. ThinClaw normalizes
`config.json` `model_type` by lowercasing and replacing `-` with `_`, then
accepts only `bert`, `xlm_roberta`, `qwen3`, `gemma3_text`, and `modernbert`.
`modernbert` additionally requires `architectures: ["ModernBertModel"]`;
masked-language-model configurations produce token-level tensors rather than
the one vector per input required by the OpenAI embeddings API. Other importable
0.0.5 modules (`lfm2`, `colqwen2_5`, and `siglip`) are intentionally rejected
because they do not satisfy that text-only two-dimensional response contract.
Embedding search expands Hub config only into a backend-private candidate,
filters unsupported cards before returning the public card DTO, and repeats the
same check against the immutable repository commit when planning and rebuilding
a download. This allowlist must be re-audited whenever either Python package
pin changes.

MLX vision discovery is fail-closed at two levels. Search results and the
immutable-revision preflight require a multimodal `config.json` marker before a
repository is offered or planned. A completed download is accepted only when
its safetensors header or `model.safetensors.index.json` also contains
`vision_tower.*`, `vision_model.*`, or `multi_modal_projector.*` tensor keys.
Managed inventory and the MLX engine repeat that same config-and-tensor check;
a text-only config, or a vision marker backed only by language-model weights,
cannot be installed or silently launched as a text model.

Video and generic TTS Hub search are intentionally absent until ThinClaw has a
validated local consumer and artifact contract for them. Search results are
post-filtered against the active profile. Family-narrowed searches follow a
bounded set of same-route Hub pagination links until they fill the requested
compatible result window or exhaust the route; the response reports when more
compatible results may remain. A partial multi-filter Hub failure fails
explicitly instead of returning a misleading subset.

File plans resolve an immutable 40-character repository commit and recursively
page the repository tree. Alternative quantizations remain separate choices;
split shards are one logical artifact; mmproj files are explicit companions.
Downloads rebuild the pinned plan, stream into a private staging directory,
enforce file and aggregate limits, verify Hub LFS SHA-256 values, and publish
only after every file and the managed manifest are durable.

Each published install contains `.thinclaw-model.json`, recording its runtime,
task, category, repository, revision, base artifact, optional projector and
exact loadable paths. The inventory returns one logical entry per install and
marks runtime compatibility explicitly. An invalid manifest is shown as one
incomplete, non-loadable install so its shards or nested weights cannot be
mistaken for independent models.

## Verification boundary

Automated coverage verifies setup-script syntax, manifest/lock alignment, GGUF
bounds and quant mapping, archive checksum handling, target gates, and every
compile-time engine feature. The macOS release pipeline builds the MLX variant
and executes the installed `uv` sidecar from the `.app`. Linux CUDA model
throughput, Ollama model execution, and non-macOS installed-app execution remain
release-candidate hardware checks.

## Primary sources

- [llama.cpp `b9988` release](https://github.com/ggml-org/llama.cpp/releases/tag/b9988)
- [uv `0.11.28` release](https://github.com/astral-sh/uv/releases/tag/0.11.28)
- [MLX `0.31.2` release](https://github.com/ml-explore/mlx/releases/tag/v0.31.2)
- [mlx-openai-server `1.8.1`](https://pypi.org/project/mlx-openai-server/1.8.1/)
- [mlx-embeddings `0.0.5`](https://pypi.org/project/mlx-embeddings/0.0.5/)
- [mlx-whisper model format and conversion](https://github.com/ml-explore/mlx-examples/blob/main/whisper/README.md)
- [MFlux `0.17.5`](https://pypi.org/project/mflux/0.17.5/)
- [vLLM `0.25.0` release](https://github.com/vllm-project/vllm/releases/tag/v0.25.0)
- [Ollama `0.31.2` release](https://github.com/ollama/ollama/releases/tag/v0.31.2)
