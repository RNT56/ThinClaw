/**
 * OpenClaw API - wrappers for Tauri commands
 * 
 * These wrappers call the openclaw Tauri commands. The types match
 * the Rust structs in backend/src/openclaw/commands.rs
 */

import { invoke } from '@tauri-apps/api/core';
import { openPath as tauriOpenPath, revealItemInDir } from '@tauri-apps/plugin-opener';

/**
 * Guard wrapper: only call invoke when the Tauri runtime is available.
 * During Vite HMR reloads the IPC bridge can momentarily disappear,
 * which otherwise causes `Cannot read properties of undefined (reading 'invoke')`.
 */
function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    if (typeof window === 'undefined' || !(window as any).__TAURI_INTERNALS__) {
        return Promise.reject(new Error(`Tauri runtime not available (calling ${cmd})`));
    }
    return invoke<T>(cmd, args);
}

// ============================================================================
// Types (matching Rust types from commands.rs)
// ============================================================================

export interface OpenClawStatus {
    gateway_running: boolean;
    ws_connected: boolean;
    slack_enabled: boolean;
    telegram_enabled: boolean;
    port: number;
    gateway_mode: string;
    remote_url: string | null;
    remote_token: string | null;
    device_id: string;
    auth_token: string;
    state_dir: string;
    has_huggingface_token: boolean;
    huggingface_granted: boolean;
    has_anthropic_key: boolean;
    anthropic_granted: boolean;
    has_brave_key: boolean;
    brave_granted: boolean;
    has_openai_key: boolean;
    openai_granted: boolean;
    has_openrouter_key: boolean;
    openrouter_granted: boolean;
    has_gemini_key: boolean;
    gemini_granted: boolean;
    has_groq_key: boolean;
    groq_granted: boolean;
    node_host_enabled: boolean;
    local_inference_enabled: boolean;
    selected_cloud_brain: string | null;
    selected_cloud_model: string | null;
    profiles: AgentProfile[];
    setup_completed: boolean;
    auto_start_gateway: boolean;
    dev_mode_wizard: boolean;
    custom_llm_url: string | null;
    custom_llm_key: string | null;
    custom_llm_model: string | null;
    custom_llm_enabled: boolean;
    enabled_cloud_providers: string[];
    enabled_cloud_models: Record<string, string[]>;
}

export interface AgentProfile {
    id: string;
    name: string;
    url: string;
    token: string | null;
    mode: string;
    auto_connect: boolean;
}

export interface SlackConfigInput {
    enabled: boolean;
    bot_token: string | null;
    app_token: string | null;
}

export interface TelegramConfigInput {
    enabled: boolean;
    bot_token: string | null;
    dm_policy: string;
    groups_enabled: boolean;
}

export interface OpenClawSession {
    session_key: string;
    title: string | null;
    updated_at_ms: number | null;
    source: string | null;
}

export interface OpenClawSessionsResponse {
    sessions: OpenClawSession[];
}

export interface OpenClawMessage {
    id: string;
    role: string;
    ts_ms: number;
    text: string;
    source: string | null;
    metadata?: any;
}

export interface OpenClawHistoryResponse {
    messages: OpenClawMessage[];
    has_more: boolean;
}

export interface OpenClawRpcResponse {
    ok: boolean;
    message: string | null;
}

export interface CronJob {
    key: string;
    description: string;
    schedule: string;
    nextRun?: string;
    lastStatus?: 'ok' | 'error' | string;
    lastRun?: string;
}

export interface CronHistoryItem {
    timestamp: number;
    status: string;
    duration_ms: number;
    output?: string;
}

export interface Skill {
    skillKey: string;
    name: string;
    description: string;
    disabled: boolean;
    eligible: boolean;
    emoji?: string;
    homepage?: string;
    source: string;
    requirements?: {
        bins: string[];
    };
    missing?: {
        bins: string[];
    };
    install?: Array<{
        installId: string;
        type: string;
        bins: string[];
    }>;
}

export interface OpenClawSkillsStatus {
    skills: Skill[];
}

export interface OpenClawDiagnostics {
    timestamp: string;
    gateway_running: boolean;
    ws_connected: boolean;
    version: string;
    platform: string;
    port: number | null;
    state_dir: string | null;
    slack_enabled: boolean | null;
    telegram_enabled: boolean | null;
}

// ============================================================================
// API Functions
// ============================================================================

/**
 * Get current OpenClaw status
 */
export async function getOpenClawStatus(): Promise<OpenClawStatus> {
    return safeInvoke('openclaw_get_status');
}

/**
 * Save Slack configuration
 */
export async function saveSlackConfig(config: SlackConfigInput): Promise<void> {
    return invoke('openclaw_save_slack_config', { configInput: config });
}

/**
 * Save Telegram configuration
 */
export async function saveTelegramConfig(config: TelegramConfigInput): Promise<void> {
    return invoke('openclaw_save_telegram_config', { configInput: config });
}

/**
 * Save Anthropic API key
 */
export async function saveAnthropicKey(key: string): Promise<void> {
    return invoke('openclaw_save_anthropic_key', { key });
}

/**
 * Save Gateway configuration
 */
export async function saveGatewaySettings(
    mode: string,
    url: string | null,
    token: string | null
): Promise<void> {
    return invoke('openclaw_save_gateway_settings', { mode, url, token });
}

/**
 * Start the OpenClaw gateway (WS client)
 */
export async function startOpenClawGateway(): Promise<void> {
    return invoke('openclaw_start_gateway');
}

/**
 * Stop the OpenClaw gateway
 */
export async function stopOpenClawGateway(): Promise<void> {
    return invoke('openclaw_stop_gateway');
}

export interface CustomLlmConfigInput {
    url: string | null;
    key: string | null;
    model: string | null;
    enabled: boolean;
}

export async function saveCloudConfig(
    enabledProviders: string[],
    enabledModels: Record<string, string[]>,
    customLlm: CustomLlmConfigInput | null
): Promise<void> {
    return invoke('openclaw_save_cloud_config', { enabledProviders, enabledModels, customLlm });
}

/**
 * Get list of OpenClaw sessions
 */
export async function deleteOpenClawSession(sessionKey: string): Promise<void> {
    await invoke('openclaw_delete_session', { sessionKey });
}

export async function resetOpenClawSession(sessionKey: string): Promise<void> {
    await invoke('openclaw_reset_session', { sessionKey });
}

export async function getOpenClawSessions(): Promise<OpenClawSessionsResponse> {
    return invoke('openclaw_get_sessions');
}

/**
 * Get chat history for a session
 */
export async function getOpenClawHistory(
    sessionKey: string,
    limit: number,
    before?: string
): Promise<OpenClawHistoryResponse> {
    return invoke('openclaw_get_history', { sessionKey, limit, before: before ?? null });
}

/**
 * Send a message to a OpenClaw session
 */
export async function sendOpenClawMessage(
    sessionKey: string,
    text: string,
    deliver: boolean = true
): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_send_message', { sessionKey, text, deliver });
}

/**
 * Subscribe to a session for live updates
 */
export async function subscribeOpenClawSession(sessionKey: string): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_subscribe_session', { sessionKey });
}

/**
 * Abort a running chat
 */
export async function abortOpenClawChat(
    sessionKey: string,
    runId?: string
): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_abort_chat', { sessionKey, runId: runId ?? null });
}

/**
 * Resolve an approval request
 */
export async function resolveOpenClawApproval(
    approvalId: string,
    approved: boolean
): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_resolve_approval', { approvalId, approved });
}

/**
 * Get diagnostic information
 */
export async function getOpenClawDiagnostics(): Promise<OpenClawDiagnostics> {
    return invoke('openclaw_get_diagnostics');
}

/**
 * Clear OpenClaw memory (deletes memory directory)
 */
/**
 * Clear OpenClaw memory (deletes memory directory)
 */
export async function clearOpenClawMemory(target: 'memory' | 'identity' | 'all'): Promise<void> {
    return invoke('openclaw_clear_memory', { target });
}

/**
 * Get OpenClaw memory content (MEMORY.md)
 */
export async function getOpenClawMemory(): Promise<string> {
    return invoke('openclaw_get_memory');
}

/**
 * Get content of a specific file in the OpenClaw workspace
 */
export async function getOpenClawFile(path: string): Promise<string> {
    return invoke('openclaw_get_file', { path });
}

/**
 * List all markdown files in the OpenClaw workspace
 */
export async function listWorkspaceFiles(): Promise<string[]> {
    return invoke('openclaw_list_workspace_files');
}

/**
 * Write content to a specific file in the OpenClaw workspace
 */
export async function writeOpenClawFile(path: string, content: string): Promise<void> {
    return invoke('openclaw_write_file', { path, content });
}

/**
 * Open a path in the system file manager
 */
export async function openPath(path: string): Promise<void> {
    return tauriOpenPath(path);
}

export async function revealPath(path: string): Promise<void> {
    return revealItemInDir(path);
}

// ============================================================================
// New OpenClaw Gateway RPC Methods
// ============================================================================

export async function getOpenClawCronList(): Promise<CronJob[]> {
    return invoke('openclaw_cron_list');
}

export async function runOpenClawCron(key: string): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_cron_run', { key });
}

export async function getOpenClawCronHistory(key: string, limit: number): Promise<CronHistoryItem[]> {
    return invoke('openclaw_cron_history', { key, limit });
}

export async function getOpenClawSkillsList(): Promise<Skill[]> {
    return invoke('openclaw_skills_list');
}

export async function getOpenClawSkillsStatus(): Promise<OpenClawSkillsStatus> {
    return invoke('openclaw_skills_status');
}

export async function toggleOpenClawSkill(key: string, enabled: boolean): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_skills_toggle', { key, enabled });
}

export async function toggleOpenClawNodeHost(enabled: boolean): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_toggle_node_host', { enabled });
}

export async function toggleOpenClawLocalInference(enabled: boolean): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_toggle_local_inference', { enabled });
}

export async function toggleOpenClawExposeInference(enabled: boolean): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_toggle_expose_inference', { enabled });
}

export async function selectOpenClawBrain(brain: string | null): Promise<void> {
    return invoke('select_openclaw_brain', { brain });
}

export async function selectOpenClawModel(model: string | null): Promise<void> {
    return invoke('openclaw_save_selected_cloud_model', { model });
}

export async function installOpenClawSkillRepo(repoUrl: string): Promise<string> {
    return invoke('openclaw_install_skill_repo', { repoUrl });
}

export async function installOpenClawSkillDeps(name: string, installId?: string): Promise<void> {
    return invoke('openclaw_install_skill_deps', { name, installId });
}

export async function getOpenClawConfigSchema(): Promise<Record<string, any>> {
    return invoke('openclaw_config_schema');
}

export async function getOpenClawConfig(): Promise<Record<string, any>> {
    return invoke('openclaw_config_get');
}

export async function patchOpenClawConfig(patch: any): Promise<void> {
    return invoke('openclaw_config_patch', { patch });
}

export async function getOpenClawSystemPresence(): Promise<any> {
    return invoke('openclaw_system_presence');
}

export async function getOpenClawLogsTail(limit: number): Promise<{ lines: string[] }> {
    return invoke('openclaw_logs_tail', { limit });
}

export async function runOpenClawUpdate(): Promise<void> {
    return invoke('openclaw_update_run');
}

export async function loginOpenClawWhatsapp(): Promise<void> {
    return invoke('openclaw_web_login_whatsapp');
}

export async function loginOpenClawTelegram(): Promise<void> {
    return invoke('openclaw_web_login_telegram');
}



export async function getPermissionStatus(): Promise<{ accessibility: boolean, screen_recording: boolean }> {
    return invoke('get_permission_status');
}

export async function requestPermission(permission: string): Promise<void> {
    return invoke('request_permission', { permission });
}

export async function setSetupCompleted(completed: boolean): Promise<void> {
    return invoke('openclaw_set_setup_completed', { completed });
}

export async function addAgentProfile(profile: AgentProfile): Promise<void> {
    return invoke('openclaw_add_agent_profile', { profile });
}

export async function removeAgentProfile(id: string): Promise<void> {
    return invoke('openclaw_remove_agent_profile', { id });
}

export async function setHfToken(token: string): Promise<void> {
    return invoke('openclaw_set_hf_token', { token });
}

export async function toggleOpenClawAutoStart(enabled: boolean): Promise<void> {
    return invoke('openclaw_toggle_auto_start', { enabled });
}

export async function setDevModeWizard(enabled: boolean): Promise<void> {
    return invoke('openclaw_set_dev_mode_wizard', { enabled });
}

export async function switchToProfile(profileId: string): Promise<void> {
    return invoke('openclaw_switch_to_profile', { profileId });
}

export async function broadcastCommand(command: string): Promise<void> {
    return invoke('openclaw_broadcast_command', { command });
}

export async function verifyConnection(url: string, token: string | null): Promise<boolean> {
    return invoke('openclaw_test_connection', { url, token });
}

export interface AgentStatusSummary {
    id: string;
    name: string;
    url: string;
    online: boolean;
    latency_ms: number | null;
    version: string | null;
    stats: any | null;
    current_task: string | null;
    progress: number | null;
    logs: string[] | null;
    parent_id: string | null;
    children_ids: string[] | null;
    active_session_id: string | null;
    active: boolean;
    capabilities: string[] | null;
    run_status: string | null; // idle | processing | waiting_approval | error | offline
    model: string | null;
}

export async function getFleetStatus(): Promise<AgentStatusSummary[]> {
    return invoke('openclaw_get_fleet_status');
}

export async function spawnSession(agentId: string, task: string): Promise<string> {
    return invoke('openclaw_spawn_session', { agentId, task });
}

export async function getAgentsList(): Promise<AgentProfile[]> {
    return invoke('openclaw_agents_list');
}

export async function canvasPush(content: string): Promise<void> {
    return invoke('openclaw_canvas_push', { content });
}

export async function canvasNavigate(url: string): Promise<void> {
    return invoke('openclaw_canvas_navigate', { url });
}

export async function abortSession(sessionKey: string, runId?: string): Promise<void> {
    return invoke('openclaw_abort_chat', { sessionKey, runId: runId ?? null });
}

export async function dispatchCanvasEvent(
    sessionKey: string,
    eventType: string,
    payload: any,
    runId?: string
): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_canvas_dispatch_event', { sessionKey, runId, eventType, payload });
}

export async function syncLocalLlm(): Promise<void> {
    return invoke('openclaw_sync_local_llm');
}
