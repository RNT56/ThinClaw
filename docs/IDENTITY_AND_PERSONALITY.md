# Identity And Personality

This document is the canonical guide to ThinClaw's agent identity model.

## The Stack

ThinClaw now separates durable identity from temporary session tone.

1. `THINCLAW_HOME/SOUL.md` is the canonical durable soul across projects.
2. Workspaces inherit that global soul by default.
3. `SOUL.local.md` is an explicit-only workspace overlay for rare cases where a workspace genuinely needs narrower tone adjustments or stricter boundaries.
4. `IDENTITY.md`, `USER.md`, and `AGENTS.md` remain workspace-scoped.
5. `agent.personality_pack` defines the initial seed pack used when the canonical soul is first created.
6. `/personality` applies a temporary session-only overlay without rewriting the durable identity files.

`/vibe` remains available as a compatibility alias, but `/personality` is the primary user-facing command.

## Zero-Confusion Default

Normal users should not have to think about a separate "workspace persona."

- Default behavior is `global soul only`.
- New workspaces do not create `SOUL.local.md`.
- Surfaces should say "Using global soul" unless a local overlay actually exists.

## Personality Packs

Built-in packs:

- `balanced`
- `professional`
- `creative_partner`
- `research_assistant`
- `mentor`
- `minimal`

The pack chosen during onboarding seeds the first canonical `THINCLAW_HOME/SOUL.md` and is stored in `agent.personality_pack`. Later pack changes do not silently rewrite an already-authored soul.

## Temporary Session Personalities

Use `/personality` to inspect, set, or reset a session-only overlay:

```text
/personality
/personality technical
/personality reset
```

Available built-in overlays include the core packs plus lightweight tone presets such as `concise`, `creative`, `technical`, `playful`, `formal`, and `eli5`.

## Compatibility Notes

- `agent.persona_seed` is still read for backward compatibility.
- `AGENT_PERSONA_SEED` still works, but `AGENT_PERSONALITY_PACK` is the preferred environment override.
- Legacy `assets/persona_seeds/*.md` files are migration stubs; new code should use `assets/personality_packs/*.md`.
- Legacy workspace `SOUL.md` files are migrated to the canonical home soul or archived as `SOUL.legacy.md` plus `SOUL.local.md` when the workspace is agent-specific.
