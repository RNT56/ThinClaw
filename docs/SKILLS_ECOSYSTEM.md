# Skills Ecosystem

This document covers the current ThinClaw skill lifecycle beyond the basic `SKILL.md` format.

## Trust And Provenance

ThinClaw keeps authority and provenance separate on purpose:

- `trust` is the hard tool-attentuation ceiling: `installed` or `trusted`
- `source_tier` is ecosystem/display provenance: `builtin`, `official`, `trusted`, `community`, `unvetted`

`source_tier` is informational. It does not grant tool access.

## Current Discovery Layers

Skills are discovered in deterministic precedence order:

1. workspace skills
2. user skills
3. installed skills
4. external read-only skill directories

Earlier layers win on name collisions.

## Bundled Runtime Skills

ThinClaw can ship trusted runtime skills with the binary. These are source-tier
`builtin` skills and still follow the normal skill-selection and tool-profile
rules.

- `creative-comfyui`: activates for image generation, img2img, inpaint,
  upscale, video-generation, and ComfyUI troubleshooting requests. It prefers
  the built-in `image_generate` tool for simple prompt-to-image work, uses
  `comfy_health` and `comfy_check_deps` for diagnostics, and only uses
  `comfy_manage` for explicit lifecycle requests because those actions can
  install Python packages, download models, mutate local state, or spend cloud
  credits. Generated media from `image_generate` and `comfy_run_workflow` is
  auto-attached to the final response on channels that support media delivery;
  text-only channels receive an explicit path/link fallback.

## Remote Source Adapters

Remote marketplace search is aggregated by `RemoteSkillHub`.

- ClawHub remains the built-in catalog path.
- GitHub taps come from `skill_taps`.
- `/.well-known/skills` registries come from `well_known_skill_registries`.
- LobeHub is available through `LobeHubSkillSource`; set `LOBEHUB_SKILLS_ENABLED=false` to disable it or `LOBEHUB_SKILLS_INDEX_URL` to override the default index.
- `skills.sh` is available through `SkillsShSource` when `SKILLS_SH_INDEX_URL` points to a JSON index.

ThinClaw never executes marketplace installer scripts. Adapters must return direct `SKILL.md` content or a direct `SKILL.md` URL, and installs continue through quarantine and provenance locks.

## Current Agent-Facing Skill Lifecycle

The runtime now exposes these skill-management tools:

- `skill_list`
- `skill_read`
- `skill_inspect`
- `skill_search`
- `skill_check`
- `skill_install`
- `skill_update`
- `skill_audit`
- `skill_snapshot`
- `skill_publish`
- `skill_tap_list`
- `skill_tap_add`
- `skill_tap_remove`
- `skill_tap_refresh`
- `skill_remove`
- `skill_reload`
- `skill_trust_promote`
- `skill_manage`

## Quarantine And Provenance Locks

Externally sourced installs pass through the quarantine manager before landing in the install directory.

- risky findings are surfaced before install for community-trust sources
- `skill_check` validates inline content, local paths, or direct HTTPS `SKILL.md` URLs without installing
- approved installs persist a `.thinclaw-skill-lock.json` provenance record next to `SKILL.md`
- `skill_update` uses that provenance lock when it is available
- `skill_audit` re-runs the content scanner without removing or mutating the skill
- `skill_snapshot` writes a point-in-time JSON manifest to `~/.thinclaw/skills/.hub/`

The quarantine scanner is versioned as `skill_quarantine_v2`. It scans `SKILL.md` plus package support files and reports rule IDs, file paths, line numbers, severity, recommendations, scanner version, content hash, and finding summaries. Current rule groups cover network fetches, pipe-to-shell patterns, command/code execution, secret/environment reads, destructive filesystem operations, traversal attempts, encoded payloads, persistence hooks, dependency install scripts, and prompt-override attempts.

Package layout validation also blocks absolute paths, path traversal, hidden/VCS/cache paths, symlinks, and provenance-lock spoofing inside incoming skill packages. Community-trust installs with high-risk findings still require explicit approval before files are written.

This is a concrete install-time scanner, not a complete formal audit system. It is intentionally smaller than a full marketplace-scale ruleset and should continue to grow as new skill abuse patterns are identified.

## Inspect, Taps, And Publishing

`skill_inspect` returns a loaded skill report with metadata, source/trust fields, provenance lock details, publishable file inventory, and optional quarantine findings. It is read-only and honors restricted agent `allowed_skills` contexts.

GitHub skill taps are persisted in the settings DB under `skill_taps` and use the existing `SkillTapConfig` shape: `repo`, `path`, optional `branch`, and `trust_level`. `skill_tap_add`, `skill_tap_remove`, and `skill_tap_refresh` rebuild the shared remote hub so search and install see current tap state without restarting. Config-file overlays can still seed or override startup settings.

`skill_publish` is dry-run by default. A dry run validates the local skill package, runs the quarantine scanner, confirms the target repo is a configured tap, and returns the target repo/path, publishable file list, content hash, trust/source details, and draft PR plan. Remote writes require all of the following:

- `remote_write=true`
- `confirm_remote_write=true`
- tool approval from the caller
- a configured GitHub tap matching `target_repo`
- local `git` and authenticated `gh`

The publish package is written to `<tap.path>/<skill-name>/`. It includes `SKILL.md` and regular non-hidden support files, and excludes provenance locks, hidden files, VCS/cache/temp directories, symlinks, and traversal paths. Remote publishing clones the tap repo, creates `codex/skill-publish/<skill-name>-<hash8>`, commits the package, pushes the branch, and opens a draft PR with `gh pr create`.

The Skills HTTP API exposes matching inspect, publish, tap list/add/remove/refresh routes. Mutating routes require `X-Confirm-Action: true`; publish only mutates when `remote_write=true`.

## Generated Skills

Generated workflow skills are not a separate subsystem anymore.

- post-turn synthesis candidates enter the existing Learning Ledger lifecycle
- normal mode keeps them review-oriented
- `reckless_desktop` can auto-activate them through the same lifecycle machinery

For the learning-side behavior and settings knobs, see [OUTCOME_BACKED_LEARNING.md](OUTCOME_BACKED_LEARNING.md).
