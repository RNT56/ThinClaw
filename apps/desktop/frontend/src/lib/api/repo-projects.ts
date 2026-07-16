/**
 * ThinClaw repository projects compatibility API.
+ *
+ * Component-friendly names delegate to the generated command client.
+ */

import { compatibilityCommands } from '../command-client';
import { jsonValue } from './shared';
import type { ThinClawJobEvent } from './operations';

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
