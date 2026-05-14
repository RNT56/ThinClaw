# ComfyUI Media Generation

ThinClaw exposes ComfyUI through native Rust tools. The `comfy` CLI is used only
for explicit lifecycle actions such as install, launch, stop, model download,
and custom node installation. Workflow execution uses ComfyUI REST and
WebSocket APIs directly.

## Configuration

Enable the tools with either TOML/settings or environment variables:

```toml
[comfyui]
enabled = true
mode = "local_existing" # local_existing, local_managed, cloud
host = "http://127.0.0.1:8188"
default_workflow = "sdxl_txt2img"
default_aspect_ratio = "square"
allow_lifecycle_management = false
allow_untrusted_workflows = false
request_timeout_secs = 600
```

Cloud mode uses the ThinClaw secret named `comfy_cloud_api_key` by default, with
`COMFY_CLOUD_API_KEY` as an environment fallback.

## Agent Tools

- `image_generate`: prompt-to-image using the configured default workflow.
- `comfy_health`: read-only server and object-info health check.
- `comfy_check_deps`: read-only workflow dependency report.
- `comfy_run_workflow`: advanced bundled or approved workflow execution.
- `comfy_manage`: explicit setup/lifecycle/model/node actions, registered only
  when `allow_lifecycle_management = true` and always approval-gated.

Generated outputs are saved under `~/.thinclaw/media_cache/generated` by
default and returned as JSON plus renderable tool artifacts for web clients.

## CLI

```bash
thinclaw comfy health
thinclaw comfy hardware-check
thinclaw comfy setup --gpu cpu
thinclaw comfy launch
thinclaw comfy stop
thinclaw comfy list-workflows
thinclaw comfy check-deps sdxl_txt2img
thinclaw comfy generate "a cinematic product photo of a matte black espresso machine" --aspect-ratio wide
```

`generate` uses the configured default workflow unless `--workflow` is provided.
When `allow_untrusted_workflows = true`, `--workflow` and `check-deps` may also
point to an API-format workflow JSON file.

## Workflow Format

Workflows must be ComfyUI API-format JSON: a top-level object keyed by node IDs
where each node has `class_type` and `inputs`. Editor-format workflows with
`nodes` and `links` are rejected.

Bundled workflow names:

- `sdxl_txt2img`
- `sd15_txt2img`
- `sdxl_img2img`
- `upscale_4x`

Advanced inpaint, Flux, AnimateDiff, and Wan workflows can be run with
`comfy_run_workflow` from approved API-format JSON files.

## Security Model

ComfyUI is treated as a trusted sidecar, not a sandboxed WASM extension.
Custom nodes and workflows may execute Python and download large files. ThinClaw
therefore keeps lifecycle actions explicit and approval-gated, disables
untrusted workflow paths by default, stores cloud API keys in the secrets store,
sanitizes output filenames, rejects traversal paths, enforces output-size
limits, and avoids forwarding cloud API headers to signed output redirects.
