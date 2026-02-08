/**
 * Clawdbot API - wrappers for Tauri commands
 * 
 * These wrappers call the clawdbot Tauri commands. The types match
 * the Rust structs in src-tauri/src/clawdbot/commands.rs
 */

import { invoke } from '@tauri-apps/api/core';
import { openPath as tauriOpenPath, revealItemInDir } from '@tauri-apps/plugin-opener';

// ============================================================================
// Types (matching Rust types from commands.rs)
// ============================================================================

export interface ClawdbotStatus {
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
    node_host_enabled: boolean;
    local_inference_enabled: boolean;
    selected_cloud_brain: string | null;
    setup_completed: boolean;
    auto_start_gateway: boolean;
    dev_mode_wizard: boolean;
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

export interface ClawdbotSession {
    session_key: string;
    title: string | null;
    updated_at_ms: number | null;
    source: string | null;
}

export interface ClawdbotSessionsResponse {
    sessions: ClawdbotSession[];
}

export interface ClawdbotMessage {
    id: string;
    role: string;
    ts_ms: number;
    text: string;
    source: string | null;
    metadata?: any;
}

export interface ClawdbotHistoryResponse {
    messages: ClawdbotMessage[];
    has_more: boolean;
}

export interface ClawdbotRpcResponse {
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

export interface ClawdbotSkillsStatus {
    skills: Skill[];
}

export interface ClawdbotDiagnostics {
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
 * Get current Clawdbot status
 */
export async function getClawdbotStatus(): Promise<ClawdbotStatus> {
    return invoke('get_clawdbot_status');
}

/**
 * Save Slack configuration
 */
export async function saveSlackConfig(config: SlackConfigInput): Promise<void> {
    return invoke('save_slack_config', { configInput: config });
}

/**
 * Save Telegram configuration
 */
export async function saveTelegramConfig(config: TelegramConfigInput): Promise<void> {
    return invoke('save_telegram_config', { configInput: config });
}

/**
 * Save Anthropic API key
 */
export async function saveAnthropicKey(key: string): Promise<void> {
    return invoke('save_anthropic_key', { key });
}

/**
 * Save Gateway configuration
 */
export async function saveGatewaySettings(
    mode: string,
    url: string | null,
    token: string | null
): Promise<void> {
    return invoke('save_gateway_settings', { mode, url, token });
}

/**
 * Start the Clawdbot gateway (WS client)
 */
export async function startClawdbotGateway(): Promise<void> {
    return invoke('start_clawdbot_gateway');
}

/**
 * Stop the Clawdbot gateway
 */
export async function stopClawdbotGateway(): Promise<void> {
    return invoke('stop_clawdbot_gateway');
}

/**
 * Get list of Clawdbot sessions
 */
export async function deleteClawdbotSession(sessionKey: string): Promise<void> {
    await invoke('delete_clawdbot_session', { sessionKey });
}

export async function resetClawdbotSession(sessionKey: string): Promise<void> {
    await invoke('reset_clawdbot_session', { sessionKey });
}

export async function getClawdbotSessions(): Promise<ClawdbotSessionsResponse> {
    return invoke('get_clawdbot_sessions');
}

/**
 * Get chat history for a session
 */
export async function getClawdbotHistory(
    sessionKey: string,
    limit: number,
    before?: string
): Promise<ClawdbotHistoryResponse> {
    return invoke('get_clawdbot_history', { sessionKey, limit, before: before ?? null });
}

/**
 * Send a message to a Clawdbot session
 */
export async function sendClawdbotMessage(
    sessionKey: string,
    text: string,
    deliver: boolean = true
): Promise<ClawdbotRpcResponse> {
    return invoke('send_clawdbot_message', { sessionKey, text, deliver });
}

/**
 * Subscribe to a session for live updates
 */
export async function subscribeClawdbotSession(sessionKey: string): Promise<ClawdbotRpcResponse> {
    return invoke('subscribe_clawdbot_session', { sessionKey });
}

/**
 * Abort a running chat
 */
export async function abortClawdbotChat(
    sessionKey: string,
    runId?: string
): Promise<ClawdbotRpcResponse> {
    return invoke('abort_clawdbot_chat', { sessionKey, runId: runId ?? null });
}

/**
 * Resolve an approval request
 */
export async function resolveClawdbotApproval(
    approvalId: string,
    approved: boolean
): Promise<ClawdbotRpcResponse> {
    return invoke('resolve_clawdbot_approval', { approvalId, approved });
}

/**
 * Get diagnostic information
 */
export async function getClawdbotDiagnostics(): Promise<ClawdbotDiagnostics> {
    return invoke('get_clawdbot_diagnostics');
}

/**
 * Clear Clawdbot memory (deletes memory directory)
 */
/**
 * Clear Clawdbot memory (deletes memory directory)
 */
export async function clearClawdbotMemory(target: 'memory' | 'identity' | 'all'): Promise<void> {
    return invoke('clear_clawdbot_memory', { target });
}

/**
 * Get Clawdbot memory content (MEMORY.md)
 */
export async function getClawdbotMemory(): Promise<string> {
    return invoke('get_clawdbot_memory');
}

/**
 * Get content of a specific file in the Clawdbot workspace
 */
export async function getClawdbotFile(path: string): Promise<string> {
    return invoke('get_clawdbot_file', { path });
}

/**
 * List all markdown files in the Clawdbot workspace
 */
export async function listWorkspaceFiles(): Promise<string[]> {
    return invoke('list_workspace_files');
}

/**
 * Write content to a specific file in the Clawdbot workspace
 */
export async function writeClawdbotFile(path: string, content: string): Promise<void> {
    return invoke('write_clawdbot_file', { path, content });
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

export async function getClawdbotCronList(): Promise<CronJob[]> {
    return invoke('clawdbot_cron_list');
}

export async function runClawdbotCron(key: string): Promise<ClawdbotRpcResponse> {
    return invoke('clawdbot_cron_run', { key });
}

export async function getClawdbotCronHistory(key: string, limit: number): Promise<CronHistoryItem[]> {
    return invoke('clawdbot_cron_history', { key, limit });
}

export async function getClawdbotSkillsList(): Promise<Skill[]> {
    return invoke('clawdbot_skills_list');
}

export async function getClawdbotSkillsStatus(): Promise<ClawdbotSkillsStatus> {
    return invoke('clawdbot_skills_status');
}

export async function toggleClawdbotSkill(key: string, enabled: boolean): Promise<ClawdbotRpcResponse> {
    return invoke('clawdbot_skills_toggle', { key, enabled });
}

export async function toggleClawdbotNodeHost(enabled: boolean): Promise<ClawdbotRpcResponse> {
    return invoke('clawdbot_toggle_node_host', { enabled });
}

export async function toggleClawdbotLocalInference(enabled: boolean): Promise<ClawdbotRpcResponse> {
    return invoke('clawdbot_toggle_local_inference', { enabled });
}

export async function toggleClawdbotExposeInference(enabled: boolean): Promise<ClawdbotRpcResponse> {
    return invoke('clawdbot_toggle_expose_inference', { enabled });
}

export async function selectClawdbotBrain(brain: string | null): Promise<void> {
    return invoke('select_clawdbot_brain', { brain });
}

export async function installClawdbotSkillRepo(repoUrl: string): Promise<string> {
    return invoke('clawdbot_install_skill_repo', { repoUrl });
}

export async function installClawdbotSkillDeps(name: string, installId?: string): Promise<void> {
    return invoke('clawdbot_install_skill_deps', { name, installId });
}

export async function getClawdbotConfigSchema(): Promise<Record<string, any>> {
    return invoke('clawdbot_config_schema');
}

export async function getClawdbotConfig(): Promise<Record<string, any>> {
    return invoke('clawdbot_config_get');
}

export async function patchClawdbotConfig(patch: any): Promise<void> {
    return invoke('clawdbot_config_patch', { patch });
}

export async function getClawdbotSystemPresence(): Promise<any> {
    return invoke('clawdbot_system_presence');
}

export async function getClawdbotLogsTail(limit: number): Promise<{ lines: string[] }> {
    return invoke('clawdbot_logs_tail', { limit });
}

export async function runClawdbotUpdate(): Promise<void> {
    return invoke('clawdbot_update_run');
}

export async function loginClawdbotWhatsapp(): Promise<void> {
    return invoke('clawdbot_web_login_whatsapp');
}

export async function loginClawdbotTelegram(): Promise<void> {
    return invoke('clawdbot_web_login_telegram');
}



export async function getPermissionStatus(): Promise<{ accessibility: boolean, screen_recording: boolean }> {
    return invoke('get_permission_status');
}

export async function requestPermission(permission: string): Promise<void> {
    return invoke('request_permission', { permission });
}

export async function setSetupCompleted(completed: boolean): Promise<void> {
    return invoke('clawdbot_set_setup_completed', { completed });
}

export async function toggleClawdbotAutoStart(enabled: boolean): Promise<void> {
    return invoke('clawdbot_toggle_auto_start', { enabled });
}

export async function setDevModeWizard(enabled: boolean): Promise<void> {
    return invoke('clawdbot_set_dev_mode_wizard', { enabled });
}
