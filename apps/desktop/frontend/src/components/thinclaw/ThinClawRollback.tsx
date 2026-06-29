import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import { GitBranch, RefreshCw, RotateCcw, Clock, History, AlertTriangle } from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';

export function ThinClawRollback() {
    const [projectDir, setProjectDir] = useState('');
    const [entries, setEntries] = useState<thinclaw.CheckpointEntry[]>([]);
    const [selected, setSelected] = useState<string | null>(null);
    const [diff, setDiff] = useState('');
    const [isLoading, setIsLoading] = useState(true);
    const [notice, setNotice] = useState<string | null>(null);
    const [pendingRestore, setPendingRestore] = useState<string | null>(null);

    const load = useCallback(async (dir?: string) => {
        setIsLoading(true);
        setNotice(null);
        try {
            const path = dir ?? projectDir ?? (await thinclaw.getWorkspacePath());
            setProjectDir(path);
            const list = await thinclaw.listCheckpoints(path);
            setEntries(list);
        } catch (e: any) {
            const msg = String(e?.message ?? e);
            // Disabled-by-default and empty repos surface as a friendly notice, not an error.
            setNotice(
                msg.includes('disabled')
                    ? 'Filesystem checkpoints are disabled. Enable `checkpoints_enabled` in the agent config to record reversible snapshots before file edits.'
                    : msg,
            );
            setEntries([]);
        } finally {
            setIsLoading(false);
        }
    }, [projectDir]);

    useEffect(() => {
        (async () => {
            try {
                const path = await thinclaw.getWorkspacePath();
                await load(path);
            } catch {
                await load('');
            }
        })();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    const showDiff = async (hash: string) => {
        setSelected(hash);
        setDiff('');
        try {
            setDiff(await thinclaw.diffCheckpoint(projectDir, hash));
        } catch (e: any) {
            setDiff(`Failed to load diff: ${String(e?.message ?? e)}`);
        }
    };

    const restore = async (hash: string) => {
        if (pendingRestore !== hash) {
            setPendingRestore(hash);
            setTimeout(() => setPendingRestore((p) => (p === hash ? null : p)), 4000);
            return;
        }
        setPendingRestore(null);
        const tId = toast.loading('Restoring checkpoint…');
        try {
            await thinclaw.restoreCheckpoint(projectDir, hash);
            toast.success('Project restored to checkpoint', { id: tId });
            load();
        } catch (e: any) {
            toast.error(`Restore failed: ${String(e?.message ?? e)}`, { id: tId });
        }
    };

    return (
        <motion.div className="flex-1 overflow-y-auto p-8 space-y-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-cyan-500/10 border border-cyan-500/20">
                        <History className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Checkpoints &amp; Rollback</h1>
                        <p className="text-xs text-muted-foreground truncate max-w-[40ch]">
                            Shadow-git snapshots taken before file edits · {projectDir || '—'}
                        </p>
                    </div>
                </div>
                <button
                    onClick={() => load()}
                    className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                >
                    <RefreshCw className={cn('w-3.5 h-3.5', isLoading && 'animate-spin')} />
                </button>
            </div>

            {notice && (
                <div className="rounded-xl border border-amber-500/20 bg-amber-500/5 px-4 py-3 flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-400 mt-0.5 shrink-0" />
                    <p className="text-xs text-amber-200/90">{notice}</p>
                </div>
            )}

            {!notice && (
                <div className="grid grid-cols-1 lg:grid-cols-5 gap-4">
                    {/* Checkpoint list */}
                    <div className="lg:col-span-2 space-y-2">
                        {entries.length === 0 && !isLoading ? (
                            <p className="text-xs text-muted-foreground">No checkpoints recorded for this project yet.</p>
                        ) : (
                            entries.map((e) => (
                                <div
                                    key={e.commit_hash}
                                    onClick={() => showDiff(e.commit_hash)}
                                    className={cn(
                                        'rounded-xl border px-3 py-2.5 cursor-pointer transition-all',
                                        selected === e.commit_hash
                                            ? 'border-primary/40 bg-primary/5'
                                            : 'border-white/5 bg-white/[0.02] hover:bg-white/[0.04]',
                                    )}
                                >
                                    <div className="flex items-center gap-2">
                                        <GitBranch className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
                                        <span className="text-xs font-mono text-foreground/90">{e.commit_hash.slice(0, 8)}</span>
                                    </div>
                                    <p className="text-xs text-foreground/80 mt-1 line-clamp-2">{e.summary || '(no summary)'}</p>
                                    <div className="flex items-center justify-between mt-1.5">
                                        <span className="flex items-center gap-1 text-[9px] text-muted-foreground">
                                            <Clock className="w-2.5 h-2.5" />{new Date(e.timestamp).toLocaleString()}
                                        </span>
                                        <button
                                            onClick={(ev) => { ev.stopPropagation(); restore(e.commit_hash); }}
                                            className={cn(
                                                'inline-flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium border transition-all',
                                                pendingRestore === e.commit_hash
                                                    ? 'bg-red-500/15 text-red-300 border-red-500/30'
                                                    : 'bg-white/[0.03] text-muted-foreground hover:text-foreground border-white/5',
                                            )}
                                        >
                                            <RotateCcw className="w-2.5 h-2.5" />
                                            {pendingRestore === e.commit_hash ? 'Confirm restore' : 'Restore'}
                                        </button>
                                    </div>
                                </div>
                            ))
                        )}
                    </div>

                    {/* Diff viewer */}
                    <div className="lg:col-span-3 rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-4 min-h-[12rem]">
                        {selected ? (
                            <pre className="text-[10px] leading-relaxed font-mono text-foreground/80 whitespace-pre-wrap break-words overflow-x-auto">
                                {diff || 'No changes vs current state.'}
                            </pre>
                        ) : (
                            <div className="h-full flex items-center justify-center text-xs text-muted-foreground">
                                Select a checkpoint to view its diff against the current project state.
                            </div>
                        )}
                    </div>
                </div>
            )}
        </motion.div>
    );
}
