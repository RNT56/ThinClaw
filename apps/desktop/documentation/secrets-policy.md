# ThinClaw Desktop Secrets Policy

Last updated: 2026-07-13

This policy documents how Desktop stores, grants, injects, and migrates secrets.

## Naming Rules

- Public branding is ThinClaw.
- New secret writes use canonical ThinClaw identifiers such as `llm_anthropic_api_key`, `llm_openai_api_key`, `llm_compatible_api_key`, `search_brave_api_key`, and `hf_token`; providers without a renamed contract keep their provider slug, and custom secrets keep their generated ID.
- Legacy Scrappy/ThinClaw key names are canonicalized when the unified keychain blob is loaded; fallback reads remain for rollback compatibility.
- Do not add new writes to legacy Scrappy identifiers.

## Storage

Local Desktop stores provider keys in the OS keychain through one application-wide
`SecretStore`. Direct Workbench consumers use its host methods; the ThinClaw runtime
uses the `SecretsStore` trait implementation on the same service, with agent grants
checked before every runtime read. Clones share grant state and never create a second
keychain cache or secret store.

- macOS: one authenticated AES-256-GCM envelope is stored in Keychain under
  `api_keys`; its random 32-byte master key uses the shared core
  `thinclaw/master_key` secure-store item, so CLI/runtime rotation remains
  durable across restarts. The core `SecretsCrypto` contract uses HKDF-SHA-256 and authenticated
  envelope metadata. Raw values exist only in the process cache while Desktop
  is running, never in the persisted JSON envelope.
- Existing unified plaintext blobs are encrypted in place after authentication;
  old per-provider Keychain items are deleted only after that encrypted write
  succeeds. A failed migration retains `identity.json` and legacy items for a
  later retry.
- Corrupt, tampered, or unavailable Keychain data fails closed at startup. It is
  never interpreted as an empty vault.
- Other platforms: the current compatibility implementation is process-local
  and not durable. A real OS secrets backend is required before Windows/Linux
  Desktop packages can claim persistent credential support.
- Runtime config files may store provider status, enabled providers, selected models, and grant flags, but must not store raw API keys.

## Grants

Saving a key is not enough to expose it to ThinClaw tools. The user must also grant access.

The grant-aware runtime view must enforce grants for:

- `get`
- `get_for_injection`
- `exists`
- `list`
- `is_accessible`

Denied methods should fail closed. UI status may show that a key exists only when the current grant policy allows the status check.
Saving, deleting, or toggling a local credential refreshes the shared grant state
immediately. Deleting a provider or custom secret also revokes its persisted grant.

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

- Persisted envelopes contain no plaintext values and reject ciphertext or AAD tampering.
- Legacy plaintext is detected for one-time encryption, and old data is not deleted when the encrypted write fails.
- Key rotation increments the envelope key version and makes the old master key unusable.
- New writes use ThinClaw identifiers.
- Legacy Scrappy aliases migrate to canonical names without overwriting a newer canonical value.
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
