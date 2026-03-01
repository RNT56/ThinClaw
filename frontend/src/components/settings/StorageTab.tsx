/**
 * A5-1: StorageTab — Cloud Storage settings page.
 * Includes: storage mode toggle, storage breakdown, provider picker,
 * S3 config form, migration progress dialog, recovery key panel.
 */
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
    Cloud, HardDrive, Shield, CheckCircle2, XCircle,
    Loader2, RefreshCw, AlertTriangle, Copy, Eye, EyeOff, Key,
    Upload, Download, Info, X, Wifi, Lock,
    Server, FolderOpen, Database, ImageIcon, FileText, Box, Cpu
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import { AnimatePresence, motion } from 'framer-motion';
import {
    useCloudStatus,
    type S3ConfigInput,
    type ConnectionTestResult,
    type MigrationProgress,
    PHASE_LABELS,
    UPLOAD_PHASES,
    DOWNLOAD_PHASES,
} from '../../hooks/use-cloud-status';

// ── Helpers ──────────────────────────────────────────────────────────────

function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.min(Math.floor(Math.log2(bytes) / 10), 4);
    return `${(bytes / Math.pow(1024, i)).toFixed(i > 1 ? 1 : 0)} ${units[i]}`;
}

function formatSpeed(bps: number): string {
    if (bps === 0) return '—';
    if (bps >= 1024 * 1024) return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
    if (bps >= 1024) return `${(bps / 1024).toFixed(0)} KB/s`;
    return `${bps} B/s`;
}

function formatEta(seconds: number | null): string {
    if (seconds == null || seconds <= 0) return '—';
    if (seconds < 60) return `${seconds}s`;
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
    return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

// ── Category icons + colors ──────────────────────────────────────────────

const CATEGORY_META: Record<string, { icon: React.ElementType; color: string }> = {
    database: { icon: Database, color: 'bg-violet-500' },
    documents: { icon: FileText, color: 'bg-blue-500' },
    images: { icon: ImageIcon, color: 'bg-emerald-500' },
    generated: { icon: ImageIcon, color: 'bg-teal-500' },
    vectors: { icon: Cpu, color: 'bg-amber-500' },
    previews: { icon: ImageIcon, color: 'bg-pink-500' },
    openclaw: { icon: Box, color: 'bg-rose-500' },
};

// ── A5-2: StorageBreakdown — Visual bar chart ────────────────────────────

function StorageBreakdown({
    breakdown,
    totalSize,
}: {
    breakdown: { id: string; label: string; size_bytes: number }[];
    totalSize: number;
}) {
    if (breakdown.length === 0) return null;

    // Sort by size desc
    const sorted = [...breakdown].sort((a, b) => b.size_bytes - a.size_bytes);

    return (
        <div className="space-y-4">
            <div className="flex items-center justify-between">
                <h3 className="text-sm font-bold uppercase tracking-wider text-muted-foreground/60">
                    Storage Breakdown
                </h3>
                <span className="text-sm font-bold text-foreground">{formatBytes(totalSize)} total</span>
            </div>

            {/* Stacked bar */}
            <div className="h-4 rounded-full overflow-hidden flex bg-muted/30 border border-border/30">
                {sorted.filter(c => c.size_bytes > 0).map(cat => {
                    const pct = totalSize > 0 ? (cat.size_bytes / totalSize) * 100 : 0;
                    const meta = CATEGORY_META[cat.id] ?? { color: 'bg-gray-500' };
                    return (
                        <div
                            key={cat.id}
                            className={cn(meta.color, 'transition-all duration-500 first:rounded-l-full last:rounded-r-full')}
                            style={{ width: `${Math.max(pct, 1)}%` }}
                            title={`${cat.label}: ${formatBytes(cat.size_bytes)} (${pct.toFixed(1)}%)`}
                        />
                    );
                })}
            </div>

            {/* Legend */}
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
                {sorted.filter(c => c.size_bytes > 0).map(cat => {
                    const meta = CATEGORY_META[cat.id] ?? { icon: FolderOpen, color: 'bg-gray-500' };
                    const Icon = meta.icon;
                    return (
                        <div key={cat.id} className="flex items-center gap-2 text-xs">
                            <div className={cn('w-2.5 h-2.5 rounded-sm shrink-0', meta.color)} />
                            <Icon className="w-3 h-3 text-muted-foreground shrink-0" />
                            <span className="text-muted-foreground truncate">{cat.label}</span>
                            <span className="ml-auto font-mono text-foreground font-medium">{formatBytes(cat.size_bytes)}</span>
                        </div>
                    );
                })}
            </div>
        </div>
    );
}

// ── A5-4: S3ConfigForm ───────────────────────────────────────────────────

function S3ConfigForm({
    onTestConnection,
    testing,
}: {
    onTestConnection: (config: S3ConfigInput) => Promise<void>;
    testing: boolean;
}) {
    const [endpoint, setEndpoint] = useState('');
    const [bucket, setBucket] = useState('');
    const [region, setRegion] = useState('auto');
    const [accessKey, setAccessKey] = useState('');
    const [secretKey, setSecretKey] = useState('');
    const [root, setRoot] = useState('scrappy-data');
    const [showSecret, setShowSecret] = useState(false);

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!bucket || !accessKey || !secretKey) return;
        await onTestConnection({
            endpoint: endpoint || null,
            bucket,
            region: region || null,
            access_key_id: accessKey,
            secret_access_key: secretKey,
            root: root || null,
        });
    };

    const presets = [
        { label: 'Custom S3', endpoint: '', hint: 'Any S3-compatible endpoint' },
        { label: 'Cloudflare R2', endpoint: 'https://<account-id>.r2.cloudflarestorage.com', hint: 'R2 — no egress fees' },
        { label: 'Backblaze B2', endpoint: 'https://s3.<region>.backblazeb2.com', hint: 'B2 S3 API' },
        { label: 'Wasabi', endpoint: 'https://s3.<region>.wasabisys.com', hint: 'No egress, $6.99/TB/mo' },
    ];

    return (
        <form onSubmit={handleSubmit} className="space-y-5">
            {/* Presets */}
            <div className="space-y-2">
                <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                    Provider Preset
                </label>
                <div className="flex flex-wrap gap-2">
                    {presets.map(p => (
                        <button
                            key={p.label}
                            type="button"
                            onClick={() => setEndpoint(p.endpoint)}
                            className={cn(
                                'px-3 py-1.5 rounded-lg text-xs font-medium border transition-all',
                                endpoint === p.endpoint
                                    ? 'border-primary bg-primary/10 text-primary'
                                    : 'border-border/50 bg-muted/30 text-muted-foreground hover:bg-muted/60'
                            )}
                            title={p.hint}
                        >
                            {p.label}
                        </button>
                    ))}
                </div>
            </div>

            <div className="grid gap-4">
                <div className="space-y-1.5">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                        Endpoint URL <span className="text-muted-foreground/40">(blank for AWS)</span>
                    </label>
                    <input
                        type="url"
                        value={endpoint}
                        onChange={e => setEndpoint(e.target.value)}
                        placeholder="https://s3.amazonaws.com or custom endpoint"
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>

                <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-1.5">
                        <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                            Bucket *
                        </label>
                        <input
                            type="text"
                            value={bucket}
                            onChange={e => setBucket(e.target.value)}
                            placeholder="my-scrappy-backup"
                            required
                            className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                        />
                    </div>
                    <div className="space-y-1.5">
                        <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                            Region
                        </label>
                        <input
                            type="text"
                            value={region}
                            onChange={e => setRegion(e.target.value)}
                            placeholder="auto"
                            className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                        />
                    </div>
                </div>

                <div className="space-y-1.5">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                        Access Key ID *
                    </label>
                    <input
                        type="text"
                        value={accessKey}
                        onChange={e => setAccessKey(e.target.value)}
                        placeholder="AKIA..."
                        required
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>

                <div className="space-y-1.5">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                        Secret Access Key *
                    </label>
                    <div className="relative">
                        <input
                            type={showSecret ? 'text' : 'password'}
                            value={secretKey}
                            onChange={e => setSecretKey(e.target.value)}
                            placeholder="wJal..."
                            required
                            className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 pr-12 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                        />
                        <button
                            type="button"
                            onClick={() => setShowSecret(!showSecret)}
                            className="absolute right-3 top-2.5 text-muted-foreground hover:text-foreground transition-colors"
                        >
                            {showSecret ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                        </button>
                    </div>
                </div>

                <div className="space-y-1.5">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                        Root Path <span className="text-muted-foreground/40">(prefix inside bucket)</span>
                    </label>
                    <input
                        type="text"
                        value={root}
                        onChange={e => setRoot(e.target.value)}
                        placeholder="scrappy-data"
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>
            </div>

            <button
                type="submit"
                disabled={testing || !bucket || !accessKey || !secretKey}
                className={cn(
                    'w-full h-11 rounded-xl bg-primary text-primary-foreground font-bold text-xs uppercase tracking-wider',
                    'flex items-center justify-center gap-2 shadow-sm hover:bg-primary/90 transition-all hover:translate-y-[-1px]',
                    (testing || !bucket || !accessKey || !secretKey) && 'opacity-50 cursor-not-allowed transform-none'
                )}
            >
                {testing ? (
                    <><Loader2 className="w-4 h-4 animate-spin" /> Testing Connection...</>
                ) : (
                    <><Wifi className="w-4 h-4" /> Test Connection</>
                )}
            </button>
        </form>
    );
}

// ── A5-5: MigrationProgressDialog ────────────────────────────────────────

function MigrationProgressDialog({
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
                                <div
                                    key={phase}
                                    className={cn(
                                        'flex items-center gap-2.5 px-3 py-1.5 rounded-lg text-sm transition-all',
                                        isDone ? 'text-muted-foreground/60' :
                                            isCurrent ? 'bg-primary/5 text-foreground font-medium' :
                                                'text-muted-foreground/30'
                                    )}
                                >
                                    {isDone ? (
                                        <CheckCircle2 className="w-3.5 h-3.5 text-emerald-500 shrink-0" />
                                    ) : isCurrent ? (
                                        <Loader2 className="w-3.5 h-3.5 text-primary animate-spin shrink-0" />
                                    ) : (
                                        <div className="w-3.5 h-3.5 rounded-full border border-border/50 shrink-0" />
                                    )}
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

// ── A5-6: RecoveryKeyPanel ───────────────────────────────────────────────

function RecoveryKeyPanel() {
    const [recoveryKey, setRecoveryKey] = useState<string | null>(null);
    const [showKey, setShowKey] = useState(false);
    const [loading, setLoading] = useState(false);
    const [importKey, setImportKey] = useState('');
    const [importing, setImporting] = useState(false);
    const [showImport, setShowImport] = useState(false);

    const handleGetKey = async () => {
        setLoading(true);
        try {
            const key = await invoke<string>('cloud_get_recovery_key');
            setRecoveryKey(key);
            setShowKey(true);
        } catch (e) {
            toast.error('Failed to retrieve recovery key: ' + String(e));
        } finally {
            setLoading(false);
        }
    };

    const handleCopy = async () => {
        if (!recoveryKey) return;
        try {
            await navigator.clipboard.writeText(recoveryKey);
            toast.success('Recovery key copied to clipboard');
        } catch {
            toast.error('Failed to copy');
        }
    };

    const handleImport = async () => {
        if (!importKey.trim()) return;
        setImporting(true);
        try {
            await invoke('cloud_import_recovery_key', { recoveryKey: importKey.trim() });
            toast.success('Recovery key imported successfully');
            setImportKey('');
            setShowImport(false);
        } catch (e) {
            toast.error('Failed to import recovery key: ' + String(e));
        } finally {
            setImporting(false);
        }
    };

    return (
        <div className="p-5 rounded-2xl border border-border/50 bg-card/40 space-y-4">
            <div className="flex items-center gap-3">
                <div className="p-2.5 rounded-xl bg-amber-500/10">
                    <Shield className="w-5 h-5 text-amber-500" />
                </div>
                <div>
                    <h3 className="font-bold text-base">Recovery Key</h3>
                    <p className="text-xs text-muted-foreground">
                        Required to decrypt your data on a new device. Store this safely — it cannot be regenerated.
                    </p>
                </div>
            </div>

            {showKey && recoveryKey ? (
                <div className="space-y-3">
                    <div className="p-3 rounded-xl bg-muted/30 border border-border/30 font-mono text-xs break-all select-all">
                        {recoveryKey}
                    </div>
                    <div className="flex gap-2">
                        <button
                            onClick={handleCopy}
                            className="flex items-center gap-1.5 px-3 h-8 rounded-lg text-xs font-medium border border-border/50 hover:bg-accent/50 transition-all"
                        >
                            <Copy className="w-3.5 h-3.5" /> Copy
                        </button>
                        <button
                            onClick={() => { setShowKey(false); setRecoveryKey(null); }}
                            className="flex items-center gap-1.5 px-3 h-8 rounded-lg text-xs font-medium text-muted-foreground hover:bg-muted/50 transition-all"
                        >
                            <EyeOff className="w-3.5 h-3.5" /> Hide
                        </button>
                    </div>
                </div>
            ) : (
                <div className="flex gap-2">
                    <button
                        onClick={handleGetKey}
                        disabled={loading}
                        className={cn(
                            'flex items-center gap-1.5 px-4 h-9 rounded-xl text-xs font-bold uppercase tracking-wider',
                            'bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20',
                            'hover:bg-amber-500/20 transition-all',
                            loading && 'opacity-50 cursor-wait'
                        )}
                    >
                        {loading ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Eye className="w-3.5 h-3.5" />}
                        Show Recovery Key
                    </button>
                    <button
                        onClick={() => setShowImport(!showImport)}
                        className="flex items-center gap-1.5 px-4 h-9 rounded-xl text-xs font-bold uppercase tracking-wider
                            border border-border/50 text-muted-foreground hover:bg-muted/50 transition-all"
                    >
                        <Key className="w-3.5 h-3.5" /> Import Key
                    </button>
                </div>
            )}

            <AnimatePresence>
                {showImport && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="flex gap-2 pt-2">
                            <input
                                type="text"
                                value={importKey}
                                onChange={e => setImportKey(e.target.value)}
                                placeholder="Paste your recovery key here…"
                                className="flex-1 h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                            />
                            <button
                                onClick={handleImport}
                                disabled={importing || !importKey.trim()}
                                className={cn(
                                    'px-4 h-10 rounded-xl bg-primary text-primary-foreground text-xs font-bold uppercase tracking-wider',
                                    'hover:bg-primary/90 transition-all shadow-sm',
                                    (importing || !importKey.trim()) && 'opacity-50 cursor-not-allowed'
                                )}
                            >
                                {importing ? <Loader2 className="w-4 h-4 animate-spin" /> : 'Import'}
                            </button>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            <div className="flex items-start gap-2 p-3 rounded-xl bg-amber-500/5 border border-amber-500/10 text-[11px] text-amber-600 dark:text-amber-400 leading-relaxed">
                <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                <span>
                    <strong>Important:</strong> Without this key, your encrypted cloud data is <strong>permanently unrecoverable</strong>.
                    Store it in a password manager or write it down and keep it in a secure place.
                </span>
            </div>
        </div>
    );
}

// ── A5-3: CloudProviderPicker ────────────────────────────────────────────

const PROVIDERS = [
    {
        id: 's3',
        name: 'S3-Compatible',
        description: 'AWS S3, Cloudflare R2, Backblaze B2, MinIO, Wasabi, DigitalOcean Spaces',
        icon: Server,
        color: 'text-blue-500',
        gradient: 'from-blue-500/20 to-sky-500/10',
        available: true,
    },
    {
        id: 'icloud',
        name: 'iCloud Drive',
        description: 'Apple iCloud — native macOS integration',
        icon: Cloud,
        color: 'text-sky-500',
        gradient: 'from-sky-500/20 to-cyan-500/10',
        available: false,
    },
    {
        id: 'gdrive',
        name: 'Google Drive',
        description: 'Google Drive via OAuth — free 15 GB tier',
        icon: Cloud,
        color: 'text-amber-500',
        gradient: 'from-amber-500/20 to-yellow-500/10',
        available: false,
    },
    {
        id: 'dropbox',
        name: 'Dropbox',
        description: 'Dropbox via OAuth — free 2 GB tier',
        icon: Cloud,
        color: 'text-blue-600',
        gradient: 'from-blue-600/20 to-indigo-500/10',
        available: false,
    },
];

// ── A5-1: Main StorageTab Component ──────────────────────────────────────

export function StorageTab() {
    const {
        status, breakdown, totalSize, migrationProgress, loading, error: _error,
        isCloud, isLocal, isMigrating, refresh, refreshBreakdown, setMigrationProgress,
    } = useCloudStatus();

    const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
    const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);
    const [testing, setTesting] = useState(false);
    const [cancelling, setCancelling] = useState(false);
    const [showMigrationDialog, setShowMigrationDialog] = useState(false);

    // Auto-show migration dialog when migration starts
    useEffect(() => {
        if (isMigrating && migrationProgress) {
            setShowMigrationDialog(true);
        }
    }, [isMigrating, migrationProgress]);

    const handleTestConnection = async (config: S3ConfigInput) => {
        setTesting(true);
        setTestResult(null);
        try {
            const result = await invoke<ConnectionTestResult>('cloud_test_connection', { config });
            setTestResult(result);
            if (result.connected) {
                toast.success(`Connected to ${result.provider_name}!`);
            } else {
                toast.error(result.error ?? 'Connection failed');
            }
        } catch (e) {
            toast.error('Connection test failed: ' + String(e));
        } finally {
            setTesting(false);
        }
    };

    const handleMigrateToCloud = async () => {
        try {
            setShowMigrationDialog(true);
            await invoke('cloud_migrate_to_cloud');
        } catch (e) {
            toast.error('Migration failed: ' + String(e));
        }
    };

    const handleMigrateToLocal = async () => {
        try {
            setShowMigrationDialog(true);
            await invoke('cloud_migrate_to_local');
        } catch (e) {
            toast.error('Migration failed: ' + String(e));
        }
    };

    const handleCancelMigration = async () => {
        setCancelling(true);
        try {
            await invoke('cloud_cancel_migration');
            toast.info('Migration cancellation requested');
        } catch (e) {
            toast.error('Failed to cancel: ' + String(e));
        } finally {
            setCancelling(false);
        }
    };

    const handleCloseMigrationDialog = () => {
        setShowMigrationDialog(false);
        setMigrationProgress(null);
        refresh();
        refreshBreakdown();
    };

    if (loading) {
        return (
            <div className="flex items-center justify-center p-20">
                <Loader2 className="w-8 h-8 animate-spin text-primary/50" />
            </div>
        );
    }

    return (
        <div className="space-y-8 pb-20">

            {/* ── Current Mode Card ────────────────────────────────────── */}
            <div className="relative overflow-hidden rounded-2xl border border-border/50 bg-card/40 shadow-sm">
                <div className={cn(
                    'absolute inset-0 bg-gradient-to-br pointer-events-none opacity-40',
                    isCloud ? 'from-blue-500/20 to-sky-500/10' : 'from-emerald-500/20 to-green-500/10'
                )} />
                <div className="relative p-6 space-y-4">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                            <div className={cn(
                                'p-3 rounded-xl',
                                isCloud ? 'bg-blue-500/10' : 'bg-emerald-500/10'
                            )}>
                                {isCloud ? (
                                    <Cloud className="w-6 h-6 text-blue-500" />
                                ) : (
                                    <HardDrive className="w-6 h-6 text-emerald-500" />
                                )}
                            </div>
                            <div>
                                <h2 className="text-xl font-bold">
                                    {isCloud ? 'Cloud Storage' : 'Local Storage'}
                                </h2>
                                <p className="text-sm text-muted-foreground">
                                    {isCloud
                                        ? `Connected to ${status?.provider_name ?? 'cloud provider'}. Data is encrypted and synced.`
                                        : 'All data stored on this device. Private and offline-capable.'
                                    }
                                </p>
                            </div>
                        </div>

                        <div className="flex items-center gap-3">
                            {isCloud && (
                                <div className="flex items-center gap-1.5 px-3 py-1.5 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 rounded-full text-[10px] font-bold uppercase tracking-wider border border-emerald-500/20">
                                    <Lock className="w-3 h-3" />
                                    End-to-End Encrypted
                                </div>
                            )}
                            <button
                                onClick={() => { refresh(); refreshBreakdown(); }}
                                className="p-2 hover:bg-muted/50 rounded-lg transition-colors"
                                title="Refresh"
                            >
                                <RefreshCw className="w-4 h-4 text-muted-foreground" />
                            </button>
                        </div>
                    </div>

                    {/* Quick stats */}
                    <div className="flex gap-6 text-xs">
                        <div>
                            <span className="text-muted-foreground">Total Size:</span>
                            <span className="ml-1.5 font-bold text-foreground">{formatBytes(totalSize)}</span>
                        </div>
                        {status?.last_sync_at && (
                            <div>
                                <span className="text-muted-foreground">Last Sync:</span>
                                <span className="ml-1.5 font-bold text-foreground">
                                    {new Date(status.last_sync_at * 1000).toLocaleString()}
                                </span>
                            </div>
                        )}
                        {isCloud && status?.storage_available != null && (
                            <div>
                                <span className="text-muted-foreground">Cloud Usage:</span>
                                <span className="ml-1.5 font-bold text-foreground">
                                    {formatBytes(status?.storage_used ?? 0)}
                                    {status.storage_available > 0 && ` / ${formatBytes(status.storage_available)}`}
                                </span>
                            </div>
                        )}
                    </div>
                </div>
            </div>

            {/* ── Storage Breakdown ────────────────────────────────────── */}
            <div className="p-6 rounded-2xl border border-border/50 bg-card/40 shadow-sm">
                <StorageBreakdown breakdown={breakdown} totalSize={totalSize} />
            </div>

            {/* ── Cloud Provider Picker ────────────────────────────────── */}
            {isLocal && (
                <div className="space-y-4">
                    <div className="flex items-center gap-2">
                        <Cloud className="w-4 h-4 text-primary" />
                        <h3 className="text-sm font-bold uppercase tracking-wider text-muted-foreground/60">
                            Enable Cloud Storage
                        </h3>
                    </div>

                    <div className="grid grid-cols-2 gap-3">
                        {PROVIDERS.map(p => {
                            const Icon = p.icon;
                            const isSelected = selectedProvider === p.id;
                            return (
                                <button
                                    key={p.id}
                                    onClick={() => p.available && setSelectedProvider(isSelected ? null : p.id)}
                                    disabled={!p.available}
                                    className={cn(
                                        'relative overflow-hidden rounded-2xl border p-5 text-left transition-all duration-300',
                                        isSelected
                                            ? 'border-primary bg-primary/5 ring-2 ring-primary/20 shadow-md'
                                            : p.available
                                                ? 'border-border/50 bg-card/40 hover:bg-card/60 hover:border-border cursor-pointer shadow-sm'
                                                : 'border-border/30 bg-muted/20 opacity-50 cursor-not-allowed'
                                    )}
                                >
                                    <div className={cn(
                                        'absolute inset-0 bg-gradient-to-br opacity-0 transition-opacity duration-500 pointer-events-none',
                                        isSelected && 'opacity-100',
                                        p.gradient
                                    )} />
                                    <div className="relative space-y-2">
                                        <div className="flex items-center gap-2.5">
                                            <Icon className={cn('w-5 h-5', p.available ? p.color : 'text-muted-foreground')} />
                                            <span className="font-bold text-sm">{p.name}</span>
                                            {!p.available && (
                                                <span className="ml-auto text-[9px] font-bold uppercase bg-muted/50 text-muted-foreground px-2 py-0.5 rounded">
                                                    Coming Soon
                                                </span>
                                            )}
                                        </div>
                                        <p className="text-xs text-muted-foreground leading-relaxed">{p.description}</p>
                                    </div>
                                </button>
                            );
                        })}
                    </div>
                </div>
            )}

            {/* ── S3 Config Form ───────────────────────────────────────── */}
            <AnimatePresence>
                {selectedProvider === 's3' && isLocal && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        transition={{ duration: 0.3 }}
                        className="overflow-hidden"
                    >
                        <div className="p-6 rounded-2xl border border-border/50 bg-card/40 shadow-sm space-y-5">
                            <h3 className="font-bold text-base flex items-center gap-2">
                                <Server className="w-4 h-4 text-blue-500" />
                                S3 Connection
                            </h3>
                            <S3ConfigForm onTestConnection={handleTestConnection} testing={testing} />

                            {/* Connection test result */}
                            <AnimatePresence>
                                {testResult && (
                                    <motion.div
                                        initial={{ height: 0, opacity: 0 }}
                                        animate={{ height: 'auto', opacity: 1 }}
                                        exit={{ height: 0, opacity: 0 }}
                                    >
                                        <div className={cn(
                                            'p-4 rounded-xl border',
                                            testResult.connected
                                                ? 'bg-emerald-500/5 border-emerald-500/20'
                                                : 'bg-rose-500/5 border-rose-500/20'
                                        )}>
                                            <div className="flex items-center gap-2 mb-2">
                                                {testResult.connected ? (
                                                    <CheckCircle2 className="w-4 h-4 text-emerald-500" />
                                                ) : (
                                                    <XCircle className="w-4 h-4 text-rose-500" />
                                                )}
                                                <span className="font-bold text-sm">
                                                    {testResult.connected ? `Connected to ${testResult.provider_name}` : 'Connection Failed'}
                                                </span>
                                            </div>
                                            {testResult.connected && (
                                                <p className="text-xs text-muted-foreground">
                                                    Storage: {formatBytes(testResult.storage_used)} used
                                                    {testResult.storage_available != null && ` / ${formatBytes(testResult.storage_available)} available`}
                                                </p>
                                            )}
                                            {testResult.error && (
                                                <p className="text-xs text-rose-600 dark:text-rose-400 mt-1">{testResult.error}</p>
                                            )}

                                            {/* Migrate button */}
                                            {testResult.connected && (
                                                <button
                                                    onClick={handleMigrateToCloud}
                                                    className="mt-4 w-full h-11 rounded-xl bg-gradient-to-r from-blue-500 to-sky-500 text-white font-bold text-xs uppercase tracking-wider
                                                        flex items-center justify-center gap-2 shadow-lg shadow-blue-500/20 hover:shadow-blue-500/30
                                                        hover:translate-y-[-1px] transition-all"
                                                >
                                                    <Upload className="w-4 h-4" />
                                                    Migrate to Cloud
                                                </button>
                                            )}
                                        </div>
                                    </motion.div>
                                )}
                            </AnimatePresence>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* ── Migrate Back to Local ────────────────────────────────── */}
            {isCloud && (
                <div className="p-6 rounded-2xl border border-border/50 bg-card/40 shadow-sm space-y-4">
                    <div className="flex items-center gap-3">
                        <div className="p-2.5 rounded-xl bg-emerald-500/10">
                            <HardDrive className="w-5 h-5 text-emerald-500" />
                        </div>
                        <div>
                            <h3 className="font-bold text-base">Switch to Local Storage</h3>
                            <p className="text-xs text-muted-foreground">
                                Download all data from the cloud and switch back to local-only mode.
                            </p>
                        </div>
                    </div>
                    <button
                        onClick={handleMigrateToLocal}
                        disabled={isMigrating}
                        className={cn(
                            'w-full h-11 rounded-xl border border-emerald-500/30 text-emerald-600 dark:text-emerald-400 font-bold text-xs uppercase tracking-wider',
                            'flex items-center justify-center gap-2 hover:bg-emerald-500/10 transition-all',
                            isMigrating && 'opacity-50 cursor-not-allowed'
                        )}
                    >
                        {isMigrating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Download className="w-4 h-4" />}
                        Migrate to Local
                    </button>
                </div>
            )}

            {/* ── Recovery Key Panel ───────────────────────────────────── */}
            <RecoveryKeyPanel />

            {/* ── Info Footer ─────────────────────────────────────────── */}
            <div className="p-5 rounded-2xl border border-primary/10 bg-primary/5 flex gap-4 items-start">
                <div className="p-2.5 bg-primary/10 rounded-xl shrink-0">
                    <Info className="w-5 h-5 text-primary" />
                </div>
                <div className="space-y-1.5">
                    <p className="text-sm font-bold">How Cloud Storage Works</p>
                    <p className="text-xs text-muted-foreground leading-relaxed">
                        All files are <span className="text-foreground font-medium">encrypted client-side</span> with AES-256-GCM before upload.
                        Your encryption key never leaves this device. The cloud provider only sees encrypted blobs — they cannot read your data.
                        You can migrate freely between local and cloud modes at any time. Use the recovery key to access your data on a new device.
                    </p>
                </div>
            </div>

            {/* ── Migration Progress Dialog ────────────────────────────── */}
            <AnimatePresence>
                {showMigrationDialog && migrationProgress && (
                    <MigrationProgressDialog
                        progress={migrationProgress}
                        onCancel={handleCancelMigration}
                        cancelling={cancelling}
                        onClose={handleCloseMigrationDialog}
                    />
                )}
            </AnimatePresence>
        </div>
    );
}
