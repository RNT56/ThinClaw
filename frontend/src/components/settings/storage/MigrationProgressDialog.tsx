/**
 * MigrationProgressDialog — modal showing real-time migration progress.
 */
import {
    CheckCircle2, XCircle, Loader2,
    Upload, Download, X
} from 'lucide-react';
import { motion } from 'framer-motion';
import { cn } from '../../../lib/utils';
import {
    type MigrationProgress,
    PHASE_LABELS,
    UPLOAD_PHASES,
    DOWNLOAD_PHASES,
} from '../../../hooks/use-cloud-status';

function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

function formatSpeed(bps: number): string {
    if (bps === 0) return '—';
    return formatBytes(bps) + '/s';
}

function formatEta(seconds: number | null): string {
    if (seconds == null || seconds <= 0) return '—';
    if (seconds < 60) return `${Math.ceil(seconds)}s`;
    return `${Math.ceil(seconds / 60)}m`;
}

export function MigrationProgressDialog({
    progress,
    onCancel,
    cancelling,
    onClose,
}: {
    progress: MigrationProgress;
    onCancel: () => void;
    cancelling: boolean;
    onClose: () => void;
}) {
    const isUpload = progress.direction === 'to_cloud';
    const phases = isUpload ? UPLOAD_PHASES : DOWNLOAD_PHASES;
    const currentIdx = phases.indexOf(progress.phase);
    const isComplete = progress.complete;
    const hasError = !!progress.error;

    return (
        <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 backdrop-blur-sm"
            onClick={e => { if (e.target === e.currentTarget && (isComplete || hasError)) onClose(); }}
        >
            <motion.div
                initial={{ scale: 0.9, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                exit={{ scale: 0.9, opacity: 0 }}
                className="w-full max-w-lg bg-card rounded-2xl border border-border/50 shadow-2xl overflow-hidden"
            >
                {/* Header */}
                <div className={cn(
                    'px-6 py-4 flex items-center gap-3',
                    hasError ? 'bg-rose-500/10 border-b border-rose-500/20' :
                        isComplete ? 'bg-emerald-500/10 border-b border-emerald-500/20' :
                            'bg-primary/5 border-b border-border/30'
                )}>
                    <div className={cn(
                        'p-2.5 rounded-xl',
                        hasError ? 'bg-rose-500/20' : isComplete ? 'bg-emerald-500/20' : 'bg-primary/10'
                    )}>
                        {hasError ? <XCircle className="w-5 h-5 text-rose-500" /> :
                            isComplete ? <CheckCircle2 className="w-5 h-5 text-emerald-500" /> :
                                isUpload ? <Upload className="w-5 h-5 text-primary" /> :
                                    <Download className="w-5 h-5 text-primary" />}
                    </div>
                    <div className="flex-1">
                        <h2 className="font-bold text-lg">
                            {hasError ? 'Migration Failed' :
                                isComplete ? 'Migration Complete' :
                                    isUpload ? 'Migrating to Cloud…' : 'Migrating to Local…'}
                        </h2>
                        <p className="text-xs text-muted-foreground">{progress.message}</p>
                    </div>
                    {(isComplete || hasError) && (
                        <button onClick={onClose} className="p-1.5 hover:bg-muted rounded-lg transition-colors">
                            <X className="w-4 h-4 text-muted-foreground" />
                        </button>
                    )}
                </div>

                <div className="px-6 py-5 space-y-5">
                    {/* Progress bar */}
                    <div className="space-y-2">
                        <div className="flex justify-between text-xs">
                            <span className="font-medium text-muted-foreground">
                                {formatBytes(progress.bytes_done)} / {formatBytes(progress.bytes_total)}
                            </span>
                            <span className="font-bold text-foreground">{progress.overall_percent.toFixed(1)}%</span>
                        </div>
                        <div className="h-3 rounded-full bg-muted/30 overflow-hidden border border-border/30">
                            <motion.div
                                className={cn(
                                    'h-full rounded-full transition-colors',
                                    hasError ? 'bg-rose-500' :
                                        isComplete ? 'bg-emerald-500' :
                                            'bg-gradient-to-r from-primary to-primary/70'
                                )}
                                initial={{ width: 0 }}
                                animate={{ width: `${Math.min(progress.overall_percent, 100)}%` }}
                                transition={{ duration: 0.5, ease: 'easeOut' }}
                            />
                        </div>
                        {!isComplete && !hasError && (
                            <div className="flex items-center justify-between text-[10px] text-muted-foreground/60">
                                <span>{progress.files_done}/{progress.files_total} files</span>
                                <span>{formatSpeed(progress.speed_bps)}</span>
                                <span>ETA: {formatEta(progress.eta_seconds)}</span>
                            </div>
                        )}
                    </div>

                    {/* Phase checklist */}
                    <div className="space-y-1 max-h-[250px] overflow-y-auto pr-2 custom-scrollbar">
                        {phases.map((phase, i) => {
                            const isDone = i < currentIdx || isComplete;
                            const isCurrent = i === currentIdx && !isComplete && !hasError;
                            const label = PHASE_LABELS[phase] ?? phase;
                            return (
                                <div key={phase} className={cn(
                                    'flex items-center gap-2.5 px-3 py-1.5 rounded-lg text-sm transition-all',
                                    isDone ? 'text-muted-foreground/60' :
                                        isCurrent ? 'bg-primary/5 text-foreground font-medium' :
                                            'text-muted-foreground/30'
                                )}>
                                    {isDone ? <CheckCircle2 className="w-3.5 h-3.5 text-emerald-500 shrink-0" /> :
                                        isCurrent ? <Loader2 className="w-3.5 h-3.5 text-primary animate-spin shrink-0" /> :
                                            <div className="w-3.5 h-3.5 rounded-full border border-border/50 shrink-0" />}
                                    <span>{label}</span>
                                </div>
                            );
                        })}
                    </div>

                    {/* Error message */}
                    {hasError && (
                        <div className="p-3 rounded-xl bg-rose-500/10 border border-rose-500/20 text-sm text-rose-600 dark:text-rose-400">
                            <span className="font-bold">Error:</span> {progress.error}
                        </div>
                    )}

                    {/* Actions */}
                    <div className="flex justify-end gap-3 pt-2">
                        {!isComplete && !hasError && (
                            <button
                                onClick={onCancel}
                                disabled={cancelling}
                                className={cn(
                                    'px-5 h-9 rounded-xl border border-border/50 text-xs font-bold uppercase tracking-wider',
                                    'hover:bg-rose-500/10 hover:text-rose-600 hover:border-rose-500/30 transition-all',
                                    cancelling && 'opacity-50 cursor-wait'
                                )}
                            >
                                {cancelling ? <><Loader2 className="w-3.5 h-3.5 animate-spin inline mr-1.5" />Cancelling…</> : 'Cancel'}
                            </button>
                        )}
                        {(isComplete || hasError) && (
                            <button
                                onClick={onClose}
                                className="px-5 h-9 rounded-xl bg-primary text-primary-foreground text-xs font-bold uppercase tracking-wider hover:bg-primary/90 transition-all"
                            >
                                Done
                            </button>
                        )}
                    </div>
                </div>
            </motion.div>
        </motion.div>
    );
}
