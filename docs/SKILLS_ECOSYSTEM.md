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
- `skill_search`
- `skill_install`
- `skill_update`
- `skill_audit`
- `skill_snapshot`
- `skill_remove`
- `skill_reload`
- `skill_trust_promote`
- `skill_manage`

## Quarantine And Provenance Locks

Externally sourced installs pass through the quarantine manager before landing in the install directory.

- risky findings are surfaced before install for community-trust sources
- approved installs persist a `.thinclaw-skill-lock.json` provenance record next to `SKILL.md`
- `skill_update` uses that provenance lock when it is available
- `skill_audit` re-runs the content scanner without removing or mutating the skill
- `skill_snapshot` writes a point-in-time JSON manifest to `~/.thinclaw/skills/.hub/`

## Generated Skills

Generated workflow skills are not a separate subsystem anymore.

- post-turn synthesis candidates enter the existing Learning Ledger lifecycle
- normal mode keeps them review-oriented
- `reckless_desktop` can auto-activate them through the same lifecycle machinery

For the learning-side behavior and settings knobs, see [OUTCOME_BACKED_LEARNING.md](OUTCOME_BACKED_LEARNING.md).
