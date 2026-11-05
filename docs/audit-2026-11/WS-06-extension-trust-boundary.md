# WS-06 — Extension Trust Boundary

| | |
|---|---|
| **Priority** | P0 (6.1–6.4, Critical C4) · P1 (6.5–6.10) · P2 (rest) |
| **Effort** | L |
| **Status** | ⏳ Not started |
| **Depends on** | WS-01 · decisions **D5**, **D6** |
| **Findings** | **C4** (EXT-1), H2 (TOOL-4 part), H5 (EXT-2), SEC-5, EXT-3 … EXT-9 |
| **Canonical docs** | `docs/EXTENSION_SYSTEM.md`, `src/tools/README.md`, `src/NETWORK_SECURITY.md`, `docs/PROCESS_ISOLATION.md`, `docs/SKILLS_ECOSYSTEM.md`, `FEATURE_PARITY.md` |

## Goal

A WASM extension gets exactly the secrets, hooks, and side-effect privileges the operator saw and approved — nothing it can name on its own. Install paths verify what they install. Native plugins, MCP servers, and skills honor their documented gates on every path (CLI, gateway, agent tool).

## Tasks

### Secrets (Critical C4)

- [ ] **6.1 — Scope credential secret names.**
  `crates/thinclaw-tools/src/wasm/capabilities_schema.rs:~235-245` only checks `secret_name` is an identifier; `crates/thinclaw-tools/src/wasm/wrapper.rs:~1642-1667` `resolve_host_credentials` → `store.get_for_injection(user_id, &mapping.secret_name, …)` with no namespace/owner/provider check (`crates/thinclaw-secrets/src/store.rs:~386`). Require names under the extension's namespace (`ext.<name>.*`) **or** an explicit per-extension grant table written only by an operator approval. Enforce at both manifest validation and injection time. Make the existing `CredentialInjector` `allowed_secrets` check (test-only today) the production path, or delete it and fix `src/NETWORK_SECURITY.md:~524-528`.

- [ ] **6.2 — Allowlist `auth.env_var`.**
  `capabilities_schema.rs:~436` accepts any env name; `src/extensions/manager/wasm.rs:~82-87` copies `std::env::var(env_var)` into the extension's secret (e.g. `ANTHROPIC_API_KEY`, `DATABASE_URL`, `SECRETS_MASTER_KEY` when env master keys are allowed). Restrict to an extension-scoped prefix or an operator-approved list; never allow known ThinClaw/provider/master-key variables.

- [ ] **6.3 — Activation is an approved, informed action.**
  `tool_activate` is `ApprovalRequirement::Never` and auto-runs `auth_with_context(name, None)` on "not authenticated" (`crates/thinclaw-tools/src/builtin/extension_tools.rs:~698`). Make activation require approval; show requested secrets, env vars, hosts, and hooks on the install/activate approval cards.

- [ ] **6.4 — WASM channel secrets.**
  `setup.required_secrets` names are unscoped (`crates/thinclaw-channels/src/wasm/schema.rs:~283-292`, `runtime_config.rs:~364-405`). Apply the 6.1 rule.

### Privileges and integrity

- [ ] **6.5 — Don't hand webhook secrets to guests (SEC-5).**
  `src/main_helpers.rs:~399-459` and `src/extensions/manager/wasm.rs:~629` pass the signature secret to `apply_channel_host_config`; `crates/thinclaw-channels/src/wasm/runtime_config.rs:~192` inserts `webhook_secret` into every channel's config (for Twilio this is the account Auth Token); the shared guest persists config to `~/.thinclaw/channels/<name>.workspace.json` (`channels-src/shared_webhook_channel/src/impl.rs:~88`, `crates/thinclaw-channels/src/wasm/wrapper/mod.rs:~229-232`). Inject only for channels that need it (Telegram `setWebhook`), and never persist it in guest workspace.

- [ ] **6.6 — Manifest hooks need consent (H5/EXT-2).**
  `src/extensions/manager/wasm.rs:~511-520` and `src/hooks/bootstrap.rs:~159-230` register a manifest's `hooks` block as `plugin.tool:<name>` rule hooks/webhooks at any priority/point; rule hooks can prepend/append/replace/reject (`src/hooks/bundled.rs:~390-440`), and modified text replaces the user's message (`src/agent/agent_loop/message_handling.rs:~132-139`) and tool params (`src/agent/dispatcher/tool_execution.rs:~170-180`). Require per-extension operator approval for hooks, cap priority below operator hooks, tag hook-modified content as extension-sourced, and document the feature in `docs/EXTENSION_SYSTEM.md`.

- [ ] **6.7 — Verify what we install (EXT-3).**
  `src/registry/manifest.rs:~156-180` (`to_registry_entry`) drops `artifact.sha256`; `src/extensions/manager/install.rs:~439-500` (`download_and_install_wasm`) checks neither hash nor `ALLOWED_ARTIFACT_HOSTS` (only the CLI path at `src/registry/installer.rs:~621-656` does); plugin-manifest WASM contributions ignore their sha256 (`src/extensions/registry.rs:~441`); capabilities sidecars are never hashed; `publish_extension_files(…, true)` (`install.rs:~494-499`) silently replaces existing extensions. Carry sha256 through `ExtensionSource::WasmDownload`, verify wasm + capabilities bytes, enforce the host allowlist, refuse replacement without force.

- [ ] **6.8 — Native-plugin gates are operator-local (EXT-4, D6).**
  `src/extensions/manager/native.rs:~31-36` reads `allow_native_plugins`, `allow_unsafe_in_process_native_plugins`, `require_plugin_signatures`, `trusted_manifest_public_keys`, `native_plugin_allowlist_dirs` from DB settings writable via `PUT /api/settings/{key}` (`crates/thinclaw-gateway/src/web/settings.rs:~169-185` filters only `is_sensitive_settings_key`), surfaced in the web UI by d40bb3a. The gateway activate handler accepts `native-plugin` (`src/channels/web/handlers/extensions.rs:~315`) and `tool_activate` reaches `activate_native_plugin` (`src/extensions/manager/lifecycle.rs:~229`). Per D6: move these keys to local config only (or mark sensitive/immutable over the API), reject native kind in gateway and agent activation.

- [ ] **6.9 — Default approval for WASM and un-annotated MCP tools (H2/TOOL-4, D5).**
  `crates/thinclaw-tools/src/wasm/wrapper.rs:~957` doesn't override `requires_approval` → trait default `Never` (`crates/thinclaw-tools-core/src/tool.rs:~445`), so e.g. gmail `send_message`/`trash_message` run unprompted. MCP tools without destructive/side-effect annotations → `Never` (`crates/thinclaw-tools-core/src/mcp.rs:~692`). Default both to `UnlessAutoApproved`; allow a manifest-declared read-only action allowlist (WASM) and honor `readOnlyHint` (MCP) to return `Never`.

- [ ] **6.10 — Skill gates the model can't flip (EXT-5).**
  `skill_install` honors LLM-supplied `approve_risky` to skip quarantine findings (`src/tools/builtin/skill_tools/install.rs:~178-186, ~220-226`); `skill_trust_promote` (`crates/thinclaw-tools/src/builtin/skill/trust.rs:~78`) and `skill_tap_add` with `trusted`/`builtin` (`tap.rs:~225-230, ~384`) are `UnlessAutoApproved`. Return `ApprovalRequirement::Always` for all three cases.

- [ ] **6.11 — Attenuation survives lazy skill loading (EXT-6).**
  Attenuation uses keyword-selected `active_skills` fixed at turn start (`src/agent/dispatcher/loop.rs:~650`) while every installed skill's description is in the prompt index and `skill_read` (`src/tools/builtin/skill_tools/read.rs:~43-57`) returns full `prompt_content` unattenuated. Apply attenuation when `skill_read` returns an Installed-trust skill, or hide Installed skills from the index while attenuation is active.

- [ ] **6.12 — MCP strict isolation binds (EXT-7).**
  `crates/thinclaw-tools/src/mcp/stdio.rs:~511-540` read-only-binds the executable's parent **and grandparent** plus sanitized PATH entries — `~/.cargo/bin/<server>` exposes `~/.cargo/credentials.toml`; `~/.local/bin/uvx` exposes `~/.local`. Bind only the executable and an explicit runtime list; never a grandparent under `$HOME`. Verify macOS seatbelt roots too. Fix `docs/PROCESS_ISOLATION.md`.

- [ ] **6.13 — Online discovery (EXT-8).**
  `src/extensions/discovery.rs:~129-200` turns top-5-by-stars GitHub repo homepages into `validated: true` MCP entries installable by name. Show the URL on the approval card, mark discovered entries unvalidated, and default their tools to approval (6.9).

- [ ] **6.14 — Leak-scan WASM responses fully (EXT-9).**
  `crates/thinclaw-tools/src/wasm/wrapper.rs:~605-616` returns response headers unscanned; the body scan (`:~649`) runs only for UTF-8. Scan/redact header values (e.g. `Location` echoing a query-param credential) and binary bodies.

- [ ] **6.15 — Docs + regression tests.**
  Tests: manifest naming a non-namespaced secret is rejected at install and at injection; `auth.env_var` for a provider key rejected; activation without approval refused; manifest hooks inert until approved; tampered artifact rejected on the agent/gateway path; native kind refused via gateway/tool. Docs: `docs/EXTENSION_SYSTEM.md` (secrets, hooks, approval defaults, native gates), `src/NETWORK_SECURITY.md` (`allowed_secrets`, response scanning), `docs/PROCESS_ISOLATION.md`, `docs/SKILLS_ECOSYSTEM.md`. Note WASM `workspace_read` is never wired (`capabilities_schema.rs:~164`, `reader: None`) — fix docs or wire it (tracked in WS-11).

## Verification (workstream exit)

```bash
cargo test --locked --workspace
cargo test --locked --features full --test registry_channel_catalog
./scripts/build-all.sh
```
