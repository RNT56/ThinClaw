/**
 * ThinClaw compatibility API
 *
 * Existing component-friendly names delegate to the generated binding client.
 * Command names, parameters, and result data are owned by bindings.ts.
 */

import { compatibilityCommands } from '../command-client';
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
    /** Always null in status responses; retained for wire compatibility. */
    remote_token: string | null;
    has_remote_token: boolean;
    device_id: string;
    /** Always empty in status responses; retained for wire compatibility. */
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
    /** Always null in status responses; retained for wire compatibility. */
    custom_llm_key: string | null;
    has_custom_llm_key: boolean;
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

/** Reveal the local gateway token only for an explicit user copy action. */
export async function revealGatewayToken(): Promise<string> {
    return compatibilityCommands.thinclawRevealGatewayToken();
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
