# Local engine compatibility and provenance

Last verified: **2026-07-13**

ThinClaw treats native engines and their bootstrap tools as a tested matrix. A
version change must update this page, the matching setup script/runtime pin, and
the focused tests in the same change.

## Validated matrix

| Engine | Validated version | Supported host | Model contract | Provisioning |
|---|---:|---|---|---|
| llama.cpp | `b9988` | macOS arm64/x64, Linux x64, Windows x64 | GGUF v2/v3; quant types listed below | Bundled `llama-server`; release asset SHA-256 and `--version` verified before atomic install |
| MLX service stack | `mlx-openai-server==1.8.1`, `mlx-embeddings==0.1.0`, `mflux==0.18.0`, `mlx-whisper==0.4.3` | macOS Apple Silicon | MLX safetensors directory with `config.json` | Python 3.12 venv through bundled/discovered `uv`; exact direct pins and versioned bootstrap marker |
| vLLM | `vllm==0.25.0` | Linux x64 with NVIDIA CUDA | Hugging Face/AWQ safetensors directory | Reproducible Python 3.12 venv through `uv`; stale marker recreates the managed venv |
| Ollama | External; API validated with `0.31.2` | Ollama-supported hosts | Ollama model name | User-managed daemon; ThinClaw probes `/api/tags` and reports `/api/version` |
| uv | `0.11.28` | macOS arm64/x64, Linux x64 | MLX/vLLM bootstrap tool | Bundled or runtime-downloaded asset with per-platform SHA-256, size limit, timeout, version smoke, and atomic install |

The direct Python dependencies are exact pins. Their transitive resolution is
owned by `uv`; changing any direct pin changes the bootstrap fingerprint so an
existing managed environment cannot silently remain stale.

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

## Verification boundary

Automated coverage verifies setup-script syntax, exact pins/fingerprints, GGUF
bounds and quant mapping, archive checksum handling, and every compile-time
engine feature. A macOS arm64 download/extract/version smoke is run when the pin
changes. Linux CUDA throughput, Ollama model execution, and cross-platform
packaged-app execution remain release-candidate hardware checks; this document
does not claim those external-device smokes were run locally.

## Primary sources

- [llama.cpp `b9988` release](https://github.com/ggml-org/llama.cpp/releases/tag/b9988)
- [uv `0.11.28` release](https://github.com/astral-sh/uv/releases/tag/0.11.28)
- [MLX `0.32.0` release](https://github.com/ml-explore/mlx/releases/tag/v0.32.0)
- [mlx-openai-server `1.8.1`](https://pypi.org/project/mlx-openai-server/1.8.1/)
- [vLLM `0.25.0` release](https://github.com/vllm-project/vllm/releases/tag/v0.25.0)
- [Ollama `0.31.2` release](https://github.com/ollama/ollama/releases/tag/v0.31.2)
