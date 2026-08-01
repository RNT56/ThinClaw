# Runtime Agent and Channel Capability Matrix

> **Temporary execution input.** This file is the exact capability reconciliation for the audited 0.16.0 runtime. It prevents implementation from treating an early raw count, a protected-name array, or a CLI command list as the source of truth. Delete it with the rest of this directory after generated canonical capability documentation and identity-parity tests exist.

## 1. Counting and parity rules

The audited source contains **124 statically named tool IDs** across all mutually conditional registration paths. That is a catalog maximum, not the number available in any one process. A runtime may also register names discovered from MCP servers, WASM components, operator user-tool manifests, or signed native-plugin manifests.

For every tool, keep these facts separate:

- catalogued in this build/source tree;
- compiled for the current target/profile;
- selected/configured;
- inserted into the live registry;
- dependency/auth ready;
- exposed to the current agent/profile/turn;
- approval policy;
- live health, including `unknown`, `not_probed`, and `not_supported`.

`registered` never means ready or exposed. The maximum `124` must not become a product assertion. Tests compare stable identities and descriptors to the final live registry.

CLI parity means an operator-facing domain service is shared with the agent tool. It does **not** mean invoking a tool implementation from the CLI or adding a leaf for every execution primitive.

Disposition vocabulary:

- **KEEP — agent-only:** real operational ability; show it in capability inventory, but no dedicated terminal leaf.
- **SHARE — CLI/admin:** expose the safe operator lifecycle/read operation through the same domain service.
- **OVERHAUL:** keep the capability but repair registration, provenance, readiness, policy, or naming.
- **HIDE BY POLICY:** registered but intentionally omitted from ordinary tool exposure until explicitly granted.

## 2. Exhaustive static agent-tool catalog

Every statically named ID appears exactly once in this table. Dynamic names are covered in section 3.

| Origin / maximum | Exact statically named IDs | Current registration predicate | CLI relationship and final disposition |
|---|---|---|---|
| Core / 10 | `echo`, `time`, `json`, `device_info`, `canvas`, `clarify`, `agent_think`, `emit_user_message`, `http`, `web_search` | Root-independent built-ins; all are inserted during base tool assembly. Readiness/policy still varies, especially network access. | **KEEP — agent-only.** These are execution/conversation primitives. `/think` UI removal does not remove `agent_think`; they are unrelated. Inventory them by identity and policy. |
| Optional content/integration / 3 | `extract_document`, `homeassistant`, `browser` | `extract_document` requires `document-extraction`; `homeassistant` requires usable `HASS_URL`/`HASS_TOKEN`; agent `browser` requires the Cargo `browser` feature and its backend. | **KEEP — agent-only / OVERHAUL reporting.** `dev browser` is a separate external-browser utility, not a mirror of agent `browser`. |
| Vision/reasoning / 2 | `vision_analyze`, `mixture_of_agents` | Vision is registered with the selected LLM; MoA only when its configured model set is viable. | **KEEP — agent-only.** Report model/dependency readiness; do not add diagnostic reasoning leaves. |
| Development/build / 11 | `shell`, `read_file`, `write_file`, `list_dir`, `apply_patch`, `grep`, `build_software`, `todo`, `process`, `execute_code`, `search_files` | Filesystem/shell/build depend on workspace mode and builder policy; process/execute-code depend on local/sandbox policy; search/todo have their own assembly predicates. Child processes can currently inherit unrelated ThinClaw/provider secrets plus real home/temp and executable-search context. | **KEEP — agent-only / SECURITY+POLICY OVERHAUL.** Do not create a second shell/file CLI. Route execution through the checked process-launch descriptor: sanitized environment, validated executable resolution, private/scope-appropriate home and temp, explicit filesystem/network/isolation, and no general secret grant. Only a separately typed eligible consumer may inject an exact slot; arbitrary shell/process/execute-code never can. Show `host_unconfined` honestly when selected, otherwise the exact containment/backend/network/home policy, descriptor ID, and approval in `status tools`. |
| Memory / 6 | `memory_search`, `session_search`, `memory_write`, `memory_read`, `memory_tree`, `memory_delete` | Workspace is currently constructed when DB exists; `session_search` also needs DB. Embeddings affect semantic quality, not all memory availability. | **SHARE — CLI/admin.** Existing `data memory` covers search/read/write/tree/status; add guarded `delete`. Durable `data conversations search` covers session search. Use one workspace/memory service. |
| Extension lifecycle / 6 | `tool_search`, `tool_install`, `tool_auth`, `tool_activate`, `tool_list`, `tool_remove` | Registered when `ExtensionManager` is constructed. It manages MCP, WASM tools/channels, and signed native-plugin registrations subject to kind-specific rules. The portable/gateway kind enums currently omit native plugins; auth/activate/remove are name-only; auth auto-activates; activation recognizes auth failure by message substrings. Separately, protected HTTP `/api/chat/auth-token`, WebSocket `AuthToken`, and pending conversational auth accept raw token text and then activate implicitly; the agent `tool_auth` input itself correctly has no token field. | **SHARE — CLI/admin / SECURITY OVERHAUL.** Converge lifecycle operations on typed four-kind manager ports. Replace raw HTTP/WebSocket/chat token ingestion with owner/kind/purpose-bound expiring `ExtensionAuthSession`s: server-side OAuth/PKCE, binding of a pre-authorized source ID, or local masked/stdin/env/file source creation. No remote DTO or transcript carries credential text, and clients without secure ingress fail `secure_input_unavailable`. Authentication binds one source and returns `next_action=activate` without activating. Add only the missing generic `extensions activate NAME [--kind KIND]` leaf through the running runtime—never an ephemeral CLI manager. Keep truthful native removal semantics (unload, operator artifact retained). Do not invent deactivate: there is no durable symmetric public operation. |
| Skills / 17 | `skill_inspect`, `skill_read`, `skill_list`, `skill_search`, `skill_check`, `skill_install`, `skill_update`, `skill_audit`, `skill_snapshot`, `skill_publish`, `skill_tap_list`, `skill_tap_add`, `skill_tap_remove`, `skill_tap_refresh`, `skill_remove`, `skill_reload`, `skill_trust_promote` | Registered only when skills are enabled and the registry/catalog/quarantine service is constructed; remote operations additionally need their hub/store/auth. | **SHARE — CLI/admin.** The 17-operation mapping in CR-02.9/section 7 of the leaf matrix remains exact. |
| Learning / 12 | `prompt_manage`, `skill_manage`, `learning_status`, `learning_outcomes`, `learning_history`, `learning_feedback`, `external_memory_recall`, `external_memory_export`, `external_memory_setup`, `external_memory_off`, `external_memory_status`, `learning_proposal_review` | Base ten require DB; `prompt_manage` additionally requires workspace and `skill_manage` a skill registry. The five external-memory tools are registered but hidden from ordinary exposure by default. `external_memory_setup` currently accepts an inline API key/arbitrary config and can activate/probe while configuring. Letta is registered but fails closed on memory operations because it lacks strict subject scoping. | **MIXED / SECURITY OVERHAUL.** Share status/outcomes/history/feedback/proposal review and external-memory status/configure/activate/deactivate through the comprehensive `data learning` service in CR-02.19; remove inline credentials, make configure zero-network, and keep configured/enabled/active/scope-safe facts distinct. Keep contextual prompt/skill mutation and provider recall/export agent-only. Preserve hidden-by-default grants for external memory; do not expose Letta as operational until scope-safe. |
| Repository projects / 12 | `repo_project_create`, `repo_project_plan`, `repo_project_status`, `repo_project_pause`, `repo_project_resume`, `repo_project_enroll`, `repo_project_setup`, `repo_project_approve`, `repo_project_request_credential`, `repo_project_set_credential`, `repo_project_list_repos`, `repo_project_connect` | Nine register with DB; set-credential/list-repos/connect additionally require a secrets store. Today `repo_project_set_credential` and the gateway DTO accept plaintext `value`, while setup binds unscoped secret names. | **SHARE — CLI/admin / SECURITY OVERHAUL.** `automation projects` remains the operator surface. The request tool emits only a typed out-of-band credential-source request; it never receives a value in the transcript. `repo_project_set_credential` becomes `{slot, secret_source_id}` binding over the four catalog slots, with the same owner/purpose checks as CLI/gateway. Local masked creation is handled by the shared secret service. |
| Media / 6 | `tts`, `image_generate`, `comfy_health`, `comfy_check_deps`, `comfy_run_workflow`, `comfy_manage` | TTS and four non-lifecycle Comfy tools are registered during assembly even when the external service is not ready; `comfy_manage` requires lifecycle management permission. | **MIXED.** Keep generation/TTS agent-only; `media comfy` shares health/dependency/generate/workflow/lifecycle services where the existing CLI already has an equivalent. Registration must not imply Comfy/TTS readiness. |
| Apple Mail / 1 | `apple_mail` | macOS, Apple Mail channel configured, and Envelope Index path available/auto-detected. | **KEEP — agent-only.** Channel configuration/status is operator-facing; mail search/send remains an approved agent operation. |
| Opt-in local sensors / 4 | `screen_capture`, `camera_capture`, `talk_mode`, `location` | `allow_local_tools` plus per-capability environment opt-in; screen/desktop evidence also respects the headless blocker. | **KEEP — agent-only.** Surface OS permission, headless, and opt-in reasons; never add direct capture CLI shortcuts. |
| Reckless desktop autonomy / 7 | `desktop_apps`, `desktop_ui`, `desktop_screen`, `desktop_calendar_native`, `desktop_numbers_native`, `desktop_pages_native`, `autonomy_control` | Reckless desktop-autonomy mode enabled and not blocked by a headless profile. | **KEEP — agent-only.** Clearly label high-risk policy and platform readiness; no normal-help exposure. |
| Injected hardware bridge / 3 | `capture_camera_frame`, `record_audio_clip`, `capture_screenshot` | A host `ToolBridge` is injected; all three use session approval and always require explicit approval at the tool layer. | **KEEP — agent-only.** Distinguish these host-bridge IDs from local `camera_capture`/`screen_capture`; report bridge origin and approval. |
| Channel actions / 2 | `send_message`, `nostr_actions` | `send_message` is registered after channel assembly, even if its callback later cannot route a requested platform; `nostr_actions` requires the `nostr` feature and initialized runtime. | **KEEP — agent-only.** Root `send` injects a prompt *to* the running agent and is not parity with outbound `send_message`. Report the distinction in help/docs. |
| Jobs / 6 | `create_job`, `list_jobs`, `job_status`, `cancel_job`, `job_events`, `job_prompt` | Four base tools always register at runtime; events require a store and prompt requires an injected prompt queue. Current startup registers/replaces this group before and after scheduler construction. | **SHARE — CLI/admin / OVERHAUL registration.** `automation jobs` uses gateway services. Delete provisional registration and register the complete group exactly once against the final scheduler/event store/prompt queue—never overwrite, double-count, or use a startup rebind exception. |
| Subagents / 3 | `spawn_subagent`, `list_subagents`, `cancel_subagent` | Registered after `SubagentExecutor` construction. Autonomous tool filtering intentionally excludes dispatcher-only subagent tools. These agent tools are distinct from the desktop bridge's process-local child-session API, which is correctly `LocalOnly` when a remote desktop profile is selected. | **KEEP — agent-only / KEEP desktop route gate.** Jobs and conversational delegation are the user workflows; do not add a raw subagent CLI that bypasses parent grants/context. Never translate a remote desktop child-session request into an embedded local spawn; a future remote route needs its own authenticated profile-targeted protocol. |
| LLM/advisor / 3 | `llm_select`, `llm_list_models`, `consult_advisor` | Selection/list register late; advisor is added/removed as advisor readiness changes. | **MIXED.** `config models` shares discovery/test/catalog services. Session model override and advisor consultation remain agent-only. Registry updates must emit a new capability revision. |
| Persistent agents / 5 | `create_agent`, `list_agents`, `update_agent`, `remove_agent`, `message_agent` | Registered after `AgentRegistry` construction/load; the registry may have no durable DB in diagnostic no-DB mode. | **MIXED.** Four CRUD operations share durable `agents`; `message_agent` remains active-runtime inter-agent messaging, not registry CRUD. |
| Routines / 5 | `routine_create`, `routine_list`, `routine_update`, `routine_delete`, `routine_history` | Currently registered inside `Agent::run` only when routine store/engine startup succeeds. | **SHARE — CLI/admin / OVERHAUL assembly.** Definition/history share the service; trigger reaches the live engine. Construct/register before the startup snapshot. |

Static maximum reconciliation: `10+3+2+11+6+6+17+12+12+6+1+4+7+3+2+6+3+3+5+5 = 124`.

## 3. Dynamic tool sources

| Source | Current behavior | Required end state |
|---|---|---|
| MCP | Each enabled server discovers protocol tool names and registers proxies. Prefixing is owned by the MCP client. Concurrent activation can produce a partial set. | Preflight a whole activation, reserve all final names atomically, record server/source/version provenance, and either insert all or return a typed conflict with none inserted. |
| WASM tool | Manifest/component declares the runtime name; registration rejects protected built-ins but depends on an incomplete manual protected list. | Validate signature/capabilities, resolve the final namespace, atomically register with artifact digest/source, and report installed/compiled/active/exposed separately. |
| Operator user tool | TOML-backed shell/proxy tools register dynamic names; a duplicate dynamic name can overwrite another today. | Reject collisions deterministically, record source path/digest without secret arguments, and require explicit replacement of the same owned source. |
| Signed native plugin | Operator allowlist scan registers contribution IDs; activation loads in-process code. | Keep default-off/signature/allowlist policy. Reserve names before load, identify full-host-privilege origin, and never allow collision fallback. |

Dynamic tools do not get generated static CLI leaves. The typed extension services and category-specific `extensions ...` commands manage their lifecycle; generic activation is the only missing public leaf. `status tools` reports their live descriptors.

## 4. Registry integrity contract

Current defects in `crates/thinclaw-tools/src/registry.rs`:

1. `register_sync` uses two `try_write` calls and returns no result; lock contention can silently omit a built-in.
2. `register` rejects a current built-in or manual `PROTECTED_TOOL_NAMES` match, but silently replaces an existing dynamic tool with the same name.
3. `register_builtin` silently overwrites any existing entry.
4. `PROTECTED_TOOL_NAMES` is a hand-maintained subset of the static universe. Examples missing from it include `web_search`, `grep`, `job_events`, `job_prompt`, `skill_reload`, `learning_outcomes`, the local sensor IDs, and the host-bridge IDs.
5. The registry stores a tool and a separate built-in-name set, but not authoritative origin/source/revision metadata.

Implement CR-02.17 with this contract:

```text
ToolOrigin = core | memory | dev | job | extension-admin | skill | learning |
             repo-project | media | desktop | hardware-bridge | channel |
             subagent | llm | agent | routine | wasm | mcp | user-tool | native-plugin
RegistrationRequest { tool, name, origin, source_id, source_digest?, replace }
RegistrationOutcome = inserted | rebound | unchanged | rejected { conflict }
RegistryEntry { tool, name, origin, source_id, source_digest?, revision, registered_at }
```

- Build the reserved static-name set from the same typed static catalog that drives descriptors; delete the divergent manual list.
- Replace the silent synchronous path with an infallible/collision-reporting startup builder or a short non-async metadata lock. Every call returns/propagates an outcome.
- Default collision policy is reject. Replacement is permitted only when the caller names the same origin/source and explicitly requests a rebind, or uses a separately authorized uninstall/install transaction.
- Remove the first temporary job registration and bind the complete job group exactly once after the final scheduler/event store/prompt queue are constructed. A generic same-source `rebound` remains valid for genuinely hot replaceable sources, but startup jobs may not use it as an ordering workaround.
- Advisor readiness and hot extension changes increment the registry revision and publish a capability-snapshot update.
- Registry snapshot identity is the exact sorted `(name, origin, source_id, revision)` set; counts are derived afterward.

Tests must cover contention, built-in/static collisions, dynamic/dynamic collisions, source-owned rebind, partial MCP activation rollback, WASM/user/native conflicts, concurrent register/unregister, job/advisor reconciliation, and exact registry-to-snapshot parity.

## 5. Channel catalog and runtime reconciliation

### 5.1 Current channel sources

| Source | Exact known IDs/variants today | Defect / disposition |
|---|---|---|
| Local runtime surfaces | `repl`, `tui`, and one-shot REPL message mode | **KEEP**, but do not report a fictional registered channel named `cli`. Model local surfaces separately from external ingress. |
| Root native channels | `signal`, `nostr`, `discord`, `imessage` (macOS), `apple_mail` (macOS), `bluebubbles`, `gmail`, `http`, `gateway` | **KEEP / OVERHAUL catalog and readiness.** Each needs compile/platform/config/register/dependency/exposure/health facts. |
| Native lifecycle descriptors | `matrix`, `voice-call`, `apns`, `browser-push` | **KEEP / OVERHAUL.** Some descriptors exist while activation fails or code is uncompiled; preserve that distinction. |
| Bundled registry/WASM sources | `dingtalk`, `discord`, `feishu_lark`, `google_chat`, `line`, `matrix`, `mattermost`, `ms_teams`, `qq`, `slack`, `telegram`, `twilio_sms`, `twitch`, `wecom`, `weixin`, `whatsapp` | **KEEP / OVERHAUL discovery.** The CLI hard-coded list currently knows only Telegram/Slack from this catalog and misses the other bundled entries until installed. |
| Additional installed WASM channels | Arbitrary validated manifest IDs under the configured channel directory | **KEEP dynamic.** List by manifest/runtime provenance without pretending every artifact is executable or active. |

The current `src/cli/channels.rs::KNOWN_CHANNELS` contains 16 entries (`gateway`, fictional aggregate `cli`, `signal`, four native-lifecycle IDs, `nostr`, `http`, `telegram`, `slack`, `discord`, `imessage`, `apple_mail`, `gmail`, `bluebubbles`). It is neither the runtime list nor the bundled catalog and must be deleted.

There are overlapping channel IDs (`discord` and `matrix`) across native and bundled-WASM implementations. A simple name set cannot represent this safely.

### 5.2 Target channel catalog

Create one typed catalog consumed by setup, CLI list/info/check/probe, runtime activation, web settings, capability snapshots, and generated docs:

```text
ChannelId = stable user-facing service ID
ChannelDriverId = native:<id> | lifecycle:<id> | wasm:<package/source> | local:<surface>
ChannelDescriptor {
  id, driver_id, source, display_name, compile_predicate, platform_predicate,
  config_schema_id, credential_reference_keys, dependency_probe, live_probe,
  ingress, egress, attachments, lifecycle, priority
}
ChannelSelection { id, driver_id, reason }
```

Rules:

1. A service ID can have multiple drivers. Config persists the selected driver; default migration prefers the currently configured native driver, otherwise the installed validated driver. Never activate two same-ID drivers implicitly.
2. Embedded registry manifests and installed dynamic manifests are catalog inputs, not hard-coded CLI exceptions.
3. `extensions channels list` returns service rows and selected-driver state; `info` shows all drivers and the selection reason. `--verbose`/JSON includes provenance.
4. `check-config` is side-effect-free. `probe` uses only a declared safe bounded probe and returns `not_supported` where none exists.
5. `run|tui|ask --channels none` disables **native, lifecycle, gateway, and WASM ingress**. This fixes `NativeChannelActivationPlan`, where gateway and WASM currently ignore `cli_only`. `configured` selects configured drivers; a CSV selects exact service IDs and errors on unknown/ambiguous unconfigured drivers.
6. Local REPL/TUI remain usable under `none` and are reported as local surfaces, not external channels.
7. Hot install/remove/activate updates the catalog/runtime snapshot revision transactionally.

## 6. Boot/runtime finalization sequence

The current boot presentation occurs before persisted WASM channel activation, subagent tools, LLM/advisor tools, persistent-agent tools, the final scheduler job binding, and routine tools (which are registered inside `Agent::run`). Therefore its tool/channel totals cannot be final.

Implement one explicit preparation boundary:

1. build resolved config, DB/workspace, providers, static tools, skills/learning/projects/media, and extension manager;
2. load/activate configured MCP, WASM, native plugins, channels, gateway, and outbound send bindings;
3. construct context manager, final scheduler/job host, subagent executor, model/advisor state, and persistent agent registry;
4. construct the routine engine and register routine tools without yet starting its background loops;
5. reconcile deliberate dynamic entries, seal startup revision `N`, and compare it to the live registry;
6. produce one credential-free `CapabilitySnapshot` from revision `N`;
7. render compact REPL boot or pass that exact snapshot into TUI bootstrap;
8. start prepared background loops/watchers; later dynamic changes publish revision `N+1` rather than mutating a supposedly final snapshot silently.

No screen may label an earlier assembly count “final.” Startup failure before step 6 emits a typed error and no optimistic boot card.

## 7. Operator CLI parity additions

These are the only additions surfaced by the exact tool reconciliation:

```text
extensions activate NAME [--kind mcp-server|wasm-tool|wasm-channel|native-plugin]
data memory delete PATH [--dry-run] [--yes]
data learning status [--live]
data learning history KIND [filters] [--limit N] [--cursor TOKEN]
data learning outcomes list|show|review|evaluate-now
data learning feedback submit TARGET_TYPE TARGET_ID --verdict VERDICT [--note TEXT]
data learning proposals list|show|review
data learning rollbacks record ARTIFACT_TYPE ARTIFACT_NAME --reason TEXT
data learning external-memory status [--live]
data learning external-memory configure PROVIDER [typed settings and SecretSourceId bindings]
data learning external-memory activate PROVIDER [--yes]
data learning external-memory deactivate [--yes]
```

They call extracted domain ports with explicit principal scope, not agent-tool `execute`. Before activation ships, the existing extension port and gateway/desktop API become four-kind-aware and typed for list/install/auth/activate/remove; auth ceases to auto-activate, and name ambiguity is non-mutating. HTTP/WebSocket/free-form-chat and desktop MCP raw-token forms are deleted together with their UI producers, codecs, stored pending state, and generated contracts; OAuth/session/source-ID completion is the only remote path. Terminal activation reaches the running runtime through `GatewayClient` so the returned revision/identities remain live. CR-02.19 additionally reconciles existing learning gateway capabilities (candidate/version/proposal/outcome/rollback administration) that are not separate agent tool IDs; those leaves do not alter the static count. `prompt_manage`, `skill_manage`, external-memory recall/export, outbound `send_message`, Nostr actions, subagents, sensors, desktop automation, and execution primitives remain agent-only for the reasons in section 2.

## 8. Completion gate

- Every static ID above exists in a generated catalog test; additions/removals require an explicit disposition update.
- Runtime registry identities equal the snapshot identities at startup revision `N` for every supported profile fixture.
- Every dynamic entry includes origin/source and collision outcome.
- Every operator-worthy parity leaf is present in the exhaustive leaf matrix and generated CLI reference.
- Every lifecycle mutation uses the D-54 execution policy and its returned registry/catalog revision; an active manager is never bypassed by a direct-store fallback.
- Every agent-only item remains discoverable through `status tools`/`/tools` with policy/readiness reasons.
- Channel catalog, setup choices, runtime activation, web settings, and CLI output use the same descriptor identities.
- Extension authentication has one source-bound session contract across CLI/agent/gateway/Web/desktop, no raw-token compatibility decoder, and no implicit activation.
