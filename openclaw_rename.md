# OpenClaw Renaming Strategy (Exhaustive)

This document outlines the comprehensive plan to rename all project elements from the legacy names (**OpenClaw** and **OpenClawEngine**) to the unified stable branding: **OpenClaw**.

## 1. Directory & File Renames

### Directories
| Original Path | New Path | Description |
| :--- | :--- | :--- |
| `src-tauri/openclaw-engine/` | `src-tauri/openclaw-engine/` | Node.js sidecar source |
| `src-tauri/src/openclaw/` | `src-tauri/src/openclaw/` | Rust orchestration module |
| `src/components/openclaw/` | `src/components/openclaw/` | UI components |

### Files
| Original File | New File | Description |
| :--- | :--- | :--- |
| `src/lib/openclaw.ts` | `src/lib/openclaw.ts` | TS API library |
| `src-tauri/src/openclaw/config.rs` | `src-tauri/src/openclaw/config.rs` | Module config |
| `documentation/openclaw.md` | `documentation/openclaw.md` | Dev docs |
| `openclaw_docs.md` | `openclaw_docs.md` | Root docs |
| `.../openclaw-engine.json` | `.../openclaw.json` | Internal JSON config |

---

## 2. Rust Codebase (src-tauri)

### Namespaces & Branding
- **Tracing/Logs**: `info!("[openclaw] ...")` → `info!("[openclaw] ...")`
- **Output Prefixes**: `[openclaw-engine]` → `[openclaw-engine]` logic in stdout/stderr redirection.
- **Environment Variables**:
  - `OPENCLAW_STATE_DIR` → `OPENCLAW_STATE_DIR`
  - `OPENCLAW_PORT` → `OPENCLAW_PORT`
  - `MOLTBOT_CONFIG` → `OPENCLAW_ENGINE_CONFIG`

### Core Structs & Enums
- `OpenClawManager` → `OpenClawManager`
- `OpenClawConfig` → `OpenClawConfig`
- `OpenClawEngineProcess` → `OpenClawEngineProcess`
- `OpenClawEngineConfig` → `OpenClawEngineConfig`
- `OpenClawWsClient` → `OpenClawWsClient`
- `OpenClawWsHandle` → `OpenClawWsHandle`
- `OpenClawStatus` → `OpenClawStatus`
- `OpenClawSession` → `OpenClawSession`
- `OpenClawSessionsResponse` → `OpenClawSessionsResponse`
- `OpenClawMessage` → `OpenClawMessage`
- `OpenClawHistoryResponse` → `OpenClawHistoryResponse`
- `OpenClawRpcResponse` → `OpenClawRpcResponse`
- `OpenClawSkillsStatus` → `OpenClawSkillsStatus`
- `OpenClawDiagnostics` → `OpenClawDiagnostics`
- `CustomSecret` → `OpenClawCustomSecret`
- `Skill` → `OpenClawSkill`
- `CronJob` → `OpenClawCronJob`
- `CronHistoryItem` → `OpenClawCronHistoryItem`
- `SlackConfigInput` → `OpenClawSlackConfigInput`
- `TelegramConfigInput` → `OpenClawTelegramConfigInput`

### Module Imports & Paths
- **Rust Use Statements**: `use crate::openclaw::...` → `use crate::openclaw::...`
- **Sidecar Binary References**: `"node"` sidecar usage in `SidecarManager` for OpenClaw.

### Tauri Commands (Standardizing to `openclaw_` or `get_openclaw_`)
| Original Rust Command | New Rust Command |
| :--- | :--- |
| `get_openclaw_status` | `get_openclaw_status` |
| `openclaw_toggle_secret_access` | `openclaw_toggle_secret_access` |
| `start_openclaw_gateway` | `start_openclaw_gateway` |
| `stop_openclaw_gateway` | `stop_openclaw_gateway` |
| `get_openclaw_sessions` | `get_openclaw_sessions` |
| `get_openclaw_history` | `get_openclaw_history` |
| `delete_openclaw_session` | `delete_openclaw_session` |
| `send_openclaw_message` | `send_openclaw_message` |
| `subscribe_openclaw_session` | `subscribe_openclaw_session` |
| `abort_openclaw_chat` | `abort_openclaw_chat` |
| `resolve_openclaw_approval` | `resolve_openclaw_approval` |
| `get_openclaw_diagnostics` | `get_openclaw_diagnostics` |
| `clear_openclaw_memory` | `clear_openclaw_memory` |
| `get_openclaw_memory` | `get_openclaw_memory` |
| `get_openclaw_file` | `get_openclaw_file` |
| `write_openclaw_file` | `write_openclaw_file` |
| `save_openclaw_memory` | `save_openclaw_memory` |
| `list_workspace_files` | `openclaw_list_workspace_files` |
| `openclaw_cron_list` | `openclaw_cron_list` |
| `openclaw_cron_run` | `openclaw_cron_run` |
| `openclaw_cron_history` | `openclaw_cron_history` |
| `openclaw_skills_list` | `openclaw_skills_list` |
| `openclaw_skills_status` | `openclaw_skills_status` |
| `openclaw_skills_toggle` | `openclaw_skills_toggle` |
| `openclaw_install_skill_repo` | `openclaw_install_skill_repo` |
| `openclaw_install_skill_deps` | `openclaw_install_skill_deps` |
| `openclaw_config_schema` | `openclaw_config_schema` |
| `openclaw_config_get` | `openclaw_config_get` |
| `openclaw_config_set` | `openclaw_config_set` |
| `openclaw_config_patch` | `openclaw_config_patch` |
| `openclaw_system_presence` | `openclaw_system_presence` |
| `openclaw_logs_tail` | `openclaw_logs_tail` |
| `openclaw_update_run` | `openclaw_update_run` |
| `openclaw_web_login_whatsapp` | `openclaw_web_login_whatsapp` |
| `openclaw_web_login_telegram` | `openclaw_web_login_telegram` |
| `openclaw_toggle_custom_secret` | `openclaw_toggle_custom_secret` |
| `openclaw_toggle_node_host` | `openclaw_toggle_node_host` |
| `openclaw_toggle_local_inference` | `openclaw_toggle_local_inference` |
| `openclaw_toggle_expose_inference` | `openclaw_toggle_expose_inference` |
| `openclaw_set_setup_completed` | `openclaw_set_setup_completed` |
| `openclaw_toggle_auto_start` | `openclaw_toggle_auto_start` |
| `openclaw_set_dev_mode_wizard` | `openclaw_set_dev_mode_wizard` |
| `save_anthropic_key` | `openclaw_save_anthropic_key` |
| `get_anthropic_key` | `openclaw_get_anthropic_key` |
| `save_brave_key` | `openclaw_save_brave_key` |
| `get_brave_key` | `openclaw_get_brave_key` |
| `save_openai_key` | `openclaw_save_openai_key` |
| `get_openai_key` | `openclaw_get_openai_key` |
| `save_openrouter_key` | `openclaw_save_openrouter_key` |
| `get_openrouter_key` | `openclaw_get_openrouter_key` |
| `save_gemini_key` | `openclaw_save_gemini_key` |
| `get_gemini_key` | `openclaw_get_gemini_key` |
| `save_groq_key` | `openclaw_save_groq_key` |
| `get_groq_key` | `openclaw_get_groq_key` |
| `save_selected_cloud_model` | `openclaw_save_selected_cloud_model` |
| `add_custom_secret` | `openclaw_add_custom_secret` |
| `remove_custom_secret` | `openclaw_remove_custom_secret` |
| `set_hf_token` | `openclaw_set_hf_token` |

---

## 3. Frontend Codebase (src)

### TypeScript API Functions (`src/lib/openclaw.ts`)
| Original TS Function | New TS Function |
| :--- | :--- |
| `getOpenClawStatus` | `getOpenClawStatus` |
| `saveSlackConfig` | `saveSlackConfig` (Context preserved) |
| `saveTelegramConfig` | `saveTelegramConfig` |
| `saveGatewaySettings` | `saveGatewaySettings` |
| `startOpenClawGateway` | `startOpenClawGateway` |
| `stopOpenClawGateway` | `stopOpenClawGateway` |
| `getOpenClawSessions` | `getOpenClawSessions` |
| `getOpenClawHistory` | `getOpenClawHistory` |
| `sendOpenClawMessage` | `sendOpenClawMessage` |
| `subscribeOpenClawSession` | `subscribeOpenClawSession` |
| `abortOpenClawChat` | `abortOpenClawChat` |
| `resolveOpenClawApproval` | `resolveOpenClawApproval` |
| `getOpenClawDiagnostics` | `getOpenClawDiagnostics` |
| `clearOpenClawMemory` | `clearOpenClawMemory` |
| `getOpenClawMemory` | `getOpenClawMemory` |
| `getOpenClawFile` | `getOpenClawFile` |
| `writeOpenClawFile` | `writeOpenClawFile` |
| `getOpenClawCronList` | `getOpenClawCronList` |
| `runOpenClawCron` | `runOpenClawCron` |
| `getOpenClawCronHistory` | `getOpenClawCronHistory` |
| `getOpenClawSkillsList` | `getOpenClawSkillsList` |
| `getOpenClawSkillsStatus` | `getOpenClawSkillsStatus` |
| `toggleOpenClawSkill` | `toggleOpenClawSkill` |
| `toggleOpenClawNodeHost` | `toggleOpenClawNodeHost` |
| `toggleOpenClawLocalInference` | `toggleOpenClawLocalInference` |
| `toggleOpenClawExposeInference` | `toggleOpenClawExposeInference` |
| `selectOpenClawBrain` | `selectOpenClawBrain` |
| `installOpenClawSkillRepo` | `installOpenClawSkillRepo` |
| `installOpenClawSkillDeps` | `installOpenClawSkillDeps` |
| `getOpenClawConfigSchema` | `getOpenClawConfigSchema` |
| `getOpenClawConfig` | `getOpenClawConfig` |
| `patchOpenClawConfig` | `patchOpenClawConfig` |
| `getOpenClawSystemPresence` | `getOpenClawSystemPresence` |
| `getOpenClawLogsTail` | `getOpenClawLogsTail` |
| `runOpenClawUpdate` | `runOpenClawUpdate` |
| `loginOpenClawWhatsapp` | `loginOpenClawWhatsapp` |
| `loginOpenClawTelegram` | `loginOpenClawTelegram` |
| `toggleOpenClawAutoStart` | `toggleOpenClawAutoStart` |
| `saveAnthropicKey` | `saveOpenClawAnthropicKey` |
| `getAnthropicKey` | `getOpenClawAnthropicKey` |
| `saveBraveKey` | `saveOpenClawBraveKey` |
| `getBraveKey` | `getOpenClawBraveKey` |
| `saveOpenaiKey` | `saveOpenClawOpenaiKey` |
| `getOpenaiKey` | `getOpenClawOpenaiKey` |
| `saveOpenrouterKey` | `saveOpenClawOpenrouterKey` |
| `getOpenrouterKey` | `getOpenClawOpenrouterKey` |
| `saveGeminiKey` | `saveOpenClawGeminiKey` |
| `getGeminiKey` | `getOpenClawGeminiKey` |
| `saveGroqKey` | `saveOpenClawGroqKey` |
| `getGroqKey` | `getOpenClawGroqKey` |
| `saveSelectedCloudModel` | `saveOpenClawSelectedCloudModel` |
| `addCustomSecret` | `addOpenClawCustomSecret` |
| `removeCustomSecret` | `removeOpenClawCustomSecret` |
| `setHfToken` | `setOpenClawHfToken` |

### React Components (`src/components/openclaw/`)
| Original Component | New Component |
| :--- | :--- |
| `OpenClawAutomations` | `OpenClawAutomations` |
| `OpenClawBrain` | `OpenClawBrain` |
| `OpenClawChannels` | `OpenClawChannels` |
| `OpenClawChatView` | `OpenClawChatView` |
| `OpenClawDashboard` | `OpenClawDashboard` |
| `OpenClawMemory` | `OpenClawMemory` |
| `OpenClawPresence` | `OpenClawPresence` |
| `OpenClawSidebar` | `OpenClawSidebar` |
| `OpenClawSkills` | `OpenClawSkills` |
| `OpenClawSystemControl` | `OpenClawSystemControl` |
| `LiveAgentStatus` | `LiveOpenClawStatus` |
| `MemoryEditor` | `OpenClawMemoryEditor` |

---

## 4. Cross-Cutting Changes

### IPC Events
- `openclaw-event` → `openclaw-event` (Rust `app_handle.emit` and Hook `listen`)

### Workspace Tags & Strings
- `openclaw-engine-bundled` → `openclaw-engine-bundled`
- `openclaw-engine-repo` → `openclaw-engine-repo`

### `tauri.conf.json`
- `bundle > resources`: `"openclaw-engine/**/*"` → `"openclaw-engine/**/*"`
- `bundle > resources`: Update any dylib/bin references if they move out of legacy paths.

### `package.json`
- `setup:openclaw-engine` → `setup:openclaw`
- Update `setup:all` to call `setup:openclaw`.

---

## 5. Persistence & Migration

### Data Directory
- **Path**: `~/Library/Application Support/com.schack.scrappy/OpenClaw/` → `.../OpenClaw/`

### Migration Strategy
In `OpenClawConfig::new()`:
1. Check for `OpenClaw/` directory.
2. If missing, check for legacy `OpenClaw/`.
3. Execute a filesystem rename of `OpenClaw/` to `OpenClaw/`.
4. Update `~/.openclaw-engine` internal refs if encountered (Rename `~/.openclaw-engine` to `~/.openclaw`).
5. Update `DYLD_LIBRARY_PATH` and resource resolution logic in `commands.rs`.

---

## 6. Implementation Workflow

1. **Directories**: Move `src-tauri/openclaw-engine` to `src-tauri/openclaw-engine` and `src-tauri/src/openclaw` to `src-tauri/src/openclaw`.
2. **Search & Replace**: Automated batch S&R for both casing and naming variations.
3. **Bindings**: Run `npm run tauri dev` to regenerate `bindings.ts` and fix all broken TS imports.
4. **Validation**: Full build of Rust and TypeScript to ensure no missing references.
