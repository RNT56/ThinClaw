# OpenClaw Renaming Strategy (Exhaustive)

This document outlines the comprehensive plan to rename all project elements from the legacy names (**Clawdbot** and **Moltbot**) to the unified stable branding: **OpenClaw**.

## 1. Directory & File Renames

### Directories
| Original Path | New Path | Description |
| :--- | :--- | :--- |
| `src-tauri/moltbot/` | `src-tauri/openclaw-engine/` | Node.js sidecar source |
| `src-tauri/src/clawdbot/` | `src-tauri/src/openclaw/` | Rust orchestration module |
| `src/components/clawdbot/` | `src/components/openclaw/` | UI components |

### Files
| Original File | New File | Description |
| :--- | :--- | :--- |
| `src/lib/clawdbot.ts` | `src/lib/openclaw.ts` | TS API library |
| `src-tauri/src/clawdbot/config.rs` | `src-tauri/src/openclaw/config.rs` | Module config |
| `documentation/clawdbot.md` | `documentation/openclaw.md` | Dev docs |
| `clawdbot_docs.md` | `openclaw_docs.md` | Root docs |
| `.../moltbot.json` | `.../openclaw.json` | Internal JSON config |

---

## 2. Rust Codebase (src-tauri)

### Namespaces & Branding
- **Tracing/Logs**: `info!("[clawdbot] ...")` → `info!("[openclaw] ...")`
- **Output Prefixes**: `[moltbot]` → `[openclaw-engine]` logic in stdout/stderr redirection.
- **Environment Variables**:
  - `CLAWDBOT_STATE_DIR` → `OPENCLAW_STATE_DIR`
  - `CLAWDBOT_PORT` → `OPENCLAW_PORT`
  - `MOLTBOT_CONFIG` → `OPENCLAW_ENGINE_CONFIG`

### Core Structs & Enums
- `ClawdbotManager` → `OpenClawManager`
- `ClawdbotConfig` → `OpenClawConfig`
- `MoltbotProcess` → `OpenClawEngineProcess`
- `MoltbotConfig` → `OpenClawEngineConfig`
- `ClawdbotWsClient` → `OpenClawWsClient`
- `ClawdbotWsHandle` → `OpenClawWsHandle`
- `ClawdbotStatus` → `OpenClawStatus`
- `ClawdbotSession` → `OpenClawSession`
- `ClawdbotSessionsResponse` → `OpenClawSessionsResponse`
- `ClawdbotMessage` → `OpenClawMessage`
- `ClawdbotHistoryResponse` → `OpenClawHistoryResponse`
- `ClawdbotRpcResponse` → `OpenClawRpcResponse`
- `ClawdbotSkillsStatus` → `OpenClawSkillsStatus`
- `ClawdbotDiagnostics` → `OpenClawDiagnostics`
- `CustomSecret` → `OpenClawCustomSecret`
- `Skill` → `OpenClawSkill`
- `CronJob` → `OpenClawCronJob`
- `CronHistoryItem` → `OpenClawCronHistoryItem`
- `SlackConfigInput` → `OpenClawSlackConfigInput`
- `TelegramConfigInput` → `OpenClawTelegramConfigInput`

### Module Imports & Paths
- **Rust Use Statements**: `use crate::clawdbot::...` → `use crate::openclaw::...`
- **Sidecar Binary References**: `"node"` sidecar usage in `SidecarManager` for OpenClaw.

### Tauri Commands (Standardizing to `openclaw_` or `get_openclaw_`)
| Original Rust Command | New Rust Command |
| :--- | :--- |
| `get_clawdbot_status` | `get_openclaw_status` |
| `clawdbot_toggle_secret_access` | `openclaw_toggle_secret_access` |
| `start_clawdbot_gateway` | `start_openclaw_gateway` |
| `stop_clawdbot_gateway` | `stop_openclaw_gateway` |
| `get_clawdbot_sessions` | `get_openclaw_sessions` |
| `get_clawdbot_history` | `get_openclaw_history` |
| `delete_clawdbot_session` | `delete_openclaw_session` |
| `send_clawdbot_message` | `send_openclaw_message` |
| `subscribe_clawdbot_session` | `subscribe_openclaw_session` |
| `abort_clawdbot_chat` | `abort_openclaw_chat` |
| `resolve_clawdbot_approval` | `resolve_openclaw_approval` |
| `get_clawdbot_diagnostics` | `get_openclaw_diagnostics` |
| `clear_clawdbot_memory` | `clear_openclaw_memory` |
| `get_clawdbot_memory` | `get_openclaw_memory` |
| `get_clawdbot_file` | `get_openclaw_file` |
| `write_clawdbot_file` | `write_openclaw_file` |
| `save_clawdbot_memory` | `save_openclaw_memory` |
| `list_workspace_files` | `openclaw_list_workspace_files` |
| `clawdbot_cron_list` | `openclaw_cron_list` |
| `clawdbot_cron_run` | `openclaw_cron_run` |
| `clawdbot_cron_history` | `openclaw_cron_history` |
| `clawdbot_skills_list` | `openclaw_skills_list` |
| `clawdbot_skills_status` | `openclaw_skills_status` |
| `clawdbot_skills_toggle` | `openclaw_skills_toggle` |
| `clawdbot_install_skill_repo` | `openclaw_install_skill_repo` |
| `clawdbot_install_skill_deps` | `openclaw_install_skill_deps` |
| `clawdbot_config_schema` | `openclaw_config_schema` |
| `clawdbot_config_get` | `openclaw_config_get` |
| `clawdbot_config_set` | `openclaw_config_set` |
| `clawdbot_config_patch` | `openclaw_config_patch` |
| `clawdbot_system_presence` | `openclaw_system_presence` |
| `clawdbot_logs_tail` | `openclaw_logs_tail` |
| `clawdbot_update_run` | `openclaw_update_run` |
| `clawdbot_web_login_whatsapp` | `openclaw_web_login_whatsapp` |
| `clawdbot_web_login_telegram` | `openclaw_web_login_telegram` |
| `clawdbot_toggle_custom_secret` | `openclaw_toggle_custom_secret` |
| `clawdbot_toggle_node_host` | `openclaw_toggle_node_host` |
| `clawdbot_toggle_local_inference` | `openclaw_toggle_local_inference` |
| `clawdbot_toggle_expose_inference` | `openclaw_toggle_expose_inference` |
| `clawdbot_set_setup_completed` | `openclaw_set_setup_completed` |
| `clawdbot_toggle_auto_start` | `openclaw_toggle_auto_start` |
| `clawdbot_set_dev_mode_wizard` | `openclaw_set_dev_mode_wizard` |
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
| `getClawdbotStatus` | `getOpenClawStatus` |
| `saveSlackConfig` | `saveSlackConfig` (Context preserved) |
| `saveTelegramConfig` | `saveTelegramConfig` |
| `saveGatewaySettings` | `saveGatewaySettings` |
| `startClawdbotGateway` | `startOpenClawGateway` |
| `stopClawdbotGateway` | `stopOpenClawGateway` |
| `getClawdbotSessions` | `getOpenClawSessions` |
| `getClawdbotHistory` | `getOpenClawHistory` |
| `sendClawdbotMessage` | `sendOpenClawMessage` |
| `subscribeClawdbotSession` | `subscribeOpenClawSession` |
| `abortClawdbotChat` | `abortOpenClawChat` |
| `resolveClawdbotApproval` | `resolveOpenClawApproval` |
| `getClawdbotDiagnostics` | `getOpenClawDiagnostics` |
| `clearClawdbotMemory` | `clearOpenClawMemory` |
| `getClawdbotMemory` | `getOpenClawMemory` |
| `getClawdbotFile` | `getOpenClawFile` |
| `writeClawdbotFile` | `writeOpenClawFile` |
| `getClawdbotCronList` | `getOpenClawCronList` |
| `runClawdbotCron` | `runOpenClawCron` |
| `getClawdbotCronHistory` | `getOpenClawCronHistory` |
| `getClawdbotSkillsList` | `getOpenClawSkillsList` |
| `getClawdbotSkillsStatus` | `getOpenClawSkillsStatus` |
| `toggleClawdbotSkill` | `toggleOpenClawSkill` |
| `toggleClawdbotNodeHost` | `toggleOpenClawNodeHost` |
| `toggleClawdbotLocalInference` | `toggleOpenClawLocalInference` |
| `toggleClawdbotExposeInference` | `toggleOpenClawExposeInference` |
| `selectClawdbotBrain` | `selectOpenClawBrain` |
| `installClawdbotSkillRepo` | `installOpenClawSkillRepo` |
| `installClawdbotSkillDeps` | `installOpenClawSkillDeps` |
| `getClawdbotConfigSchema` | `getOpenClawConfigSchema` |
| `getClawdbotConfig` | `getOpenClawConfig` |
| `patchClawdbotConfig` | `patchOpenClawConfig` |
| `getClawdbotSystemPresence` | `getOpenClawSystemPresence` |
| `getClawdbotLogsTail` | `getOpenClawLogsTail` |
| `runClawdbotUpdate` | `runOpenClawUpdate` |
| `loginClawdbotWhatsapp` | `loginOpenClawWhatsapp` |
| `loginClawdbotTelegram` | `loginOpenClawTelegram` |
| `toggleClawdbotAutoStart` | `toggleOpenClawAutoStart` |
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
| `ClawdbotAutomations` | `OpenClawAutomations` |
| `ClawdbotBrain` | `OpenClawBrain` |
| `ClawdbotChannels` | `OpenClawChannels` |
| `ClawdbotChatView` | `OpenClawChatView` |
| `ClawdbotDashboard` | `OpenClawDashboard` |
| `ClawdbotMemory` | `OpenClawMemory` |
| `ClawdbotPresence` | `OpenClawPresence` |
| `ClawdbotSidebar` | `OpenClawSidebar` |
| `ClawdbotSkills` | `OpenClawSkills` |
| `ClawdbotSystemControl` | `OpenClawSystemControl` |
| `LiveAgentStatus` | `LiveOpenClawStatus` |
| `MemoryEditor` | `OpenClawMemoryEditor` |

---

## 4. Cross-Cutting Changes

### IPC Events
- `clawdbot-event` → `openclaw-event` (Rust `app_handle.emit` and Hook `listen`)

### Workspace Tags & Strings
- `moltbot-bundled` → `openclaw-engine-bundled`
- `moltbot-repo` → `openclaw-engine-repo`

### `tauri.conf.json`
- `bundle > resources`: `"moltbot/**/*"` → `"openclaw-engine/**/*"`
- `bundle > resources`: Update any dylib/bin references if they move out of legacy paths.

### `package.json`
- `setup:moltbot` → `setup:openclaw`
- Update `setup:all` to call `setup:openclaw`.

---

## 5. Persistence & Migration

### Data Directory
- **Path**: `~/Library/Application Support/com.schack.scrappy/Clawdbot/` → `.../OpenClaw/`

### Migration Strategy
In `OpenClawConfig::new()`:
1. Check for `OpenClaw/` directory.
2. If missing, check for legacy `Clawdbot/`.
3. Execute a filesystem rename of `Clawdbot/` to `OpenClaw/`.
4. Update `~/.moltbot` internal refs if encountered (Rename `~/.moltbot` to `~/.openclaw`).
5. Update `DYLD_LIBRARY_PATH` and resource resolution logic in `commands.rs`.

---

## 6. Implementation Workflow

1. **Directories**: Move `src-tauri/moltbot` to `src-tauri/openclaw-engine` and `src-tauri/src/clawdbot` to `src-tauri/src/openclaw`.
2. **Search & Replace**: Automated batch S&R for both casing and naming variations.
3. **Bindings**: Run `npm run tauri dev` to regenerate `bindings.ts` and fix all broken TS imports.
4. **Validation**: Full build of Rust and TypeScript to ensure no missing references.
