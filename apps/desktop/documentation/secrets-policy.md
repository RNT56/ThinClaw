# ThinClaw Desktop Secrets Policy

Last updated: 2026-05-15

This policy documents how Desktop stores, grants, injects, and migrates secrets.

## Naming Rules

- Public branding is ThinClaw.
- New secret writes use ThinClaw provider identifiers such as `anthropic`, `openai`, `gemini`, `groq`, `openrouter`, `brave`, `huggingface`, `bedrock`, and custom OpenAI-compatible provider slugs.
- Legacy Scrappy/ThinClaw key names remain fallback-only read inputs for migration and rollback.
- Do not add new writes to legacy Scrappy identifiers.

## Storage

Local Desktop stores provider keys in the OS keychain through the Desktop keychain adapter.

- macOS: Keychain.
- Other platforms: use the configured Tauri/OS secrets backend when available.
- Runtime config files may store provider status, enabled providers, selected models, and grant flags, but must not store raw API keys.

## Grants

Saving a key is not enough to expose it to ThinClaw tools. The user must also grant access.

The secrets adapter must enforce grants for:

- `get`
- `get_for_injection`
- `exists`
- `list`
- `is_accessible`

Denied methods should fail closed. UI status may show that a key exists only when the current grant policy allows the status check.

## Injection

Local mode may inject granted secrets into the in-process ThinClaw runtime or a local engine process.

Remote mode must not inject raw secrets from Desktop into a remote gateway. Remote provider credentials must move through provider-vault save/delete/status endpoints so the remote gateway stores them in its own secrets backend.

## Remote Mode

Allowed:

- Save/update a provider key through a remote provider-vault route.
- Delete a provider key through a remote provider-vault route.
- Read sanitized provider status.
- Read sanitized model/provider configuration.

Forbidden:

- Reading raw remote secrets.
- Returning raw remote secrets to Desktop.
- Raw secret injection from Desktop to a remote process.
- Treating a successful status response as proof that Desktop can access the underlying secret value.

## Provider-Specific Notes

| Provider class | Policy |
| --- | --- |
| Anthropic/OpenAI/Gemini/Groq/OpenRouter/Brave/Hugging Face | Use ThinClaw identifiers for new writes; legacy env/keychain names are fallback reads. |
| Bedrock | Support bearer-token, proxy-key, and AWS access-key paths. New persisted writes should use ThinClaw Bedrock identifiers; env variables remain fallback. |
| Custom OpenAI-compatible | Store by custom provider slug. Do not collapse all custom providers into one global key if the UI allows multiple providers. |
| Cloud storage | Store cloud credentials in the cloud/provider secrets path. Legacy app-data import is read-only migration. |

## Testing Requirements

P3 contract tests should cover:

- New writes use ThinClaw identifiers.
- Legacy Scrappy reads remain fallback-only.
- Ungranted `get`, `get_for_injection`, `exists`, `list`, and `is_accessible` are denied.
- Remote save/delete/status never returns a raw secret.
- Deleting a key revokes grants.

## Operational Checklist

Before release or review:

- Save a key in Settings > Secrets.
- Confirm status shows saved but ungranted.
- Grant access and confirm provider/model discovery works.
- Revoke grant and confirm agent injection stops.
- Delete key and confirm status/route simulation no longer treats it as available.
- Switch to remote gateway mode and confirm raw-secret read commands return unavailable/denied behavior.
