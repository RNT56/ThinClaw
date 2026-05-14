# ComfyUI Built-In Tools

ComfyUI media generation is a built-in ThinClaw tool family. It is not a WASM
tool package and is not installed with `thinclaw tool install`.

## Tools

| Tool | Purpose | Notes |
|---|---|---|
| `image_generate` | Prompt-to-image generation with the configured default workflow | Preferred tool for ordinary image requests |
| `comfy_health` | Server and object-info health check | Read-only |
| `comfy_check_deps` | Workflow model/custom-node dependency report | Read-only |
| `comfy_run_workflow` | Run bundled or approved API-format workflows | Use for img2img, upscale, inpaint, video, or custom flows |
| `comfy_manage` | Install, launch, stop, download models, or install nodes | Registered only when lifecycle management is enabled and approval-gated |

## Configuration

Enable the tools in settings or the environment:

```toml
[comfyui]
enabled = true
mode = "local_existing" # local_existing, local_managed, cloud
host = "http://127.0.0.1:8188"
default_workflow = "sdxl_txt2img"
default_aspect_ratio = "square"
allow_lifecycle_management = false
allow_untrusted_workflows = false
```

Cloud mode reads ThinClaw secret `comfy_cloud_api_key` by default. The
environment fallback is `COMFY_CLOUD_API_KEY`.

## CLI

Use `thinclaw comfy ...` for operator diagnostics and lifecycle:

```bash
thinclaw comfy health
thinclaw comfy hardware-check
thinclaw comfy setup --gpu cpu
thinclaw comfy launch
thinclaw comfy list-workflows
thinclaw comfy check-deps sdxl_txt2img
thinclaw comfy generate "a product photo of a matte black espresso machine" --aspect-ratio wide
```

## Skill

The bundled `creative-comfyui` skill activates for image generation, img2img,
inpaint, upscale, and ComfyUI troubleshooting requests. It prefers
`image_generate` for simple prompt-to-image work, uses `comfy_health` for setup
diagnosis, and reserves `comfy_manage` for explicit host-level lifecycle
requests.

## Trust Boundary

ComfyUI is an operator-trusted local or cloud sidecar, not a ThinClaw WASM
sandbox. Custom nodes and arbitrary workflows can execute Python, download large
model files, mutate local state, or spend cloud credits. Keep
`allow_untrusted_workflows = false` and `allow_lifecycle_management = false`
unless the operator deliberately grants those capabilities.

Canonical deep guide: [../docs/COMFYUI_MEDIA_GENERATION.md](../docs/COMFYUI_MEDIA_GENERATION.md).
