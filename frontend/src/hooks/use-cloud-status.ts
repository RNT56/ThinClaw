/**
 * A5-8: useCloudStatus hook — polls cloud_get_status + listens to cloud_migration_progress events.
 */
import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// ── Types (mirroring Rust cloud::commands types) ─────────────────────────

export interface CloudStatusResponse {
    mode: string;
    provider_connected: boolean;
    provider_name: string | null;
    storage_used: number;
    storage_available: number | null;
    last_sync_at: number | null;
    has_recovery_key: boolean;
    migration_in_progress: boolean;
}

export interface StorageCategory {
    id: string;
    label: string;
    size_bytes: number;
}

export interface ConnectionTestResult {
    connected: boolean;
    provider_name: string;
    storage_used: number;
    storage_available: number | null;
    error: string | null;
}

export type MigrationPhase =
    | 'preflight'
    | 'database_snapshot'
    | 'encrypting_files'
    | 'uploading_database'
    | 'uploading_documents'
    | 'uploading_images'
    | 'uploading_generated_images'
    | 'uploading_vectors'
    | 'uploading_agent_state'
    | 'uploading_manifest'
    | 'verification'
    | 'cleanup'
    | 'complete'
    | 'downloading_manifest'
    | 'downloading_files'
    | 'restoring_database'
    | 'rebuilding_vectors';

export interface MigrationProgress {
    migration_id: string;
    direction: string;
    phase: MigrationPhase;
    overall_percent: number;
    phase_percent: number;
    files_done: number;
    files_total: number;
    bytes_done: number;
    bytes_total: number;
    speed_bps: number;
    eta_seconds: number | null;
    message: string;
    complete: boolean;
    error: string | null;
}

export interface S3ConfigInput {
    endpoint: string | null;
    bucket: string;
    region: string | null;
    access_key_id: string;
    secret_access_key: string;
    root: string | null;
}

// ── Phase labels (for UI checklist) ──────────────────────────────────────

export const PHASE_LABELS: Record<string, string> = {
    preflight: 'Pre-flight checks',
    database_snapshot: 'Database snapshot',
    encrypting_files: 'Encrypting files',
    uploading_database: 'Uploading database',
    uploading_documents: 'Uploading documents',
    uploading_images: 'Uploading images',
    uploading_generated_images: 'Uploading generated images',
    uploading_vectors: 'Uploading vector indices',
    uploading_agent_state: 'Uploading agent state',
    uploading_manifest: 'Uploading manifest',
    verification: 'Verifying archive',
    cleanup: 'Cleaning up',
    complete: 'Complete',
    downloading_manifest: 'Downloading manifest',
    downloading_files: 'Downloading files',
    restoring_database: 'Restoring database',
    rebuilding_vectors: 'Rebuilding vector indices',
};

export const UPLOAD_PHASES: MigrationPhase[] = [
    'preflight', 'database_snapshot', 'encrypting_files',
    'uploading_database', 'uploading_documents', 'uploading_images',
    'uploading_generated_images', 'uploading_vectors', 'uploading_agent_state',
    'uploading_manifest', 'verification', 'cleanup', 'complete',
];

export const DOWNLOAD_PHASES: MigrationPhase[] = [
    'preflight', 'downloading_manifest', 'downloading_files',
    'restoring_database', 'rebuilding_vectors', 'cleanup', 'complete',
];

// ── Hook ─────────────────────────────────────────────────────────────────

export function useCloudStatus() {
    const [status, setStatus] = useState<CloudStatusResponse | null>(null);
    const [breakdown, setBreakdown] = useState<StorageCategory[]>([]);
    const [migrationProgress, setMigrationProgress] = useState<MigrationProgress | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    const refresh = useCallback(async () => {
        try {
            const s = await invoke<CloudStatusResponse>('cloud_get_status');
            setStatus(s);
            setError(null);
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    }, []);

    const refreshBreakdown = useCallback(async () => {
        try {
            const b = await invoke<StorageCategory[]>('cloud_get_storage_breakdown');
            setBreakdown(b);
        } catch (e) {
            console.error('[useCloudStatus] Failed to fetch breakdown:', e);
        }
    }, []);

    // Initial load + polling
    useEffect(() => {
        refresh();
        refreshBreakdown();
        const interval = setInterval(refresh, 10_000); // poll every 10s
        return () => clearInterval(interval);
    }, [refresh, refreshBreakdown]);

    // Listen for migration progress events
    useEffect(() => {
        let unlisten: UnlistenFn | null = null;
        (async () => {
            unlisten = await listen<MigrationProgress>('cloud_migration_progress', (event) => {
                setMigrationProgress(event.payload);
                // Refresh status when migration completes or fails
                if (event.payload.complete || event.payload.error) {
                    setTimeout(() => {
                        refresh();
                        refreshBreakdown();
                    }, 500);
                }
            });
        })();
        return () => { unlisten?.(); };
    }, [refresh, refreshBreakdown]);

    const isCloud = status?.mode?.startsWith('cloud:') ?? false;
    const isLocal = !isCloud;
    const isMigrating = status?.migration_in_progress || (migrationProgress != null && !migrationProgress.complete && !migrationProgress.error);
    const totalSize = breakdown.reduce((acc, c) => acc + c.size_bytes, 0);

    return {
        status,
        breakdown,
        totalSize,
        migrationProgress,
        loading,
        error,
        isCloud,
        isLocal,
        isMigrating,
        refresh,
        refreshBreakdown,
        setMigrationProgress,
    };
}
