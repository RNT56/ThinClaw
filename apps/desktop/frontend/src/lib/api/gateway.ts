/**
 * ThinClaw gateway and session compatibility API.
+ *
+ * Component-friendly names delegate to the generated command client.
+ */

import { compatibilityCommands } from '../command-client';
import type {
    AgentProfile,
    CronHistoryItem,
    CronJob,
    Skill,
    ThinClawRpcResponse,
    ThinClawSkillsStatus,
} from './core';

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

export type RoutineTriggerType = 'cron' | 'system_event';

/** Create a scheduled agent job or heartbeat system event. */
export async function createRoutine(
    name: string,
    description: string,
    schedule: string,
    task: string,
    triggerType: RoutineTriggerType = 'cron',
): Promise<CreateRoutineResult> {
    return compatibilityCommands.thinclawRoutineCreate(
        name,
        description,
        schedule,
        task,
        triggerType,
    );
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
