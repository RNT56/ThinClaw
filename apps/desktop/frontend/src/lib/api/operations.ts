/**
 * ThinClaw operations and observability compatibility API.
+ *
+ * Component-friendly names delegate to the generated command client.
+ */

import { compatibilityCommands } from '../command-client';
import type { ThinClawJson } from './shared';

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

export type TrajectoryExportFormat = 'sft' | 'dpo';

export interface TrajectoryExport {
    format: TrajectoryExportFormat;
    payload: string;
    source_record_count: number;
    exported_record_count: number;
    skipped_counts: Record<string, number>;
}

/** Render a bounded local training export for an explicit frontend download. */
export async function exportTrajectory(format: TrajectoryExportFormat): Promise<TrajectoryExport> {
    return compatibilityCommands.thinclawTrajectoryExport(format) as Promise<TrajectoryExport>;
}

export interface ProfileEvolutionStatus {
    profile_path: string;
    profile_exists: boolean;
    profile_parse_error: string | null;
    preferred_name: string | null;
    confidence: number | null;
    message_count: number | null;
    profile_updated_at: string | null;
    profile: ThinClawJson | null;
    routine_exists: boolean;
    routine_id: string | null;
    routine_enabled: boolean;
    last_run_at: string | null;
    next_fire_at: string | null;
    run_count: number;
    consecutive_failures: number;
}

export async function getProfileEvolutionStatus(): Promise<ProfileEvolutionStatus> {
    return compatibilityCommands.thinclawProfileEvolutionStatus();
}

export async function runProfileEvolution(): Promise<{ routine_id: string; run_id: string }> {
    return compatibilityCommands.thinclawProfileEvolutionRun();
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
 * Returns non-secret completion metadata — caller should check `success`.
 * OAuth credentials remain in the encrypted desktop secret store.
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
