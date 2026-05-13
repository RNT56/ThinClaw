---
name: creative-comfyui
version: 0.1.0
description: Generate and transform media with the built-in ComfyUI tools.
activation:
  keywords:
    - image generation
    - generate image
    - ComfyUI
    - txt2img
    - img2img
    - inpaint
    - upscale
    - video generation
  patterns:
    - "(?i)\\b(generate|create|make)\\b.*\\b(image|picture|art|illustration|render)\\b"
    - "(?i)\\b(upscale|inpaint|img2img|txt2img|comfyui)\\b"
metadata:
  openclaw:
    provenance: builtin
---

# ComfyUI Creative Media

Use this skill when the user asks ThinClaw to create, transform, upscale, inpaint, or diagnose generated media using ComfyUI.

Prefer `image_generate` for ordinary prompt-to-image requests. Keep the tool call simple: provide the user's prompt, choose an aspect ratio if they gave one, and add a negative prompt only when it materially improves the request.

Use `comfy_health` before debugging setup or connectivity. If the server is unreachable, explain that ComfyUI is not reachable and use `comfy_manage` only when the user explicitly asks to install, launch, stop, or modify local ComfyUI.

Use `comfy_check_deps` before running a custom or advanced workflow when missing models or nodes are likely. Use `comfy_run_workflow` for bundled img2img/upscale workflows, or for inpaint/video/Flux workflows supplied as approved API-format JSON.

Never ask `comfy_manage` to install nodes, download models, or launch/stop ComfyUI unless the user has explicitly requested that host-level action. These actions may install Python packages, download large model files, mutate local state, or spend cloud credits.

ComfyUI workflows must be API-format JSON: a top-level object of node IDs whose values have `class_type` and `inputs`. Editor-format workflows with `nodes` and `links` are not executable.
