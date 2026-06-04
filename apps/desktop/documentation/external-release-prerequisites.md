# External Release Prerequisites

Last updated: 2026-05-15

This file tracks release-operator inputs that cannot be proven by committed
fixtures or local CI. They are not Desktop implementation TODOs, and every
runtime path that depends on them must remain visibly gated with a concrete
reason.

## Signing And Publishing

- Apple Developer Team ID, signing certificate, notarization credentials, and
  app-specific password or API key.
- Private updater signing keys and production updater metadata destination.
- Final release channel decision for unsigned fixture builds versus signed
  production builds.

## Real External Services

- Optional real ThinClaw gateway smoke target.
- Optional real provider secrets for OpenAI, Anthropic, Gemini, Groq,
  OpenRouter, Bedrock, Hugging Face, or custom OpenAI-compatible providers.
- Optional real Gmail OAuth client credentials and channel accounts.
- Optional real Slack, Telegram, Apple Mail, and pairing test accounts.

## Host Capabilities

- macOS clean-machine validation for keychain access groups, entitlements,
  sidecar execution, and notarized launch.
- Platform-specific validation for Windows/Linux features that are hidden or
  gated on macOS-only release candidates.
- GPU cloud validation and test launch only when a fixture provider or real
  configured secrets expose that capability.

## Fixture-Proven Completion Rule

Desktop behavior is considered implementation-complete when deterministic
fixtures pass local and remote acceptance. Real service smoke is useful release
evidence, but absence of real credentials must not block fixture acceptance.
