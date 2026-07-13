/**
 * ThinClaw integrations and configuration compatibility API.
+ *
+ * Component-friendly names delegate to the generated command client.
+ */

import { compatibilityCommands } from '../command-client';
import type { ThinClawJson } from './shared';

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
