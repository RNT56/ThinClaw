/**
 * ThinClaw compatibility API
 *
 * Existing component-friendly names delegate to the generated binding client.
 * Command names, parameters, and result data are owned by bindings.ts.
 */

import { openPath as tauriOpenPath, revealItemInDir } from '@tauri-apps/plugin-opener';
import { compatibilityCommands } from './command-client';
import type { JsonValue } from './bindings';

function jsonValue(value: unknown): JsonValue {
    return value as JsonValue;
}

// ============================================================================
// Types (matching Rust types from commands.rs)
// ============================================================================

export interface CustomSecret {
    id: string;
    name: string;
    description: string | null;
    granted: boolean;
}

export interface ThinClawStatus {
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
    custom_secrets: CustomSecret[];
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
    // --- Extended cloud provider status ---
    has_xai_key: boolean;
    xai_granted: boolean;
    has_venice_key: boolean;
    venice_granted: boolean;
    has_together_key: boolean;
    together_granted: boolean;
    has_moonshot_key: boolean;
    moonshot_granted: boolean;
    has_minimax_key: boolean;
    minimax_granted: boolean;
    has_nvidia_key: boolean;
    nvidia_granted: boolean;
    has_qianfan_key: boolean;
    qianfan_granted: boolean;
    has_mistral_key: boolean;
    mistral_granted: boolean;
    has_xiaomi_key: boolean;
    xiaomi_granted: boolean;
    has_cohere_key: boolean;
    cohere_granted: boolean;
    has_voyage_key: boolean;
    voyage_granted: boolean;
    has_deepgram_key: boolean;
    deepgram_granted: boolean;
    has_elevenlabs_key: boolean;
    elevenlabs_granted: boolean;
    has_stability_key: boolean;
    stability_granted: boolean;
    has_fal_key: boolean;
    fal_granted: boolean;
    has_bedrock_key: boolean;
    bedrock_granted: boolean;
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

export interface ThinClawSession {
    session_key: string;
    title: string | null;
    updated_at_ms: number | null;
    source: string | null;
}

export interface ThinClawSessionsResponse {
    sessions: ThinClawSession[];
}

export interface ThinClawMessage {
    id: string;
    role: string;
    ts_ms: number;
    text: string;
    source: string | null;
    metadata?: any;
    tokensPerSec?: number;
}

export interface ThinClawHistoryResponse {
    messages: ThinClawMessage[];
    has_more: boolean;
}

export interface ThinClawRpcResponse {
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
    version?: string;
    trust?: string;
    keywords?: string[];
}

export interface ThinClawSkillsStatus {
    skills: Skill[];
}

export interface ThinClawDiagnostics {
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
 * Get current ThinClaw status
 */
export async function getThinClawStatus(): Promise<ThinClawStatus> {
    return compatibilityCommands.thinclawGetStatus();
}

/**
 * Save Slack configuration
 */
export async function saveSlackConfig(config: SlackConfigInput): Promise<void> {
    return compatibilityCommands.thinclawSaveSlackConfig(config);
}

/**
 * Save Telegram configuration
 */
export async function saveTelegramConfig(config: TelegramConfigInput): Promise<void> {
    return compatibilityCommands.thinclawSaveTelegramConfig(config);
}

/**
 * Save Anthropic API key
 */
export async function saveAnthropicKey(key: string): Promise<void> {
    return compatibilityCommands.thinclawSaveAnthropicKey(key);
}

/**
 * Save Gateway configuration
 */
export async function saveGatewaySettings(
    mode: string,
    url: string | null,
    token: string | null
): Promise<void> {
    return compatibilityCommands.thinclawSaveGatewaySettings(mode, url, token);
}

/**
 * Start the ThinClaw runtime (in-process, no HTTP server)
 */
export async function startThinClawGateway(): Promise<void> {
    return compatibilityCommands.thinclawStartGateway();
}

/**
 * Stop the ThinClaw runtime
 */
export async function stopThinClawGateway(): Promise<void> {
    return compatibilityCommands.thinclawStopGateway();
}

/**
 * Reload secrets (API keys) into the running ThinClaw runtime.
 *
 * Performs a graceful engine restart to re-inject keys from macOS Keychain.
 * Call after saving or toggling API keys so the agent picks up changes
 * without requiring manual restart.
 */
export async function reloadSecrets(): Promise<void> {
    return compatibilityCommands.thinclawReloadSecrets();
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
    return compatibilityCommands.thinclawSaveCloudConfig(enabledProviders, enabledModels, customLlm);
}

/**
 * Get list of ThinClaw sessions
 */
export async function deleteThinClawSession(sessionKey: string): Promise<void> {
    await compatibilityCommands.thinclawDeleteSession(sessionKey);
}

export async function resetThinClawSession(sessionKey: string): Promise<void> {
    await compatibilityCommands.thinclawResetSession(sessionKey);
}

export async function getThinClawSessions(): Promise<ThinClawSessionsResponse> {
    return compatibilityCommands.thinclawGetSessions();
}

/**
 * Get chat history for a session
 */
export async function getThinClawHistory(
    sessionKey: string,
    limit: number,
    before?: string
): Promise<ThinClawHistoryResponse> {
    return compatibilityCommands.thinclawGetHistory(sessionKey, limit, before ?? null);
}

/**
 * Send a message to a ThinClaw session
 */
export async function sendThinClawMessage(
    sessionKey: string,
    text: string,
    deliver: boolean = true
): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawSendMessage(sessionKey, text, deliver);
}

/**
 * Subscribe to a session for live updates
 */
export async function subscribeThinClawSession(sessionKey: string): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawSubscribeSession(sessionKey);
}

/**
 * Abort a running chat
 */
export async function abortThinClawChat(
    sessionKey: string,
    runId?: string
): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawAbortChat(sessionKey, runId ?? null);
}

/**
 * Resolve an approval request (3-tier: Deny / Allow Once / Allow Session)
 *
 * @param approvalId   Unique approval request ID from the agent
 * @param approved     Whether the action is approved (true) or denied (false)
 * @param allowSession If true, approve for the entire session (until engine restart)
 */
export async function resolveThinClawApproval(
    approvalId: string,
    approved: boolean,
    allowSession: boolean = false
): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawResolveApproval(approvalId, approved, allowSession);
}

/**
 * Get diagnostic information
 */
export async function getThinClawDiagnostics(): Promise<ThinClawDiagnostics> {
    return compatibilityCommands.thinclawGetDiagnostics();
}

/**
 * Clear ThinClaw memory (deletes memory directory)
 */
/**
 * Clear ThinClaw memory (deletes memory directory)
 */
export async function clearThinClawMemory(target: 'memory' | 'identity' | 'all'): Promise<void> {
    return compatibilityCommands.thinclawClearMemory(target);
}

/**
 * Get ThinClaw memory content (MEMORY.md)
 */
export async function getThinClawMemory(): Promise<string> {
    return compatibilityCommands.thinclawGetMemory();
}

/**
 * Get content of a specific file in the ThinClaw workspace
 */
export async function getThinClawFile(path: string): Promise<string> {
    return compatibilityCommands.thinclawGetFile(path);
}

/**
 * List all markdown files in the ThinClaw workspace
 */
export async function listWorkspaceFiles(): Promise<string[]> {
    return compatibilityCommands.thinclawListWorkspaceFiles();
}

/**
 * Write content to a specific file in the ThinClaw workspace
 */
export async function writeThinClawFile(path: string, content: string): Promise<void> {
    return compatibilityCommands.thinclawWriteFile(path, content);
}

/**
 * Delete a file from the ThinClaw DB workspace.
 * Core seeded files (SOUL.md, IDENTITY.md, etc.) are protected.
 */
export async function deleteThinClawFile(path: string): Promise<void> {
    return compatibilityCommands.thinclawDeleteFile(path);
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
// New ThinClaw Gateway RPC Methods
// ============================================================================

export async function getThinClawCronList(): Promise<CronJob[]> {
    return compatibilityCommands.thinclawCronList();
}

export async function runThinClawCron(key: string): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawCronRun(key);
}

export async function getThinClawCronHistory(key: string, limit: number): Promise<CronHistoryItem[]> {
    return compatibilityCommands.thinclawCronHistory(key, limit);
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
    return compatibilityCommands.thinclawRoutineAuditList(routineKey, limit ?? null, outcome ?? null);
}

/** Clear routine run history. If routineKey is provided, clears only that routine's runs. */
export async function clearRoutineRuns(routineKey?: string): Promise<void> {
    await compatibilityCommands.thinclawClearRoutineRuns(routineKey ?? null);
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

export async function getThinClawChannelsList(): Promise<ChannelsListResponse> {
    return compatibilityCommands.thinclawChannelsList();
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
    return compatibilityCommands.thinclawCronLint(expression);
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
    return compatibilityCommands.thinclawRoutineCreate(name, description, schedule, task);
}

export async function getThinClawSkillsList(): Promise<Skill[]> {
    return compatibilityCommands.thinclawSkillsList();
}

export async function getThinClawSkillsStatus(): Promise<ThinClawSkillsStatus> {
    return compatibilityCommands.thinclawSkillsStatus();
}

export async function toggleThinClawSkill(key: string, enabled: boolean): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawSkillsToggle(key, enabled);
}

export async function toggleThinClawLocalTools(enabled: boolean): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawToggleLocalTools(enabled);
}

export async function setThinClawWorkspaceMode(mode: string, root: string | null): Promise<string> {
    return compatibilityCommands.thinclawSetWorkspaceMode(mode, root);
}

export async function toggleThinClawLocalInference(enabled: boolean): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawToggleLocalInference(enabled);
}

export async function toggleThinClawExposeInference(enabled: boolean): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawToggleExposeInference(enabled);
}

export async function selectThinClawBrain(brain: string | null): Promise<void> {
    return compatibilityCommands.selectThinclawBrain(brain);
}

export async function selectThinClawModel(model: string | null): Promise<void> {
    return compatibilityCommands.thinclawSaveSelectedCloudModel(model);
}

export async function installThinClawSkillRepo(repoUrl: string): Promise<string> {
    return compatibilityCommands.thinclawInstallSkillRepo(repoUrl);
}

export async function installThinClawSkillDeps(name: string, installId?: string): Promise<void> {
    return compatibilityCommands.thinclawInstallSkillDeps(name, installId ?? null);
}

export interface SkillInfo {
    name: string;
    description: string;
    version: string;
    trust: string;
    source: string;
    keywords: string[];
}

export interface SkillSearchResponse {
    catalog: any[];
    installed: SkillInfo[];
    registry_url: string;
    catalog_error?: string | null;
}

export interface SkillActionResponse {
    success?: boolean;
    ok?: boolean;
    message?: string;
    [key: string]: any;
}

export async function searchSkillsCatalog(query: string): Promise<SkillSearchResponse> {
    return compatibilityCommands.thinclawSkillsSearch(query);
}

export async function installSkill(name: string, opts: { url?: string | null; content?: string | null; force?: boolean } = {}): Promise<SkillActionResponse> {
    return compatibilityCommands.thinclawSkillInstall(name, opts.url ?? null, opts.content ?? null, opts.force ?? false);
}

export async function removeSkill(name: string): Promise<SkillActionResponse> {
    return compatibilityCommands.thinclawSkillRemove(name);
}

export async function setSkillTrust(name: string, trust: string): Promise<SkillActionResponse> {
    return compatibilityCommands.thinclawSkillTrust(name, trust);
}

export async function reloadSkill(name: string): Promise<SkillActionResponse> {
    return compatibilityCommands.thinclawSkillReload(name);
}

export async function reloadAllSkills(): Promise<SkillActionResponse> {
    return compatibilityCommands.thinclawSkillsReloadAll();
}

export async function inspectSkill(
    name: string,
    opts: { includeContent?: boolean; includeFiles?: boolean; audit?: boolean } = {},
): Promise<any> {
    return compatibilityCommands.thinclawSkillInspect(name, opts.includeContent ?? false, opts.includeFiles ?? true, opts.audit ?? true);
}

export async function publishSkill(
    name: string,
    targetRepo: string,
    opts: { dryRun?: boolean; remoteWrite?: boolean; confirmRemoteWrite?: boolean; approveRisky?: boolean } = {},
): Promise<any> {
    return compatibilityCommands.thinclawSkillPublish(name, targetRepo, opts.dryRun ?? true, opts.remoteWrite ?? false, opts.confirmRemoteWrite ?? false, opts.approveRisky ?? false);
}

export async function getThinClawConfigSchema(): Promise<Record<string, any>> {
    return compatibilityCommands.thinclawConfigSchema();
}

export async function getThinClawConfig(): Promise<Record<string, any>> {
    return compatibilityCommands.thinclawConfigGet();
}

export async function patchThinClawConfig(patch: any): Promise<void> {
    return compatibilityCommands.thinclawConfigPatch(patch);
}

export async function getThinClawSystemPresence(): Promise<AgentRuntimePresence> {
    return compatibilityCommands.thinclawSystemPresence();
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

export async function getThinClawLogsTail(limit: number): Promise<{ logs: LogLine[]; lines: string[] }> {
    return compatibilityCommands.thinclawLogsTail(limit);
}

export async function runThinClawUpdate(): Promise<void> {
    return compatibilityCommands.thinclawUpdateRun();
}

export async function loginThinClawWhatsapp(): Promise<void> {
    return compatibilityCommands.thinclawWebLoginWhatsapp();
}

export async function loginThinClawTelegram(): Promise<void> {
    return compatibilityCommands.thinclawWebLoginTelegram();
}



export async function getPermissionStatus(): Promise<{ accessibility: boolean, screen_recording: boolean }> {
    return compatibilityCommands.getPermissionStatus();
}

export async function requestPermission(permission: string): Promise<{ accessibility: boolean, screen_recording: boolean }> {
    return compatibilityCommands.requestPermission(permission);
}

export async function openPermissionSettings(permission: string): Promise<void> {
    return compatibilityCommands.openPermissionSettings(permission);
}

export async function setSetupCompleted(completed: boolean): Promise<void> {
    return compatibilityCommands.thinclawSetSetupCompleted(completed);
}

export async function addAgentProfile(profile: AgentProfile): Promise<void> {
    return compatibilityCommands.thinclawAddAgentProfile(profile);
}

export async function removeAgentProfile(id: string): Promise<void> {
    return compatibilityCommands.thinclawRemoveAgentProfile(id);
}

export async function setHfToken(token: string): Promise<void> {
    return compatibilityCommands.thinclawSetHfToken(token);
}

export async function toggleThinClawAutoStart(enabled: boolean): Promise<void> {
    return compatibilityCommands.thinclawToggleAutoStart(enabled);
}

export async function setDevModeWizard(enabled: boolean): Promise<void> {
    return compatibilityCommands.thinclawSetDevModeWizard(enabled);
}

export async function switchToProfile(profileId: string): Promise<void> {
    return compatibilityCommands.thinclawSwitchToProfile(profileId);
}

export async function broadcastCommand(command: string): Promise<void> {
    return compatibilityCommands.thinclawBroadcastCommand(command);
}

export async function verifyConnection(url: string, token: string | null): Promise<boolean> {
    return compatibilityCommands.thinclawTestConnection(url, token);
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
    return compatibilityCommands.thinclawGetFleetStatus();
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
    return compatibilityCommands.thinclawSpawnSession(agentId, task, parentSession ?? null);
}

/**
 * List all child sessions spawned by a parent session.
 */
export async function listChildSessions(parentSession: string): Promise<ChildSessionInfo[]> {
    return compatibilityCommands.thinclawListChildSessions(parentSession);
}

/**
 * Update a sub-agent's status (mark as completed/failed).
 */
export async function updateSubAgentStatus(
    childSession: string,
    status: 'running' | 'completed' | 'failed',
    resultSummary?: string
): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawUpdateSubAgentStatus(childSession, status, resultSummary ?? null);
}

export async function getAgentsList(): Promise<AgentProfile[]> {
    return compatibilityCommands.thinclawAgentsList();
}

export async function canvasPush(content: string): Promise<void> {
    return compatibilityCommands.thinclawCanvasPush(content);
}

export async function canvasNavigate(url: string): Promise<void> {
    return compatibilityCommands.thinclawCanvasNavigate(url);
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
): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawCanvasDispatchEvent(sessionKey, runId ?? null, eventType, payload);
}

export async function abortSession(sessionKey: string, runId?: string): Promise<void> {
    return compatibilityCommands.thinclawAbortChat(sessionKey, runId ?? null);
}

export async function dispatchCanvasEvent(
    sessionKey: string,
    eventType: string,
    payload: any,
    runId?: string
): Promise<ThinClawRpcResponse> {
    return compatibilityCommands.thinclawCanvasDispatchEvent(sessionKey, runId ?? null, eventType, payload);
}

export async function syncLocalLlm(): Promise<void> {
    return compatibilityCommands.thinclawSyncLocalLlm();
}

// ============================================================================
// New Feature API Functions
// ============================================================================

export interface ThinkingConfigResult {
    enabled: boolean;
    budget_tokens: number | null;
}

/**
 * Set thinking mode natively via ThinClaw's ThinkingConfig.
 *
 * This replaces the old localStorage hack that prepended
 * "Think step by step" to messages.
 */
export async function setThinking(
    enabled: boolean,
    budgetTokens?: number
): Promise<ThinkingConfigResult> {
    return compatibilityCommands.thinclawSetThinking(enabled, budgetTokens ?? null);
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
 * Search workspace memory using ThinClaw's hybrid BM25+vector search.
 * Falls back to simple text search if vector search is unavailable.
 */
export async function searchMemory(
    query: string,
    limit?: number
): Promise<MemorySearchResponse> {
    return compatibilityCommands.thinclawMemorySearch(query, limit ?? null);
}

/** Rendered cross-session transcript search results. */
export interface SessionSearchResult {
    results: ThinClawJson[];
    summarized: boolean;
    fallback: boolean;
}

/**
 * Search stored conversation transcripts across sessions (local/embedded mode).
 * Optionally LLM-summarizes matching sessions.
 */
export async function searchSessions(
    query: string,
    limit?: number | null,
    summarize?: boolean | null,
): Promise<SessionSearchResult> {
    return compatibilityCommands.thinclawSessionSearch(query, limit ?? null, summarize ?? null);
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
    return compatibilityCommands.thinclawExportSession(sessionKey, format);
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
    return compatibilityCommands.thinclawHooksList();
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
    return compatibilityCommands.thinclawHooksRegister({ bundle_json: bundleJson, source: source || null });
}

/** Unregister (remove) a hook by name. */
export async function unregisterHook(hookName: string): Promise<HookUnregisterResponse> {
    return compatibilityCommands.thinclawHooksUnregister(hookName);
}

export interface ExtensionInfoItem {
    name: string;
    kind: string;
    description: string | null;
    url: string | null;
    active: boolean;
    authenticated: boolean;
    auth_mode: string;
    auth_status: string;
    tools: string[];
    needs_setup: boolean;
    shared_auth_provider: string | null;
    missing_scopes: string[];
    activation_status: string | null;
    activation_error: string | null;
    channel_diagnostics: any | null;
    reconnect_supported: boolean;
    setup: any;
}

export interface ExtensionsListResponse {
    extensions: ExtensionInfoItem[];
    total: number;
}

export interface ExtensionActionResponse {
    ok: boolean;
    message: string | null;
    auth_url?: string | null;
    setup_url?: string | null;
    auth_mode?: string | null;
    auth_status?: string | null;
    awaiting_token?: boolean | null;
    instructions?: string | null;
    shared_auth_provider?: string | null;
    missing_scopes?: string[];
    activated?: boolean | null;
    needs_restart?: boolean | null;
}

/** List all installed extensions/plugins. */
export async function listExtensions(): Promise<ExtensionsListResponse> {
    return compatibilityCommands.thinclawExtensionsList();
}

export async function installExtension(name: string, url?: string | null, kind?: string | null): Promise<ExtensionActionResponse> {
    return compatibilityCommands.thinclawExtensionInstall(name, url ?? null, kind ?? null);
}

export async function searchExtensionRegistry(query?: string): Promise<{ entries: any[] }> {
    return compatibilityCommands.thinclawExtensionRegistrySearch(query ?? null);
}

/** Activate an extension by name. */
export async function activateExtension(name: string): Promise<ExtensionActionResponse> {
    return compatibilityCommands.thinclawExtensionActivate(name);
}

export async function reconnectExtension(name: string): Promise<ExtensionActionResponse> {
    return compatibilityCommands.thinclawExtensionReconnect(name);
}

export async function getExtensionSetup(name: string): Promise<any> {
    return compatibilityCommands.thinclawExtensionSetupGet(name);
}

export async function submitExtensionSetup(name: string, secrets: Record<string, string>): Promise<ExtensionActionResponse> {
    return compatibilityCommands.thinclawExtensionSetupSubmit(name, secrets);
}

export async function validateExtensionSetup(name: string): Promise<ExtensionActionResponse> {
    return compatibilityCommands.thinclawExtensionValidateSetup(name);
}

/** Remove an extension by name. */
export async function removeExtension(name: string): Promise<ExtensionActionResponse> {
    return compatibilityCommands.thinclawExtensionRemove(name);
}

export async function listMcpServers(): Promise<any> {
    return compatibilityCommands.thinclawMcpServers();
}

export async function getMcpServer(name: string): Promise<any> {
    return compatibilityCommands.thinclawMcpServer(name);
}

export async function listMcpServerTools(name: string): Promise<any> {
    return compatibilityCommands.thinclawMcpServerTools(name);
}

export async function listMcpServerResources(name: string): Promise<any> {
    return compatibilityCommands.thinclawMcpServerResources(name);
}

export async function readMcpResource(name: string, uri: string): Promise<any> {
    return compatibilityCommands.thinclawMcpReadResource(name, uri);
}

export async function listMcpResourceTemplates(name: string): Promise<any> {
    return compatibilityCommands.thinclawMcpResourceTemplates(name);
}

export async function listMcpServerPrompts(name: string): Promise<any> {
    return compatibilityCommands.thinclawMcpServerPrompts(name);
}

export async function getMcpPrompt(serverName: string, promptName: string, args?: any): Promise<any> {
    return compatibilityCommands.thinclawMcpGetPrompt(serverName, promptName, args ?? null);
}

export async function discoverMcpOauth(name: string): Promise<any> {
    return compatibilityCommands.thinclawMcpOauth(name);
}

export async function setMcpLogLevel(name: string, level: string): Promise<ExtensionActionResponse> {
    return compatibilityCommands.thinclawMcpSetLogLevel(name, level);
}

export async function listMcpInteractions(): Promise<any> {
    return compatibilityCommands.thinclawMcpInteractions();
}

export async function respondMcpInteraction(interactionId: string, action: string, response?: any, message?: string): Promise<ExtensionActionResponse> {
    return compatibilityCommands.thinclawMcpInteractionRespond(interactionId, action, response ?? null, message ?? null);
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

/** List all ThinClaw config settings. */
export async function listSettings(): Promise<SettingsListResponse> {
    return compatibilityCommands.thinclawConfigGet();
}

/** Set a single config setting. */
export async function setSetting(key: string, value: any): Promise<{ ok: boolean }> {
    return compatibilityCommands.thinclawConfigSet(key, value);
}

/** Bulk-update settings. */
export async function patchSettings(patch: Record<string, any>): Promise<{ ok: boolean }> {
    return compatibilityCommands.thinclawConfigPatch(patch);
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
    return compatibilityCommands.thinclawDiagnostics();
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
    return compatibilityCommands.thinclawToolsList();
}

/** Get the list of globally disabled tool names. */
export async function getDisabledTools(): Promise<string[]> {
    return compatibilityCommands.thinclawToolPolicyGet();
}

/** Overwrite the list of globally disabled tool names. */
export async function setDisabledTools(disabledTools: string[]): Promise<void> {
    return compatibilityCommands.thinclawToolPolicySet(disabledTools);
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
    return compatibilityCommands.thinclawPairingList(channel);
}

/** Approve a pairing code for a channel. */
export async function approvePairing(channel: string, code: string): Promise<{ ok: boolean }> {
    return compatibilityCommands.thinclawPairingApprove(channel, code);
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
    return compatibilityCommands.thinclawCompactSession(sessionKey);
}

// ----------------------------------------------------------------------------
// Filesystem checkpoints / rollback (TDO-103)
// ----------------------------------------------------------------------------

/** A shadow-git checkpoint snapshot of a project directory. */
export interface CheckpointEntry {
    commit_hash: string;
    timestamp: string;
    summary: string;
}

/** List filesystem checkpoints (newest first) for a project directory. */
export async function listCheckpoints(projectDir: string): Promise<CheckpointEntry[]> {
    return compatibilityCommands.thinclawCheckpointsList(projectDir);
}

/** Unified diff of the current project state vs a checkpoint commit. */
export async function diffCheckpoint(projectDir: string, commitHash: string): Promise<string> {
    return compatibilityCommands.thinclawCheckpointDiff(projectDir, commitHash);
}

/** Restore a project (or a single file) to a checkpoint commit. */
export async function restoreCheckpoint(
    projectDir: string,
    commitHash: string,
    file?: string | null,
): Promise<void> {
    return compatibilityCommands.thinclawCheckpointRestore(projectDir, commitHash, file ?? null);
}

// ----------------------------------------------------------------------------
// Trajectory viewer (TDO-106)
// ----------------------------------------------------------------------------

/** Aggregate stats over the local trajectory archive. */
export interface TrajectoryStats {
    log_root: string;
    file_count: number;
    record_count: number;
    session_count: number;
    first_seen: string | null;
    last_seen: string | null;
    success_count: number;
    failure_count: number;
    neutral_count: number;
}

/** Aggregate stats (counts, span, outcomes) over the local trajectory archive. */
export async function getTrajectoryStats(): Promise<TrajectoryStats> {
    return compatibilityCommands.thinclawTrajectoryStats();
}

/** The most recent trajectory turn records (default 100) as raw JSON. */
export async function getTrajectoryRecords(limit?: number | null): Promise<ThinClawJson[]> {
    return compatibilityCommands.thinclawTrajectoryRecords(limit ?? null);
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
    return compatibilityCommands.thinclawCostSummary();
}

/** Export cost data as CSV string. */
export async function exportCostCsv(): Promise<string> {
    return compatibilityCommands.thinclawCostExportCsv();
}

/** Reset (clear) all cost tracking data. Persists empty state to DB. */
export async function resetCostData(): Promise<void> {
    return compatibilityCommands.thinclawCostReset();
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
    return compatibilityCommands.thinclawChannelStatusList();
}

// --- Agent Management ---

/** Set the default agent profile. */
export async function setDefaultAgent(agentId: string): Promise<void> {
    return compatibilityCommands.thinclawAgentsSetDefault(agentId);
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

/** Search ClawHub plugin catalog (proxied through ThinClaw). */
export async function searchClawHub(query: string): Promise<{ entries: ClawHubEntry[] }> {
    return compatibilityCommands.thinclawClawhubSearch(query);
}

/** Result of a ClawHub install request. In local/embedded mode this is
 *  prepare-only: the runtime stages the install and returns success=true with a
 *  "Ready to install ..." message (it does not download/activate). Remote mode
 *  proxies to the gateway install route. Callers should surface message/success
 *  rather than assuming the plugin is fully installed. */
export interface ClawHubInstallResult {
    plugin_name?: string;
    version?: string;
    install_path?: string;
    success?: boolean;
    message?: string;
}

/** Request a ClawHub plugin install. Returns the runtime's result (see
 *  {@link ClawHubInstallResult}) so callers can report the real outcome. */
export async function installFromClawHub(pluginId: string): Promise<ClawHubInstallResult> {
    return compatibilityCommands.thinclawClawhubInstall(pluginId);
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
    return compatibilityCommands.thinclawCacheStats();
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
    return compatibilityCommands.thinclawPluginLifecycleList();
}

// --- Manifest Validation ---

export interface ManifestValidation {
    errors: string[];
    warnings: string[];
}

/** Validate a plugin's manifest. */
export async function validateManifest(pluginId: string): Promise<ManifestValidation> {
    return compatibilityCommands.thinclawManifestValidate(pluginId);
}

// --- Smart Routing ---

/** Get current smart routing configuration. */
export async function getRoutingConfig(): Promise<{ smart_routing_enabled: boolean }> {
    return compatibilityCommands.thinclawRoutingGet();
}

/** Enable or disable smart routing. */
export async function setRoutingConfig(smartRoutingEnabled: boolean): Promise<void> {
    return compatibilityCommands.thinclawRoutingSet(smartRoutingEnabled);
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
    return compatibilityCommands.thinclawRoutingRulesList();
}

/** Save routing rules (full replace — ordered by priority). */
export async function saveRoutingRules(rules: RoutingRule[]): Promise<void> {
    return compatibilityCommands.thinclawRoutingRulesSave(rules);
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
 * Start the Gmail OAuth PKCE flow via ThinClaw.
 * Opens a browser for Google consent, waits for callback, exchanges for tokens.
 * Returns the full result — caller should check `success` field.
 */
export async function startGmailOAuth(): Promise<GmailOAuthResult> {
    return compatibilityCommands.thinclawGmailOauthStart();
}

// --- Routing Rule CRUD ---

/** Add a routing rule at position (or at the end). Returns updated rules list. */
export async function addRoutingRule(rule: RoutingRule, position?: number): Promise<RoutingRule[]> {
    return compatibilityCommands.thinclawRoutingRulesAdd(rule, position ?? null);
}

/** Remove a routing rule by index. Returns updated rules list. */
export async function removeRoutingRule(index: number): Promise<RoutingRule[]> {
    return compatibilityCommands.thinclawRoutingRulesRemove(index);
}

/** Reorder a routing rule (move from one position to another). Returns updated rules list. */
export async function reorderRoutingRule(from: number, to: number): Promise<RoutingRule[]> {
    return compatibilityCommands.thinclawRoutingRulesReorder(from, to);
}

/** Save explicit primary and cheap provider pool order. */
export async function saveRoutingPools(primaryPoolOrder: string[], cheapPoolOrder: string[]): Promise<void> {
    return compatibilityCommands.thinclawRoutingPoolsSave(primaryPoolOrder, cheapPoolOrder);
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
    routing_mode: string;
    primary_model: string | null;
    preferred_cheap_provider: string | null;
    cheap_model: string | null;
    primary_pool_order: string[];
    cheap_pool_order: string[];
    fallback_chain: string[];
    advisor_ready: boolean;
    advisor_disabled_reason: string | null;
    executor_target: string | null;
    advisor_target: string | null;
    diagnostics: string[];
    runtime_revision: number | null;
    llm_select_state: string;
    rule_count: number;
    rules: RoutingRuleSummary[];
    latency_data: LatencyEntry[];
}

/** Get full routing policy status including latency data. */
export async function getRoutingStatus(): Promise<RoutingStatusResponse> {
    return compatibilityCommands.thinclawRoutingStatus();
}

export interface RouteSimulationRequest {
    prompt: string;
    has_vision: boolean;
    has_tools: boolean;
    requires_streaming: boolean;
}

export interface RouteSimulationScore {
    target: string;
    telemetry_key: string | null;
    quality: number;
    cost: number;
    latency: number;
    health: number;
    policy_bias: number;
    composite: number;
}

export interface RouteSimulationResponse {
    target: string;
    reason: string;
    fallback_chain: string[];
    candidate_list: string[];
    rejections: string[];
    score_breakdown: RouteSimulationScore[];
    diagnostics: string[];
}

/** Simulate how ThinClaw will route a prompt without executing a model call. */
export async function simulateRouting(request: RouteSimulationRequest): Promise<RouteSimulationResponse> {
    return compatibilityCommands.thinclawRoutingSimulate(request);
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
    return compatibilityCommands.thinclawGmailStatus();
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
    return compatibilityCommands.thinclawCanvasPanelsList();
}

/** Get full data for a specific canvas panel. */
export async function getCanvasPanel(panelId: string): Promise<CanvasPanelData | null> {
    return compatibilityCommands.thinclawCanvasPanelGet(panelId);
}

/** Dismiss (remove) a canvas panel. */
export async function dismissCanvasPanel(panelId: string): Promise<boolean> {
    return compatibilityCommands.thinclawCanvasPanelDismiss(panelId);
}

// ============================================================================
// Routine Delete / Toggle
// ============================================================================

/** Delete a routine by ID or name. */
export async function deleteRoutine(routineId: string): Promise<{ ok: boolean; deleted_id: string }> {
    return compatibilityCommands.thinclawRoutineDelete(routineId);
}

/** Toggle a routine enabled/disabled. */
export async function toggleRoutine(routineId: string, enabled: boolean): Promise<{ ok: boolean; id: string; enabled: boolean }> {
    return compatibilityCommands.thinclawRoutineToggle(routineId, enabled);
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
    return compatibilityCommands.thinclawSetAutonomyMode(enabled);
}

/** Get the current autonomy mode setting. */
export async function getAutonomyMode(): Promise<boolean> {
    return compatibilityCommands.thinclawGetAutonomyMode();
}

// ============================================================================
// Jobs
// ============================================================================

export interface ThinClawJob {
    id: string;
    title: string;
    state: string;
    user_id?: string;
    created_at?: string;
    started_at?: string | null;
    execution_backend?: string | null;
    runtime_family?: string | null;
    runtime_mode?: string | null;
}

export interface ThinClawJobSummary {
    total: number;
    pending: number;
    in_progress: number;
    completed: number;
    failed: number;
    cancelled: number;
    interrupted: number;
    stuck: number;
}

export interface ThinClawJobTransition {
    from: string;
    to: string;
    timestamp: string;
    reason?: string | null;
}

export interface ThinClawJobDetail extends ThinClawJob {
    description?: string;
    completed_at?: string | null;
    elapsed_secs?: number | null;
    project_dir?: string | null;
    browse_url?: string | null;
    runtime_capabilities?: string[];
    network_isolation?: string | null;
    job_mode?: string | null;
    interactive?: boolean;
    transitions?: ThinClawJobTransition[];
}

export interface ThinClawJobEvent {
    id?: string;
    event_type: string;
    data?: unknown;
    created_at?: string;
}

export interface ThinClawJobFileEntry {
    name: string;
    path: string;
    is_dir: boolean;
}

export async function listJobs(): Promise<{ jobs: ThinClawJob[]; capabilities?: Record<string, boolean>; unavailable?: Record<string, string> }> {
    return compatibilityCommands.thinclawJobsList();
}

export async function getJobsSummary(): Promise<ThinClawJobSummary> {
    return compatibilityCommands.thinclawJobsSummary();
}

export async function getJobDetail(jobId: string): Promise<ThinClawJobDetail> {
    return compatibilityCommands.thinclawJobDetail(jobId);
}

export async function cancelJob(jobId: string): Promise<unknown> {
    return compatibilityCommands.thinclawJobCancel(jobId);
}

export async function restartJob(jobId: string): Promise<unknown> {
    return compatibilityCommands.thinclawJobRestart(jobId);
}

export async function promptJob(jobId: string, content: string | null, done = false): Promise<unknown> {
    return compatibilityCommands.thinclawJobPrompt(jobId, content, done);
}

export async function getJobEvents(jobId: string): Promise<{ job_id: string; events: ThinClawJobEvent[]; events_available?: boolean; unavailable_reason?: string | null }> {
    return compatibilityCommands.thinclawJobEvents(jobId);
}

export async function listJobFiles(jobId: string, path?: string): Promise<{ entries: ThinClawJobFileEntry[] }> {
    return compatibilityCommands.thinclawJobFilesList(jobId, path || null);
}

export async function readJobFile(jobId: string, path: string): Promise<{ path: string; content: string }> {
    return compatibilityCommands.thinclawJobFileRead(jobId, path);
}

// ============================================================================
// Desktop autonomy runtime
// ============================================================================

export interface AutonomyStatus {
    enabled: boolean;
    profile: string;
    deployment_mode: string;
    paused: boolean;
    pause_reason?: string | null;
    bootstrap_passed: boolean;
    emergency_stop_active: boolean;
    capture_evidence: boolean;
    kill_switch_hotkey: string;
    current_build_id?: string | null;
    last_bootstrap_at?: string | null;
    last_error?: string | null;
    code_auto_apply_paused: boolean;
    session_ready: boolean;
    action_ready: boolean;
    blocking_reason?: string | null;
    permission_summary?: unknown;
    prerequisite_summary?: unknown;
}

export interface AutonomyCheckResult {
    name: string;
    passed: boolean;
    detail?: string | null;
    evidence?: unknown;
}

export interface AutonomyRolloutSummary {
    current_build_id?: string | null;
    last_successful_build_id?: string | null;
    rollback_target_build_id?: string | null;
    code_auto_apply_paused: boolean;
    pause_reason?: string | null;
    consecutive_failed_promotions: number;
    failed_canary_count: number;
    recent_builds: Array<{
        build_id: string;
        proposal_id: string;
        title: string;
        created_at: string;
        promoted: boolean;
        checks: AutonomyCheckResult[];
        metadata?: unknown;
    }>;
}

export interface AutonomyChecksSummary {
    bootstrap_checks: AutonomyCheckResult[];
    latest_canary_checks: AutonomyCheckResult[];
    permission_report?: unknown;
}

export interface AutonomyEvidenceSummary {
    latest_bootstrap_report?: unknown;
    latest_canary_report?: unknown;
    recent_events: Array<{ kind: string; message: string; timestamp?: string | null }>;
    seeded_routines: string[];
    seeded_skills: string[];
}

export async function getAutonomyStatus(): Promise<AutonomyStatus> {
    return compatibilityCommands.thinclawAutonomyStatus();
}

export async function bootstrapAutonomy(): Promise<unknown> {
    return compatibilityCommands.thinclawAutonomyBootstrap();
}

export async function pauseAutonomy(reason?: string): Promise<unknown> {
    return compatibilityCommands.thinclawAutonomyPause(reason || null);
}

export async function resumeAutonomy(): Promise<unknown> {
    return compatibilityCommands.thinclawAutonomyResume();
}

export async function getAutonomyPermissions(): Promise<unknown> {
    return compatibilityCommands.thinclawAutonomyPermissions();
}

export async function rollbackAutonomy(): Promise<unknown> {
    return compatibilityCommands.thinclawAutonomyRollback();
}

export async function getAutonomyRollouts(): Promise<AutonomyRolloutSummary> {
    return compatibilityCommands.thinclawAutonomyRollouts();
}

export async function getAutonomyChecks(): Promise<AutonomyChecksSummary> {
    return compatibilityCommands.thinclawAutonomyChecks();
}

export async function getAutonomyEvidence(): Promise<AutonomyEvidenceSummary> {
    return compatibilityCommands.thinclawAutonomyEvidence();
}

// ============================================================================
// Bootstrap ritual
// ============================================================================

/**
 * Mark the first-run identity bootstrap ritual as completed.
 * Called by the frontend after the agent has finished naming itself.
 */
export async function setBootstrapCompleted(completed: boolean): Promise<void> {
    return compatibilityCommands.thinclawSetBootstrapCompleted(completed);
}

/**
 * Check whether the bootstrap ritual needs to run.
 * Returns true if the agent has NOT yet completed the identity ritual.
 */
export async function checkBootstrapNeeded(): Promise<boolean> {
    return compatibilityCommands.thinclawCheckBootstrapNeeded();
}

/**
 * Re-trigger the bootstrap ritual (Reinitiate Identity Ritual).
 * Resets bootstrap_completed so the modal shows on next startup.
 */
export async function triggerBootstrap(): Promise<void> {
    return compatibilityCommands.thinclawTriggerBootstrap();
}

// ── Workspace path & Finder reveal ────────────────────────────────────────────

/** Returns the local filesystem workspace root. */
export async function getWorkspacePath(): Promise<string> {
    return compatibilityCommands.thinclawGetWorkspacePath();
}

/** Opens the local workspace directory in Finder and returns the path. */
export async function revealWorkspace(): Promise<string> {
    return compatibilityCommands.thinclawRevealWorkspace();
}

export interface WorkspaceFile {
    path: string;
    absolute_path: string;
    size: number;
    modified_ms: number;
}

/** Lists all real files inside the agent_workspace filesystem directory. */
export async function listAgentWorkspaceFiles(): Promise<WorkspaceFile[]> {
    return compatibilityCommands.thinclawListAgentWorkspaceFiles();
}

/** Reveals a specific file in Finder (macOS) / Explorer (Windows). */
export async function revealFile(absolutePath: string): Promise<void> {
    return compatibilityCommands.thinclawRevealFile(absolutePath);
}

/**
 * Write content to a file in the agent's local `agent_workspace` directory.
 * Returns the absolute path of the written file.
 */
export async function writeAgentWorkspaceFile(relativePath: string, content: string): Promise<string> {
    return compatibilityCommands.thinclawWriteAgentWorkspaceFile(relativePath, content);
}

/**
 * Update the heartbeat interval (in minutes) at runtime.
 * Takes effect immediately — updates the DB routine schedule and persists to config.
 */
export async function setHeartbeatInterval(intervalMinutes: number): Promise<any> {
    return compatibilityCommands.thinclawHeartbeatSetInterval(intervalMinutes);
}

// ── Experiments & learning review ───────────────────────────────────────────

export type ThinClawJson = null | boolean | number | string | ThinClawJson[] | { [key: string]: ThinClawJson };

export async function getLearningStatus(limit = 50): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningStatus(limit);
}

export async function getLearningHistory(limit = 50): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningHistory(limit);
}

export async function getLearningCandidates(limit = 50): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningCandidates(limit);
}

export async function getLearningArtifactVersions(limit = 50): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningArtifactVersions(limit);
}

export async function getLearningProviderHealth(): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningProviderHealth();
}

export async function getLearningCodeProposals(status: string | null = null, limit = 50): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningCodeProposals(status, limit);
}

export async function getLearningOutcomes(status: string | null = null, limit = 50): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningOutcomes(status, limit);
}

export async function getLearningRollbacks(limit = 50): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningRollbacks(limit);
}

export async function reviewLearningCodeProposal(proposalId: string, decision: 'approve' | 'reject', note?: string): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningReviewCodeProposal(proposalId, decision, note || null);
}

export async function reviewLearningOutcome(outcomeId: string, decision: 'confirm' | 'dismiss' | 'requeue', verdict?: 'positive' | 'neutral' | 'negative'): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningReviewOutcome(outcomeId, decision, verdict || null);
}

export async function recordLearningRollback(artifactType: string, artifactName: string, reason: string, artifactVersionId?: string): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningRecordRollback(artifactType, artifactName, artifactVersionId || null, reason);
}

export async function evaluateLearningOutcomes(): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawLearningEvaluateOutcomes();
}

export async function getExperimentProjects(): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsProjects();
}

export async function getExperimentCampaigns(): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsCampaigns();
}

export async function getExperimentRunners(): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsRunners();
}

export async function getExperimentTargets(): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsTargets();
}

export async function getExperimentTrials(campaignId: string): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsTrials(campaignId);
}

export async function getExperimentTrialArtifacts(trialId: string): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsTrialArtifacts(trialId);
}

export async function getExperimentModelUsage(limit = 100): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsModelUsage(limit);
}

export async function getExperimentOpportunities(limit = 100): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsOpportunities(limit);
}

export async function getExperimentGpuClouds(): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsGpuClouds();
}

export async function validateExperimentRunner(runnerId: string): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsValidateRunner(runnerId);
}

export async function runExperimentCampaignAction(campaignId: string, action: 'pause' | 'resume' | 'cancel' | 'promote' | 'reissue-lease'): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsCampaignAction(campaignId, action);
}

export async function validateExperimentGpuCloud(provider: string): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsGpuValidate(provider);
}

export async function launchExperimentGpuCloudTest(provider: string): Promise<ThinClawJson> {
    return compatibilityCommands.thinclawExperimentsGpuLaunchTest(provider);
}

// ============================================================================
// Repo Projects
// ============================================================================

export interface ThinClawFutureCommandUnavailable {
    available: false;
    command: string;
    reason: string;
}

export type ThinClawRepoProjectState =
    | 'setup_required'
    | 'ready'
    | 'queued'
    | 'running'
    | 'paused'
    | 'blocked'
    | 'merging'
    | 'completed'
    | 'failed'
    | 'cancelled'
    | 'archived'
    | 'error'
    | string;

export type ThinClawRepoProjectSetupKey =
    | 'feature_flag'
    | 'github_app'
    | 'docker_agents'
    | 'coding_backend'
    | 'credentials'
    | 'concurrency'
    | 'write_mode'
    | 'auto_merge_policy'
    | 'notifications'
    | string;

export interface ThinClawRepoProjectSetupItem {
    key: ThinClawRepoProjectSetupKey;
    label: string;
    state: 'complete' | 'pending' | 'blocked' | string;
    detail?: string | null;
}

export interface ThinClawRepoBacklogItem {
    id: string;
    title: string;
    priority: 'low' | 'medium' | 'high' | 'urgent' | string;
    state: 'queued' | 'running' | 'blocked' | 'done' | string;
    owner?: string | null;
    labels?: string[];
    created_at?: string | null;
    updated_at?: string | null;
}

export interface ThinClawRepoWorkerRun {
    id: string;
    backlog_id?: string | null;
    agent: string;
    state: 'queued' | 'running' | 'paused' | 'failed' | 'completed' | 'cancelled' | string;
    branch?: string | null;
    started_at?: string | null;
    updated_at?: string | null;
    duration_secs?: number | null;
    last_event?: string | null;
}

export interface ThinClawRepoPullRequest {
    id: string;
    title: string;
    number?: number | null;
    url?: string | null;
    branch?: string | null;
    state: 'draft' | 'open' | 'merged' | 'closed' | string;
    author?: string | null;
    updated_at?: string | null;
}

export interface ThinClawRepoCiCheck {
    id: string;
    name: string;
    state: 'queued' | 'running' | 'passed' | 'failed' | 'skipped' | string;
    url?: string | null;
    updated_at?: string | null;
}

export interface ThinClawRepoMergeGate {
    id: string;
    label: string;
    state: 'passed' | 'pending' | 'blocked' | 'failed' | string;
    required: boolean;
    detail?: string | null;
    updated_at?: string | null;
}

export interface ThinClawRepoProject {
    id: string;
    name: string;
    repo_url: string;
    default_branch: string;
    local_path?: string | null;
    description?: string | null;
    state: ThinClawRepoProjectState;
    active_runs: number;
    queued_items: number;
    open_prs: number;
    merge_gate_state: 'passed' | 'pending' | 'blocked' | 'failed' | string;
    feature_flag?: 'enabled' | 'disabled' | 'pending' | string;
    github_app: 'connected' | 'missing' | 'pending' | string;
    docker_agents: 'ready' | 'missing' | 'degraded' | string;
    credentials: 'ready' | 'missing' | 'partial' | string;
    coding_backend?: 'worker' | 'codex_code' | 'claude_code' | string | null;
    concurrency_limit: number;
    write_mode: ThinClawRepoWriteMode | string;
    auto_merge_policy: 'manual' | 'green_checks' | 'approved_only' | 'disabled' | string;
    notifications: 'enabled' | 'disabled' | 'partial' | string;
    updated_at?: string | null;
    setup_checklist?: ThinClawRepoProjectSetupItem[];
    backlog?: ThinClawRepoBacklogItem[];
    worker_runs?: ThinClawRepoWorkerRun[];
    pull_requests?: ThinClawRepoPullRequest[];
    ci_checks?: ThinClawRepoCiCheck[];
    merge_gates?: ThinClawRepoMergeGate[];
}

export interface ThinClawRepoProjectsListResponse {
    projects: ThinClawRepoProject[];
    unavailable?: ThinClawFutureCommandUnavailable;
}

export interface ThinClawRepoProjectResponse {
    project: ThinClawRepoProject | null;
    unavailable?: ThinClawFutureCommandUnavailable;
}

export type ThinClawRepoWriteMode =
    | 'read_only_clone'
    | 'fork_pr'
    | 'maintainer_branch_pr'
    | 'maintainer_auto_merge';

export interface ThinClawRepoProjectCreateInput {
    name: string;
    repo_url: string;
    default_branch?: string | null;
    local_path?: string | null;
    description?: string | null;
    write_mode?: ThinClawRepoWriteMode | null;
    fork_owner?: string | null;
    fork_repo?: string | null;
}

export interface ThinClawRepoBacklogEnqueueInput {
    title: string;
    description?: string | null;
    priority?: 'low' | 'medium' | 'high' | 'urgent' | string;
    labels?: string[];
}

export interface ThinClawRepoApprovalInput {
    approval_id: string;
    decision: 'approve' | 'reject';
    note?: string | null;
}

export interface ThinClawRepoProjectCommandResponse {
    ok: boolean;
    message: string | null;
    project?: ThinClawRepoProject | null;
    run?: ThinClawRepoWorkerRun | null;
    unavailable?: ThinClawFutureCommandUnavailable;
}

export interface ThinClawRepoProjectEventsResponse {
    events: ThinClawJobEvent[];
    unavailable?: ThinClawFutureCommandUnavailable;
}

export interface ThinClawRepoProjectMergeGatesResponse {
    gates: ThinClawRepoMergeGate[];
    unavailable?: ThinClawFutureCommandUnavailable;
}

// ── Setup / credentials / GitHub connector ──────────────────────────────

export interface ThinClawRepoProjectsConfigureInput {
    enabled?: boolean | null;
    app_id?: number | null;
    installation_id?: number | null;
    private_key_secret?: string | null;
    webhook_secret_secret?: string | null;
    app_slug?: string | null;
    default_coding_backend?: string | null;
    default_write_mode?: ThinClawRepoWriteMode | string | null;
    auto_merge_default?: boolean | null;
    max_concurrent_projects?: number | null;
    max_concurrent_tasks_per_project?: number | null;
    watchdog_interval_secs?: number | null;
    workspace_base_dir?: string | null;
}

export interface ThinClawRepoProjectsReadiness {
    enabled: boolean;
    credential_mode: 'github_app' | 'github_token' | 'none' | string;
    app_id?: number | null;
    installation_id?: number | null;
    private_key_secret?: string | null;
    webhook_secret_secret?: string | null;
    app_slug?: string | null;
    install_url?: string | null;
    auto_merge_default: boolean;
    default_coding_backend: string;
    default_write_mode: ThinClawRepoWriteMode | string;
    max_concurrent_projects: number;
    max_concurrent_tasks_per_project: number;
    watchdog_interval_secs: number;
    github_token_secret_present?: boolean | null;
    github_fork_token_secret_present?: boolean | null;
    private_key_secret_present?: boolean | null;
    webhook_secret_present?: boolean | null;
    ready_for_live_runs: boolean;
    checklist: ThinClawRepoProjectSetupItem[];
    unavailable?: ThinClawFutureCommandUnavailable;
}

export interface ThinClawRepoCredentialStored {
    ok: boolean;
    name: string;
    unavailable?: ThinClawFutureCommandUnavailable;
}

export interface ThinClawConnectableRepo {
    owner: string;
    repo: string;
    full_name: string;
    private: boolean;
    archived: boolean;
    default_branch: string;
    html_url?: string | null;
    permissions: {
        pull: boolean;
        triage: boolean;
        push: boolean;
        maintain: boolean;
        admin: boolean;
    };
    recommended_write_mode: ThinClawRepoWriteMode | string;
    enrolled: boolean;
    project_id?: string | null;
}

export interface ThinClawConnectableReposResponse {
    source: 'github_app' | 'github_token' | 'gh_cli' | string;
    authenticated_user?: string | null;
    total: number;
    repos: ThinClawConnectableRepo[];
    unavailable?: ThinClawFutureCommandUnavailable;
}

export interface ThinClawRepoConnectInput {
    repos?: string[];
    all?: boolean;
    write_mode?: ThinClawRepoWriteMode | null;
    fork_owner?: string | null;
    fork_repo?: string | null;
}

export interface ThinClawRepoConnectResponse {
    ok: boolean;
    connected: string[];
    skipped: string[];
    message: string;
    unavailable?: ThinClawFutureCommandUnavailable;
}

export interface ThinClawRepoEnrollInput {
    repo_url: string;
    default_branch?: string | null;
    fork_owner?: string | null;
    fork_repo?: string | null;
}

function futureCommandUnavailable(command: string, err: unknown): ThinClawFutureCommandUnavailable {
    const reason = err instanceof Error ? err.message : String(err || `Command ${command} is not available yet.`);
    return { available: false, command, reason };
}

async function safeFutureCommand<T>(
    command: string,
    call: () => Promise<T>,
    fallback: (unavailable: ThinClawFutureCommandUnavailable) => T,
): Promise<T> {
    try {
        return await call();
    } catch (err) {
        return fallback(futureCommandUnavailable(command, err));
    }
}

export async function listRepoProjects(): Promise<ThinClawRepoProjectsListResponse> {
    return safeFutureCommand('thinclaw_repo_projects_list', () => compatibilityCommands.thinclawRepoProjectsList(), (unavailable) => ({
        projects: [],
        unavailable,
    }));
}

export async function getRepoProject(projectId: string): Promise<ThinClawRepoProjectResponse> {
    return safeFutureCommand('thinclaw_repo_project_get', () => compatibilityCommands.thinclawRepoProjectGet(projectId), (unavailable) => ({
        project: null,
        unavailable,
    }));
}

export async function createRepoProject(input: ThinClawRepoProjectCreateInput): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_create', () => compatibilityCommands.thinclawRepoProjectCreate(jsonValue(input)), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}

export async function startRepoProject(projectId: string): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_start', () => compatibilityCommands.thinclawRepoProjectStart(projectId), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}

export async function planRepoProject(projectId: string): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_plan', () => compatibilityCommands.thinclawRepoProjectPlan(projectId), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}

export async function pauseRepoProject(projectId: string): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_pause', () => compatibilityCommands.thinclawRepoProjectPause(projectId), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}

export async function resumeRepoProject(projectId: string): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_resume', () => compatibilityCommands.thinclawRepoProjectResume(projectId), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}

export async function cancelRepoProject(projectId: string, runId?: string): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_cancel', () => compatibilityCommands.thinclawRepoProjectCancel(projectId, runId ?? null), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}

export async function approveRepoProject(
    projectId: string,
    input: ThinClawRepoApprovalInput,
): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_approve', () => compatibilityCommands.thinclawRepoProjectApprove(projectId, jsonValue(input)), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}

export async function enqueueRepoProject(
    projectId: string,
    item: ThinClawRepoBacklogEnqueueInput,
): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_enqueue', () => compatibilityCommands.thinclawRepoProjectEnqueue(projectId, jsonValue(item)), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}

export async function getRepoProjectEvents(projectId: string, limit = 100): Promise<ThinClawRepoProjectEventsResponse> {
    return safeFutureCommand('thinclaw_repo_project_events', () => compatibilityCommands.thinclawRepoProjectEvents(projectId, limit), (unavailable) => ({
        events: [],
        unavailable,
    }));
}

export async function getRepoProjectMergeGates(projectId: string): Promise<ThinClawRepoProjectMergeGatesResponse> {
    return safeFutureCommand('thinclaw_repo_project_merge_gates', () => compatibilityCommands.thinclawRepoProjectMergeGates(projectId), (unavailable) => ({
        gates: [],
        unavailable,
    }));
}

// ── Setup / credentials / GitHub connector ──────────────────────────────

export async function getRepoProjectsReadiness(): Promise<ThinClawRepoProjectsReadiness> {
    return safeFutureCommand('thinclaw_repo_projects_readiness', () => compatibilityCommands.thinclawRepoProjectsReadiness(), (unavailable) => ({
        enabled: false,
        credential_mode: 'none',
        auto_merge_default: false,
        default_coding_backend: 'worker',
        default_write_mode: 'fork_pr',
        max_concurrent_projects: 1,
        max_concurrent_tasks_per_project: 1,
        watchdog_interval_secs: 60,
        ready_for_live_runs: false,
        checklist: [],
        unavailable,
    }));
}

export async function setupRepoProjects(
    input: ThinClawRepoProjectsConfigureInput,
): Promise<ThinClawRepoProjectsReadiness> {
    return safeFutureCommand('thinclaw_repo_projects_setup', () => compatibilityCommands.thinclawRepoProjectsSetup(jsonValue(input)), (unavailable) => ({
        enabled: false,
        credential_mode: 'none',
        auto_merge_default: false,
        default_coding_backend: 'worker',
        default_write_mode: 'fork_pr',
        max_concurrent_projects: 1,
        max_concurrent_tasks_per_project: 1,
        watchdog_interval_secs: 60,
        ready_for_live_runs: false,
        checklist: [],
        unavailable,
    }));
}

/**
 * Securely store a GitHub credential. The value is passed straight to the
 * encrypted secrets store; it is never written to settings, events, or logs.
 */
export async function setRepoProjectCredential(
    name: string,
    valueSecret: string,
): Promise<ThinClawRepoCredentialStored> {
    return safeFutureCommand(
        'thinclaw_repo_projects_set_credential',
        () => compatibilityCommands.thinclawRepoProjectsSetCredential(name, valueSecret),
        (unavailable) => ({ ok: false, name, unavailable }),
    );
}

export async function listConnectableRepos(): Promise<ThinClawConnectableReposResponse> {
    return safeFutureCommand('thinclaw_repo_projects_connectable_repos', () => compatibilityCommands.thinclawRepoProjectsConnectableRepos(), (unavailable) => ({
        source: 'none',
        total: 0,
        repos: [],
        unavailable,
    }));
}

export async function connectRepoProjects(
    input: ThinClawRepoConnectInput,
): Promise<ThinClawRepoConnectResponse> {
    return safeFutureCommand('thinclaw_repo_projects_connect', () => compatibilityCommands.thinclawRepoProjectsConnect(jsonValue(input)), (unavailable) => ({
        ok: false,
        connected: [],
        skipped: [],
        message: unavailable.reason,
        unavailable,
    }));
}

export async function enrollRepoProject(
    projectId: string,
    input: ThinClawRepoEnrollInput,
): Promise<ThinClawRepoProjectCommandResponse> {
    return safeFutureCommand('thinclaw_repo_project_enroll', () => compatibilityCommands.thinclawRepoProjectEnroll(projectId, jsonValue(input)), (unavailable) => ({
        ok: false,
        message: unavailable.reason,
        unavailable,
    }));
}
