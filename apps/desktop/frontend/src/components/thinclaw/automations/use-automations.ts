import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';

import * as thinclaw from '../../../lib/thinclaw';
import { useThinClawEvents } from '../../../hooks/use-thinclaw-stream';
import { useThinClawStatusSnapshot } from '../ThinClawModeBadge';

export function useAutomations() {
    const [jobs, setJobs] = useState<thinclaw.CronJob[]>([]);
    const [historyJob, setHistoryJob] = useState<string | null>(null);
    const [history, setHistory] = useState<any[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [showCreateModal, setShowCreateModal] = useState(false);
    const { status: runtimeStatus } = useThinClawStatusSnapshot(15000);

    // Cron lint state
    const [cronExpr, setCronExpr] = useState('');
    const [lintResult, setLintResult] = useState<thinclaw.CronLintResult | null>(null);
    const [lintError, setLintError] = useState<string | null>(null);
    const [isLinting, setIsLinting] = useState(false);

    const fetchData = async () => {
        try {
            const data = await thinclaw.getThinClawCronList();
            setJobs(Array.isArray(data) ? data : []);
        } catch (e) {
            console.error('Failed to fetch cron jobs:', e);
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchData();
        const interval = setInterval(fetchData, 30000);
        return () => clearInterval(interval);
    }, []);

    // Listen for routine lifecycle events from backend SSE forwarder
    useThinClawEvents((payload) => {
        if (payload.kind === 'RoutineLifecycle') {
            const { routine_name, event: evType, result_summary } = payload;
            const snippet = result_summary ? `: ${String(result_summary).slice(0, 80)}` : '';
            if (evType === 'started') {
                toast.info(`⏱ "${routine_name}" started`, { duration: 4000 });
            } else if (evType === 'dispatched') {
                // full_job was queued — worker is running, real result comes later
                toast.info(`🔄 "${routine_name}" queued — worker executing`, { duration: 5000 });
            } else if (evType === 'completed') {
                toast.success(`✅ "${routine_name}" completed${snippet}`, { duration: 6000 });
                fetchData();
            } else if (evType === 'failed') {
                toast.error(`❌ "${routine_name}" failed${snippet}`, { duration: 8000 });
                fetchData();
            }
        }
    });

    const handleRun = async (key: string) => {
        try {
            toast.promise(thinclaw.runThinClawCron(key), {
                loading: `Triggering routine...`,
                success: `Routine triggered — watch output below`,
                error: (err) => `Failed to run: ${err}`
            });
        } catch (_e) { }
    };

    const handleDelete = async (key: string, name: string) => {
        try {
            await thinclaw.deleteRoutine(key);
            setJobs(prev => prev.filter(j => j.key !== key));
            toast.success(`Routine "${name}" deleted`);
        } catch (e) {
            toast.error(`Failed to delete: ${String(e)}`);
        }
    };

    const handleToggle = async (key: string, enabled: boolean, name: string) => {
        try {
            await thinclaw.toggleRoutine(key, enabled);
            setJobs(prev => prev.map(job => job.key === key ? { ...job, enabled } : job));
            toast.success(`Routine "${name}" ${enabled ? 'enabled' : 'disabled'}`);
            fetchData();
        } catch (e) {
            toast.error(`Failed to update routine: ${String(e)}`);
        }
    };

    const handleViewHistory = async (key: string) => {
        setHistoryJob(key);
        setHistory([]);
        try {
            const data = await thinclaw.getRoutineAuditList(key, 10);
            setHistory(Array.isArray(data) ? data : []);
        } catch (_e) {
            toast.error(`Failed to fetch history for ${key}`);
        }
    };

    const handleLintCron = useCallback(async () => {
        if (!cronExpr.trim()) return;
        setIsLinting(true);
        setLintError(null);
        setLintResult(null);
        try {
            const result = await thinclaw.lintCronExpression(cronExpr.trim());
            setLintResult(result);
        } catch (e) {
            setLintError(String(e));
        } finally {
            setIsLinting(false);
        }
    }, [cronExpr]);

    return {
        jobs, historyJob, setHistoryJob, history, isLoading, setIsLoading, showCreateModal,
        setShowCreateModal, runtimeStatus, cronExpr, setCronExpr, lintResult, lintError,
        isLinting, fetchData, handleRun, handleDelete, handleToggle, handleViewHistory,
        handleLintCron,
    };
}
