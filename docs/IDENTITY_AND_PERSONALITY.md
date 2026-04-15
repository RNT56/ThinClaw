# Identity And Personality

This document is the canonical guide to ThinClaw's agent identity model.

## The Stack

ThinClaw now separates durable identity from temporary session tone.

1. `IDENTITY.md`, `SOUL.md`, `USER.md`, and `AGENTS.md` define the durable workspace identity.
2. `agent.personality_pack` defines the default starting pack for new workspaces and cross-surface copy.
3. Surface/runtime overlays add channel formatting and execution-context guidance.
4. `/personality` applies a temporary session-only overlay without rewriting the durable identity files.

`/vibe` remains available as a compatibility alias, but `/personality` is the primary user-facing command.

## Personality Packs

Built-in packs:

- `balanced`
- `professional`
- `creative_partner`
- `research_assistant`
- `mentor`
- `minimal`

The pack chosen during onboarding seeds `SOUL.md` for fresh workspaces and is stored in `agent.personality_pack`.

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
