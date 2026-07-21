# External Release Prerequisites

Last updated: 2026-07-14

This file tracks release-operator inputs that cannot be proven by committed
fixtures or local CI. They are not Desktop implementation TODOs, and every
runtime path that depends on them must remain visibly gated with a concrete
reason.

## Signing And Publishing

- GitHub Actions secrets `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, and
  `TAURI_SIGNING_PRIVATE_KEY`. `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is optional
  when the updater key is unencrypted. The private updater key must match the
  public key committed in `backend/tauri.conf.json`; rotate both together.
- The committed tag-release job treats the GitHub Releases `latest.json` endpoint
  as the production stable channel. It fails before building when a required
  secret is missing and refuses to upload unless the DMG is signed, notarized,
  stapled, Gatekeeper-accepted, and accompanied by a non-empty updater signature.

## Real External Services

- Optional real ThinClaw gateway smoke target.
- Optional real provider secrets for OpenAI, Anthropic, Gemini, Groq,
  OpenRouter, Bedrock, Hugging Face, or custom OpenAI-compatible providers.
- Optional real Gmail OAuth client credentials and channel accounts.
- Optional real Slack, Telegram, Apple Mail, and pairing test accounts.

## Host Capabilities

- First real signed clean-machine launch validation for Keychain access groups,
  entitlements, bundled sidecar execution, and Gatekeeper behavior. Download,
  checksum, layout, override-generation, and size-budget setup behavior is
  already deterministic CI fixture coverage.
- Platform-specific validation for Windows/Linux features that are hidden or
  gated on macOS-only release candidates.
- GPU cloud validation and test launch only when a fixture provider or real
  configured secrets expose that capability.

## Fixture-Proven Completion Rule

Desktop behavior is considered implementation-complete when deterministic
fixtures pass local and remote acceptance. Real service smoke is useful release
evidence, but absence of real credentials must not block fixture acceptance.
