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
    engine_running: boolean;
    engine_connected: boolean;
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
    allow_local_tools: boolean;
    workspace_mode: string;
    workspace_root: string | null;
    local_inference_enabled: boolean;
    selected_cloud_brain: string | null;
    selected_cloud_model: string | null;
    profiles: AgentProfile[];
    setup_completed: boolean;
    auto_start_gateway: boolean;
    dev_mode_wizard: boolean;
    /** When true, the agent runs tools without per-tool approval prompts. */
    auto_approve_tools: boolean;
    /** Whether the first-run identity bootstrap ritual has been completed. */
    bootstrap_completed: boolean;
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
    // Sprint 13 extensions (optional for backward compat)
    is_default?: boolean;
    status?: 'running' | 'paused' | 'error' | 'offline';
    session_count?: number;
    last_active_at?: string;
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
    tokensPerSec?: number;
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
    key: string;            // UUID of the routine
    name: string;           // display name
    description: string;
    schedule: string;       // 7-field cron expression
    nextRun?: string;       // ISO timestamp
    lastRun?: string;       // ISO timestamp
    lastStatus?: 'ok' | 'error' | string;
    enabled?: boolean;
    run_count?: number;
    action_type?: 'lightweight' | 'full_job' | 'heartbeat' | string;
    trigger_type?: 'cron' | 'event' | 'webhook' | 'manual' | 'system_event' | string;
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
    engine_running: boolean;
    engine_connected: boolean;
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
 * Start the IronClaw engine (in-process, no HTTP server)
 */
export async function startOpenClawGateway(): Promise<void> {
    return invoke('openclaw_start_gateway');
}

/**
 * Stop the IronClaw engine
 */
export async function stopOpenClawGateway(): Promise<void> {
    return invoke('openclaw_stop_gateway');
}

/**
 * Reload secrets (API keys) into the running IronClaw agent.
 * 
 * Performs a graceful engine restart to re-inject keys from macOS Keychain.
 * Call after saving or toggling API keys so the agent picks up changes
 * without requiring manual restart.
 */
export async function reloadSecrets(): Promise<void> {
    return invoke('openclaw_reload_secrets');
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
 * Resolve an approval request (3-tier: Deny / Allow Once / Allow Session)
 *
 * @param approvalId   Unique approval request ID from the agent
 * @param approved     Whether the action is approved (true) or denied (false)
 * @param allowSession If true, approve for the entire session (until engine restart)
 */
export async function resolveOpenClawApproval(
    approvalId: string,
    approved: boolean,
    allowSession: boolean = false
): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_resolve_approval', { approvalId, approved, allowSession });
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
 * Delete a file from the OpenClaw DB workspace.
 * Core seeded files (SOUL.md, IDENTITY.md, etc.) are protected.
 */
export async function deleteOpenClawFile(path: string): Promise<void> {
    return invoke('openclaw_delete_file', { path });
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

export interface RoutineAuditEntry {
    routine_key: string;
    started_at: string;
    completed_at: string | null;
    outcome: 'success' | 'failure' | 'skipped' | 'timeout' | string;
    duration_ms: number | null;
    error: string | null;
}

/** Fetch routine execution history from the RoutineAuditLog. */
export async function getRoutineAuditList(
    routineKey: string,
    limit?: number,
    outcome?: string,
): Promise<RoutineAuditEntry[]> {
    return invoke('openclaw_routine_audit_list', {
        routineKey,
        limit: limit ?? null,
        outcome: outcome ?? null,
    });
}

/** Clear routine run history. If routineKey is provided, clears only that routine's runs. */
export async function clearRoutineRuns(routineKey?: string): Promise<void> {
    await invoke('openclaw_clear_routine_runs', { key: routineKey ?? null });
}

// ============================================================================
// Channel listing
// ============================================================================

export interface ChannelInfo {
    id: string;
    name: string;
    type: 'wasm' | 'native' | 'builtin';
    enabled: boolean;
    stream_mode: string;
}

export interface ChannelsListResponse {
    channels: ChannelInfo[];
}

export async function getOpenClawChannelsList(): Promise<ChannelsListResponse> {
    return invoke('openclaw_channels_list');
}

// ============================================================================
// Cron expression linting
// ============================================================================

export interface CronLintResult {
    valid: boolean;
    expression: string;
    next_fire_times: string[];
    checked_at: string;
}

export async function lintCronExpression(expression: string): Promise<CronLintResult> {
    return invoke('openclaw_cron_lint', { expression });
}

export interface CreateRoutineResult {
    id: string;
    name: string;
    description: string;
    schedule: string;
    task: string;
    created_at: string;
}

/** Create a new scheduled routine. */
export async function createRoutine(
    name: string,
    description: string,
    schedule: string,
    task: string,
): Promise<CreateRoutineResult> {
    return invoke('openclaw_routine_create', { name, description, schedule, task });
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

export async function toggleOpenClawLocalTools(enabled: boolean): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_toggle_local_tools', { enabled });
}

export async function setOpenClawWorkspaceMode(mode: string, root: string | null): Promise<string> {
    return invoke('openclaw_set_workspace_mode', { mode, root });
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

export async function getOpenClawSystemPresence(): Promise<AgentRuntimePresence> {
    return invoke('openclaw_system_presence');
}

/** Live runtime data for the Agent Runtime / Presence panel. */
export interface AgentRuntimePresence {
    online: boolean;
    engine: string;
    mode: string;
    session_count: number;
    sub_agent_count: number;
    tool_count: number;
    hook_count: number;
    channel_count: number;
    routine_engine_running: boolean;
    uptime_secs: number | null;
}

export interface LogLine {
    timestamp: string;
    level: string;
    target: string;
    message: string;
}

export async function getOpenClawLogsTail(limit: number): Promise<{ logs: LogLine[]; lines: string[] }> {
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

export async function requestPermission(permission: string): Promise<{ accessibility: boolean, screen_recording: boolean }> {
    return invoke('request_permission', { permission });
}

export async function openPermissionSettings(permission: string): Promise<void> {
    return invoke('open_permission_settings', { permission });
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

// ── Sub-agent spawning types ─────────────────────────────────────────────

export interface SpawnSessionResponse {
    session_key: string;
    parent_session: string | null;
    task: string;
}

export interface ChildSessionInfo {
    session_key: string;
    task: string;
    status: 'running' | 'completed' | 'failed';
    spawned_at: number;
    result_summary: string | null;
}

/**
 * Spawn a new sub-agent session.
 *
 * @param agentId       Agent to spawn
 * @param task          Task description for the sub-agent
 * @param parentSession Optional parent session key for tracking
 */
export async function spawnSession(
    agentId: string,
    task: string,
    parentSession?: string
): Promise<SpawnSessionResponse> {
    return invoke('openclaw_spawn_session', {
        agentId,
        task,
        parentSession: parentSession ?? null,
    });
}

/**
 * List all child sessions spawned by a parent session.
 */
export async function listChildSessions(parentSession: string): Promise<ChildSessionInfo[]> {
    return invoke('openclaw_list_child_sessions', { parentSession });
}

/**
 * Update a sub-agent's status (mark as completed/failed).
 */
export async function updateSubAgentStatus(
    childSession: string,
    status: 'running' | 'completed' | 'failed',
    resultSummary?: string
): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_update_sub_agent_status', {
        childSession,
        status,
        resultSummary: resultSummary ?? null,
    });
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

// --- Canvas / A2UI Types ---

export type PanelPosition = 'right' | 'bottom' | 'center' | 'floating';
export type NotifyLevel = 'info' | 'success' | 'warning' | 'error';
export type ButtonStyle = 'primary' | 'secondary' | 'danger' | 'ghost';

export interface KvItem {
    key: string;
    value: string;
}

export interface FormFieldText { type: 'text'; name: string; label: string; placeholder?: string; required?: boolean; }
export interface FormFieldNumber { type: 'number'; name: string; label: string; min?: number; max?: number; }
export interface FormFieldSelect { type: 'select'; name: string; label: string; options: string[]; }
export interface FormFieldCheckbox { type: 'checkbox'; name: string; label: string; checked?: boolean; }
export interface FormFieldTextarea { type: 'textarea'; name: string; label: string; rows?: number; }
export type FormField = FormFieldText | FormFieldNumber | FormFieldSelect | FormFieldCheckbox | FormFieldTextarea;

export interface UiComponentText { type: 'text'; content: string; }
export interface UiComponentHeading { type: 'heading'; text: string; level?: number; }
export interface UiComponentTable { type: 'table'; headers: string[]; rows: string[][]; }
export interface UiComponentCode { type: 'code'; language: string; content: string; }
export interface UiComponentImage { type: 'image'; src: string; alt?: string; width?: number; }
export interface UiComponentProgress { type: 'progress'; label?: string; value: number; max: number; }
export interface UiComponentKeyValue { type: 'key_value'; items: KvItem[]; }
export interface UiComponentDivider { type: 'divider'; }
export interface UiComponentButton { type: 'button'; label: string; action: string; style?: ButtonStyle; }
export interface UiComponentForm { type: 'form'; form_id: string; fields: FormField[]; submit_label: string; }
export interface UiComponentJson { type: 'json'; data: any; collapsed?: boolean; }

export type UiComponent =
    | UiComponentText | UiComponentHeading | UiComponentTable | UiComponentCode
    | UiComponentImage | UiComponentProgress | UiComponentKeyValue | UiComponentDivider
    | UiComponentButton | UiComponentForm | UiComponentJson;

export interface CanvasActionShow {
    action: 'show';
    panel_id: string;
    title: string;
    components: UiComponent[];
    position?: PanelPosition;
    modal?: boolean;
}
export interface CanvasActionUpdate {
    action: 'update';
    panel_id: string;
    components: UiComponent[];
}
export interface CanvasActionDismiss {
    action: 'dismiss';
    panel_id: string;
}
export interface CanvasActionNotify {
    action: 'notify';
    message: string;
    level?: NotifyLevel;
    duration_secs?: number;
}
export type CanvasAction = CanvasActionShow | CanvasActionUpdate | CanvasActionDismiss | CanvasActionNotify;

/** Dispatch a canvas action event (button click, form submit) back to the agent. */
export async function canvasDispatchAction(
    sessionKey: string,
    eventType: string,
    payload: any,
    runId?: string
): Promise<OpenClawRpcResponse> {
    return invoke('openclaw_canvas_dispatch_event', { sessionKey, runId: runId ?? null, eventType, payload });
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

// ============================================================================
// New Feature API Functions
// ============================================================================

export interface ThinkingConfigResult {
    enabled: boolean;
    budget_tokens: number | null;
}

/**
 * Set thinking mode natively via IronClaw's ThinkingConfig.
 *
 * This replaces the old localStorage hack that prepended
 * "Think step by step" to messages.
 */
export async function setThinking(
    enabled: boolean,
    budgetTokens?: number
): Promise<ThinkingConfigResult> {
    return invoke('openclaw_set_thinking', {
        enabled,
        budgetTokens: budgetTokens ?? null,
    });
}

export interface MemorySearchResult {
    path: string;
    snippet: string;
    score: number;
}

export interface MemorySearchResponse {
    results: MemorySearchResult[];
    query: string;
    total: number;
}

/**
 * Search workspace memory using IronClaw's hybrid BM25+vector search.
 * Falls back to simple text search if vector search is unavailable.
 */
export async function searchMemory(
    query: string,
    limit?: number
): Promise<MemorySearchResponse> {
    return invoke('openclaw_memory_search', { query, limit: limit ?? null });
}

export interface SessionExportResponse {
    transcript: string;
    session_key: string;
    message_count: number;
}

/**
 * Export a session's history in the given format.
 * Supported: 'md' (default), 'json', 'txt', 'csv', 'html'
 */
export async function exportSession(
    sessionKey: string,
    format: string = 'md'
): Promise<SessionExportResponse> {
    return invoke('openclaw_export_session', { sessionKey, format });
}

// ============================================================================
// Hooks & Extensions Management
// ============================================================================

export interface HookInfoItem {
    name: string;
    hook_points: string[];
    failure_mode: string;
    timeout_ms: number;
    priority: number;
}

export interface HooksListResponse {
    hooks: HookInfoItem[];
    total: number;
}

/** List all registered lifecycle hooks. */
export async function listHooks(): Promise<HooksListResponse> {
    return invoke('openclaw_hooks_list');
}

export interface HookRegisterResponse {
    ok: boolean;
    hooks_registered: number;
    webhooks_registered: number;
    errors: number;
    message: string | null;
}

export interface HookUnregisterResponse {
    ok: boolean;
    removed: boolean;
    message: string | null;
}

/** Register a hook bundle from a JSON configuration. */
export async function registerHookBundle(bundleJson: string, source?: string): Promise<HookRegisterResponse> {
    return invoke('openclaw_hooks_register', { input: { bundle_json: bundleJson, source: source || null } });
}

/** Unregister (remove) a hook by name. */
export async function unregisterHook(hookName: string): Promise<HookUnregisterResponse> {
    return invoke('openclaw_hooks_unregister', { hookName });
}

export interface ExtensionInfoItem {
    name: string;
    kind: string;
    description: string | null;
    active: boolean;
    authenticated: boolean;
    tools: string[];
    needs_setup: boolean;
    activation_status: string | null;
    activation_error: string | null;
}

export interface ExtensionsListResponse {
    extensions: ExtensionInfoItem[];
    total: number;
}

export interface ExtensionActionResponse {
    ok: boolean;
    message: string | null;
}

/** List all installed extensions/plugins. */
export async function listExtensions(): Promise<ExtensionsListResponse> {
    return invoke('openclaw_extensions_list');
}

/** Activate an extension by name. */
export async function activateExtension(name: string): Promise<ExtensionActionResponse> {
    return invoke('openclaw_extension_activate', { name });
}

/** Remove an extension by name. */
export async function removeExtension(name: string): Promise<ExtensionActionResponse> {
    return invoke('openclaw_extension_remove', { name });
}

// ============================================================================
// Config Editor
// ============================================================================

export interface SettingItem {
    key: string;
    value: any;
    updated_at: string;
}

export interface SettingsListResponse {
    settings: SettingItem[];
}

/** List all IronClaw config settings. */
export async function listSettings(): Promise<SettingsListResponse> {
    return invoke('openclaw_config_get');
}

/** Set a single config setting. */
export async function setSetting(key: string, value: any): Promise<{ ok: boolean }> {
    return invoke('openclaw_config_set', { key, value });
}

/** Bulk-update settings. */
export async function patchSettings(patch: Record<string, any>): Promise<{ ok: boolean }> {
    return invoke('openclaw_config_patch', { patch });
}

// ============================================================================
// System Diagnostics
// ============================================================================

export interface DiagnosticCheck {
    name: string;
    status: 'pass' | 'fail' | 'warn' | 'skip';
    detail: string;
}

export interface DiagnosticsResponse {
    checks: DiagnosticCheck[];
    passed: number;
    failed: number;
    skipped: number;
}

/** Run system diagnostics. */
export async function runDiagnostics(): Promise<DiagnosticsResponse> {
    return invoke('openclaw_diagnostics');
}

// ============================================================================
// Tool Listing (for Tool Policies)
// ============================================================================

export interface ToolInfoItem {
    name: string;
    description: string;
    enabled: boolean;
    source: string; // 'builtin' | 'skill' | 'extension' | 'mcp'
}

export interface ToolsListResponse {
    tools: ToolInfoItem[];
    total: number;
}

/** List all registered tools with their status. */
export async function listTools(): Promise<ToolsListResponse> {
    return invoke('openclaw_tools_list');
}

/** Get the list of globally disabled tool names. */
export async function getDisabledTools(): Promise<string[]> {
    return invoke('openclaw_tool_policy_get');
}

/** Overwrite the list of globally disabled tool names. */
export async function setDisabledTools(disabledTools: string[]): Promise<void> {
    return invoke('openclaw_tool_policy_set', { disabledTools });
}

/** Toggle a single tool on/off. Returns the new enabled state. */
export async function toggleTool(toolName: string, currentlyEnabled: boolean): Promise<boolean> {
    const disabled = await getDisabledTools();
    let next: string[];
    if (currentlyEnabled) {
        // Currently enabled → disable it
        next = [...new Set([...disabled, toolName])];
    } else {
        // Currently disabled → enable it
        next = disabled.filter(n => n !== toolName);
    }
    await setDisabledTools(next);
    return !currentlyEnabled;
}

// ============================================================================
// DM Pairing Management
// ============================================================================

export interface PairingItem {
    channel: string;
    user_id: string;
    paired_at: string;
    status: 'active' | 'pending';
}

export interface PairingListResponse {
    pairings: PairingItem[];
    total: number;
}

/** List pairings for a channel (pending + approved). */
export async function listPairings(channel: string): Promise<PairingListResponse> {
    return invoke('openclaw_pairing_list', { channel });
}

/** Approve a pairing code for a channel. */
export async function approvePairing(channel: string, code: string): Promise<{ ok: boolean }> {
    return invoke('openclaw_pairing_approve', { channel, code });
}

// ============================================================================
// Context Compaction
// ============================================================================

export interface CompactSessionResponse {
    tokens_before: number;
    tokens_after: number;
    turns_removed: number;
    summary: string | null;
}

/** Trigger context compaction for a session. */
export async function compactSession(sessionKey: string): Promise<CompactSessionResponse> {
    return invoke('openclaw_compact_session', { sessionKey });
}

// ============================================================================
// Sprint 13 — New Backend APIs
// ============================================================================

// --- Cost Tracking ---

export interface CostSummary {
    total_cost_usd: number;
    total_input_tokens: number;
    total_output_tokens: number;
    total_requests: number;
    avg_cost_per_request: number;
    daily: Record<string, number>;
    monthly: Record<string, number>;
    by_model: Record<string, number>;
    by_agent: Record<string, number>;
    alert_threshold_usd: number;
    alert_triggered: boolean;
}

/** Get LLM cost summary with daily/monthly/per-model breakdowns. */
export async function getCostSummary(): Promise<CostSummary> {
    return invoke('openclaw_cost_summary');
}

/** Export cost data as CSV string. */
export async function exportCostCsv(): Promise<string> {
    return invoke('openclaw_cost_export_csv');
}

/** Reset (clear) all cost tracking data. Persists empty state to DB. */
export async function resetCostData(): Promise<void> {
    return invoke('openclaw_cost_reset');
}

// --- Channel Status ---

export interface ChannelStatusEntry {
    id: string;
    name: string;
    type: 'wasm' | 'native' | 'builtin';
    state: 'Running' | 'Connecting' | 'Degraded' | 'Disconnected' | 'Error';
    enabled: boolean;
    uptime_secs: number | null;
    messages_sent: number;
    messages_received: number;
    last_error: string | null;
    stream_mode: string;
}

/** Get all channel statuses with live state and counters. */
export async function getChannelStatusList(): Promise<ChannelStatusEntry[]> {
    return invoke('openclaw_channel_status_list');
}

// --- Agent Management ---

/** Set the default agent profile. */
export async function setDefaultAgent(agentId: string): Promise<void> {
    return invoke('openclaw_agents_set_default', { agentId });
}

// --- ClawHub ---

export interface ClawHubEntry {
    id: string;
    name: string;
    description: string;
    version: string;
    author: string;
    category: string;
    install_count: number;
    tags: string[];
}

/** Search ClawHub plugin catalog (proxied through IronClaw). */
export async function searchClawHub(query: string): Promise<{ entries: ClawHubEntry[] }> {
    return invoke('openclaw_clawhub_search', { query });
}

/** Install a plugin from ClawHub. */
export async function installFromClawHub(pluginId: string): Promise<void> {
    return invoke('openclaw_clawhub_install', { pluginId });
}


// --- Cache Stats ---


export interface CacheStats {
    hits: number;
    misses: number;
    evictions: number;
    size_bytes: number;
    hit_rate: number;
}

/** Get response cache statistics. */
export async function getCacheStats(): Promise<CacheStats> {
    return invoke('openclaw_cache_stats');
}

// --- Plugin Lifecycle ---

export interface LifecycleEventItem {
    timestamp: string;
    plugin_id: string;
    event_type: string; // 'installed' | 'activated' | 'deactivated' | 'removed' | 'error'
    details: string | null;
}

/** List plugin lifecycle events. */
export async function getPluginLifecycleList(): Promise<LifecycleEventItem[]> {
    return invoke('openclaw_plugin_lifecycle_list');
}

// --- Manifest Validation ---

export interface ManifestValidation {
    errors: string[];
    warnings: string[];
}

/** Validate a plugin's manifest. */
export async function validateManifest(pluginId: string): Promise<ManifestValidation> {
    return invoke('openclaw_manifest_validate', { pluginId });
}

// --- Smart Routing ---

/** Get current smart routing configuration. */
export async function getRoutingConfig(): Promise<{ smart_routing_enabled: boolean }> {
    return invoke('openclaw_routing_get');
}

/** Enable or disable smart routing. */
export async function setRoutingConfig(smartRoutingEnabled: boolean): Promise<void> {
    return invoke('openclaw_routing_set', { smartRoutingEnabled });
}

// --- Routing Rules ---

export interface RoutingRule {
    id: string;
    label: string;
    match_kind: 'keyword' | 'context_length' | 'provider' | 'always';
    match_value: string;
    target_model: string;
    target_provider: string | null;
    priority: number;
    enabled: boolean;
}

export interface RoutingRulesResponse {
    rules: RoutingRule[];
    smart_routing_enabled: boolean;
}

/** List all routing rules along with smart routing toggle state. */
export async function getRoutingRules(): Promise<RoutingRulesResponse> {
    return invoke('openclaw_routing_rules_list');
}

/** Save routing rules (full replace — ordered by priority). */
export async function saveRoutingRules(rules: RoutingRule[]): Promise<void> {
    return invoke('openclaw_routing_rules_save', { rules });
}

// --- Gmail OAuth PKCE ---

export interface GmailOAuthResult {
    success: boolean;
    access_token: string | null;
    refresh_token: string | null;
    expires_in: number | null;
    scope: string | null;
    error: string | null;
}

/**
 * Start the Gmail OAuth PKCE flow via IronClaw.
 * Opens a browser for Google consent, waits for callback, exchanges for tokens.
 * Returns the full result — caller should check `success` field.
 */
export async function startGmailOAuth(): Promise<GmailOAuthResult> {
    return invoke('openclaw_gmail_oauth_start');
}

// --- Routing Rule CRUD ---

/** Add a routing rule at position (or at the end). Returns updated rules list. */
export async function addRoutingRule(rule: RoutingRule, position?: number): Promise<RoutingRule[]> {
    return invoke('openclaw_routing_rules_add', { rule, position: position ?? null });
}

/** Remove a routing rule by index. Returns updated rules list. */
export async function removeRoutingRule(index: number): Promise<RoutingRule[]> {
    return invoke('openclaw_routing_rules_remove', { index });
}

/** Reorder a routing rule (move from one position to another). Returns updated rules list. */
export async function reorderRoutingRule(from: number, to: number): Promise<RoutingRule[]> {
    return invoke('openclaw_routing_rules_reorder', { from, to });
}

// --- Routing Status ---

export interface RoutingRuleSummary {
    index: number;
    kind: string;
    description: string;
    provider: string | null;
}

export interface LatencyEntry {
    provider: string;
    avg_latency_ms: number;
}

export interface RoutingStatusResponse {
    enabled: boolean;
    default_provider: string;
    rule_count: number;
    rules: RoutingRuleSummary[];
    latency_data: LatencyEntry[];
}

/** Get full routing policy status including latency data. */
export async function getRoutingStatus(): Promise<RoutingStatusResponse> {
    return invoke('openclaw_routing_status');
}

// --- Gmail Status ---

export interface GmailStatusResponse {
    enabled: boolean;
    configured: boolean;
    status: string;
    project_id: string;
    subscription_id: string;
    label_filters: string[];
    allowed_senders: string[];
    missing_fields: string[];
    oauth_configured: boolean;
}

/** Get Gmail channel configuration status. */
export async function getGmailStatus(): Promise<GmailStatusResponse> {
    return invoke('openclaw_gmail_status');
}

// ============================================================================
// Canvas Panel Management
// ============================================================================

export interface CanvasPanelSummary {
    panel_id: string;
    title: string;
}

export interface CanvasPanelData {
    panel_id: string;
    title: string;
    components: unknown;
    metadata?: unknown;
}

/** List all active canvas panels. */
export async function listCanvasPanels(): Promise<{ panels: CanvasPanelSummary[] }> {
    return safeInvoke('openclaw_canvas_panels_list');
}

/** Get full data for a specific canvas panel. */
export async function getCanvasPanel(panelId: string): Promise<CanvasPanelData | null> {
    return safeInvoke('openclaw_canvas_panel_get', { panelId });
}

/** Dismiss (remove) a canvas panel. */
export async function dismissCanvasPanel(panelId: string): Promise<boolean> {
    return safeInvoke('openclaw_canvas_panel_dismiss', { panelId });
}

// ============================================================================
// Routine Delete / Toggle
// ============================================================================

/** Delete a routine by ID or name. */
export async function deleteRoutine(routineId: string): Promise<{ ok: boolean; deleted_id: string }> {
    return safeInvoke('openclaw_routine_delete', { routineId });
}

/** Toggle a routine enabled/disabled. */
export async function toggleRoutine(routineId: string, enabled: boolean): Promise<{ ok: boolean; id: string; enabled: boolean }> {
    return safeInvoke('openclaw_routine_toggle', { routineId, enabled });
}

// ============================================================================
// Autonomy mode
// ============================================================================

/**
 * Enable or disable fully autonomous tool execution.
 * When enabled, the agent runs tools without per-tool approval prompts.
 * When disabled, the user approves each tool call (human-in-the-loop).
 * Takes effect on the next engine start.
 */
export async function setAutonomyMode(enabled: boolean): Promise<void> {
    return safeInvoke('openclaw_set_autonomy_mode', { enabled });
}

/** Get the current autonomy mode setting. */
export async function getAutonomyMode(): Promise<boolean> {
    return safeInvoke('openclaw_get_autonomy_mode');
}

// ============================================================================
// Bootstrap ritual
// ============================================================================

/**
 * Mark the first-run identity bootstrap ritual as completed.
 * Called by the frontend after the agent has finished naming itself.
 */
export async function setBootstrapCompleted(completed: boolean): Promise<void> {
    return safeInvoke('openclaw_set_bootstrap_completed', { completed });
}

/**
 * Check whether the bootstrap ritual needs to run.
 * Returns true if the agent has NOT yet completed the identity ritual.
 */
export async function checkBootstrapNeeded(): Promise<boolean> {
    return safeInvoke('openclaw_check_bootstrap_needed');
}

/**
 * Re-trigger the bootstrap ritual (Reinitiate Identity Ritual).
 * Resets bootstrap_completed so the modal shows on next startup.
 */
export async function triggerBootstrap(): Promise<void> {
    return safeInvoke('openclaw_trigger_bootstrap');
}

// ── Workspace path & Finder reveal ────────────────────────────────────────────

/** Returns the local filesystem workspace root (e.g. ~/Scrappy). */
export async function getWorkspacePath(): Promise<string> {
    return safeInvoke('openclaw_get_workspace_path');
}

/** Opens the local workspace directory in Finder and returns the path. */
export async function revealWorkspace(): Promise<string> {
    return safeInvoke('openclaw_reveal_workspace');
}

export interface WorkspaceFile {
    path: string;
    absolute_path: string;
    size: number;
    modified_ms: number;
}

/** Lists all real files inside the agent_workspace filesystem directory. */
export async function listAgentWorkspaceFiles(): Promise<WorkspaceFile[]> {
    return safeInvoke('openclaw_list_agent_workspace_files');
}

/** Reveals a specific file in Finder (macOS) / Explorer (Windows). */
export async function revealFile(absolutePath: string): Promise<void> {
    return safeInvoke('openclaw_reveal_file', { path: absolutePath });
}

/**
 * Write content to a file in the agent's local `agent_workspace` directory.
 * Returns the absolute path of the written file.
 */
export async function writeAgentWorkspaceFile(relativePath: string, content: string): Promise<string> {
    return safeInvoke('openclaw_write_agent_workspace_file', { relativePath, content });
}

/**
 * Update the heartbeat interval (in minutes) at runtime.
 * Takes effect immediately — updates the DB routine schedule and persists to config.
 */
export async function setHeartbeatInterval(intervalMinutes: number): Promise<any> {
    return safeInvoke('openclaw_heartbeat_set_interval', { intervalMinutes });
}
