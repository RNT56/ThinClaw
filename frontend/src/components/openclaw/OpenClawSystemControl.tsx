import { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Terminal,
    RefreshCw,
    Save,
    Play,
    Square,
    AlertTriangle,
    Download,
    Binary,
    Trash2,
    Copy,
    ChevronDown,
    ChevronUp,
    Search,
    X
} from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

interface LogLine {
    timestamp: string;
    level: string;
    target: string;
    message: string;
}

const LEVEL_STYLES: Record<string, { badge: string; text: string; dot: string }> = {
    TRACE: { badge: 'bg-gray-500/20 text-gray-400', text: 'text-gray-600 dark:text-gray-500', dot: 'bg-gray-500' },
    DEBUG: { badge: 'bg-blue-500/20 text-blue-400', text: 'text-blue-600 dark:text-blue-300', dot: 'bg-blue-500' },
    INFO: { badge: 'bg-emerald-500/20 text-primary', text: 'text-emerald-600 dark:text-emerald-300', dot: 'bg-emerald-500' },
    WARN: { badge: 'bg-amber-500/20 text-muted-foreground', text: 'text-amber-600 dark:text-amber-300', dot: 'bg-amber-500' },
    ERROR: { badge: 'bg-red-500/20 text-red-400', text: 'text-red-600 dark:text-red-300', dot: 'bg-red-500' },
};

function LogRow({ entry, idx }: { entry: LogLine; idx: number }) {
    const s = LEVEL_STYLES[entry.level] ?? LEVEL_STYLES.DEBUG;
    const shortTarget = entry.target.split('::').slice(-2).join('::');
    const timeStr = entry.timestamp.slice(11, 23); // HH:mm:ss.mmm

    return (
        <div
            className={cn(
                'flex items-start gap-2 px-3 py-1 text-xs font-mono group hover:bg-muted/20 rounded transition-colors select-text',
                idx % 2 === 0 ? '' : 'bg-muted/10'
            )}
        >
            <span className="text-muted-foreground/30 select-none w-[30px] text-right shrink-0 mt-[1px]">
                {(idx + 1).toString().padStart(3, '0')}
            </span>
            <span className="text-muted-foreground/50 shrink-0 mt-[1px] tabular-nums">{timeStr}</span>
            <span className={cn('shrink-0 px-1.5 py-0 rounded text-[9px] font-bold uppercase tracking-widest leading-5', s.badge)}>
                {entry.level.slice(0, 4)}
            </span>
            <span className="text-muted-foreground/40 shrink-0 mt-[1px] max-w-[120px] truncate">{shortTarget}</span>
            <span className={cn('flex-1 break-all whitespace-pre-wrap leading-relaxed', s.text)}>
                {entry.message}
            </span>
        </div>
    );
}

export function OpenClawSystemControl() {
    const [activeTab, setActiveTab] = useState<'config' | 'logs' | 'system'>('config');
    const [config, setConfig] = useState<any>(null);
    const [snapshotHash, setSnapshotHash] = useState<string | null>(null);
    const [schema, setSchema] = useState<any>(null);
    const [logs, setLogs] = useState<LogLine[]>([]);
    const [filter, setFilter] = useState('');
    const [levelFilter, setLevelFilter] = useState<string>('ALL');
    const [autoScroll, setAutoScroll] = useState(true);
    const [isLive, setIsLive] = useState(false);
    const [status, setStatus] = useState<openclaw.OpenClawStatus | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [isSaving, setIsSaving] = useState(false);
    const logEndRef = useRef<HTMLDivElement>(null);
    const logContainerRef = useRef<HTMLDivElement>(null);
    const MAX_LOGS = 2000;

    const appendLog = useCallback((entry: LogLine) => {
        setLogs(prev => {
            const next = [...prev, entry];
            return next.length > MAX_LOGS ? next.slice(next.length - MAX_LOGS) : next;
        });
    }, []);

    const fetchData = async () => {
        try {
            const [s, c, sc] = await Promise.all([
                openclaw.getOpenClawStatus(),
                openclaw.getOpenClawConfig(),
                openclaw.getOpenClawConfigSchema()
            ]);
            setStatus(s);
            if (c) {
                if (c.config && typeof c.config === 'object') {
                    setConfig(c.config);
                    setSnapshotHash(c.hash || null);
                } else {
                    setConfig(c);
                    setSnapshotHash(null);
                }
            } else {
                setConfig(null);
            }
            setSchema(sc);
        } catch (e) {
            console.error('Failed to fetch system data:', e);
            toast.error(`System Sync Failed: ${e}`);
        } finally {
            setIsLoading(false);
        }
    };

    // Load historical logs when entering the logs tab
    const fetchLogHistory = useCallback(async () => {
        try {
            const data = await openclaw.getOpenClawLogsTail(500);
            if (data && Array.isArray((data as any).logs)) {
                setLogs((data as any).logs as LogLine[]);
            }
        } catch (e) {
            console.error('Failed to fetch log history:', e);
        }
    }, []);

    useEffect(() => {
        fetchData();
    }, []);

    // Live log streaming via Tauri events
    useEffect(() => {
        if (activeTab !== 'logs') return;

        fetchLogHistory();

        const unlistenPromise = listen<any>('openclaw-event', (event) => {
            const payload = event.payload;
            if (payload?.kind === 'LogEntry') {
                setIsLive(true);
                appendLog({
                    timestamp: payload.timestamp,
                    level: payload.level,
                    target: payload.target,
                    message: payload.message,
                });
            }
        });

        return () => {
            unlistenPromise.then(fn => fn());
            setIsLive(false);
        };
    }, [activeTab, appendLog, fetchLogHistory]);

    // Auto-scroll
    useEffect(() => {
        if (autoScroll && activeTab === 'logs' && logEndRef.current) {
            logEndRef.current.scrollIntoView({ behavior: 'smooth' });
        }
    }, [logs, autoScroll, activeTab]);

    // Detect manual scroll to disable auto-scroll
    const handleScroll = () => {
        const el = logContainerRef.current;
        if (!el) return;
        const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        setAutoScroll(atBottom);
    };

    const handleSaveConfig = async () => {
        setIsSaving(true);
        try {
            await openclaw.patchOpenClawConfig({
                raw: JSON.stringify(config),
                baseHash: snapshotHash
            });
            toast.success('Configuration saved and hot-reloaded');
            const c = await openclaw.getOpenClawConfig();
            if (c && c.hash) setSnapshotHash(c.hash);
        } catch (e) {
            toast.error(`Failed to save configuration: ${e}`);
        } finally {
            setIsSaving(false);
        }
    };

    const handleUpdate = async () => {
        toast.promise(openclaw.runOpenClawUpdate(), {
            loading: 'Checking for updates and rebuilding components...',
            success: 'Update completed. Restarting node...',
            error: 'Update failed'
        });
    };

    const handleToggleGateway = async () => {
        try {
            if (status?.engine_running) {
                await openclaw.stopOpenClawGateway();
                toast.success('Engine stopped');
            } else {
                await openclaw.startOpenClawGateway();
                toast.success('Engine started');
            }
            fetchData();
        } catch (e) {
            toast.error(`Operation failed: ${e}`);
        }
    };

    const handleCopyLogs = () => {
        const text = logs.map(e =>
            `${e.timestamp} [${e.level.padStart(5)}] ${e.target}  ${e.message}`
        ).join('\n');
        navigator.clipboard.writeText(text).then(() => toast.success('Logs copied to clipboard'));
    };

    const filteredLogs = logs.filter(e => {
        if (levelFilter !== 'ALL' && e.level !== levelFilter) return false;
        if (!filter) return true;
        const q = filter.toLowerCase();
        return e.message.toLowerCase().includes(q) || e.target.toLowerCase().includes(q);
    });

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center p-8">
                <RefreshCw className="w-8 h-8 text-primary animate-spin" />
            </div>
        );
    }

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 p-8 space-y-8 max-w-6xl mx-auto flex flex-col h-full overflow-hidden"
        >
            <div className="flex items-center justify-between">
                <div>
                    <h1 className="text-3xl font-bold tracking-tight">System Control</h1>
                    <p className="text-muted-foreground mt-1">Foundational node settings and diagnostic routines.</p>
                </div>
                <div className="bg-muted/30 border border-border/40 rounded-xl p-1 flex items-center gap-1 shadow-inner">
                    {(['config', 'logs', 'system'] as const).map((t) => (
                        <button
                            key={t}
                            onClick={() => setActiveTab(t)}
                            className={cn(
                                "px-4 py-2 rounded-lg text-xs font-bold uppercase tracking-widest transition-all",
                                activeTab === t ? "bg-primary text-primary-foreground shadow-lg shadow-primary/20" : "text-muted-foreground hover:text-foreground hover:bg-muted/30"
                            )}
                        >
                            {t}
                            {t === 'logs' && isLive && (
                                <span className="ml-1.5 inline-block w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
                            )}
                        </button>
                    ))}
                </div>
            </div>

            <div className="flex-1 overflow-hidden flex flex-col min-h-0">
                <AnimatePresence mode="wait">
                    {activeTab === 'config' && (
                        <motion.div
                            key="config"
                            initial={{ opacity: 0, x: -20 }}
                            animate={{ opacity: 1, x: 0 }}
                            exit={{ opacity: 0, x: 20 }}
                            className="flex-1 flex flex-col overflow-hidden"
                        >
                            <div className="flex-1 overflow-y-auto pr-4 space-y-6 scrollbar-thin">
                                {config && Object.keys(config).length > 0 ? (
                                    Object.entries(config).map(([key, value]: [string, any]) => (
                                        <div key={key} className="space-y-2 p-4 rounded-xl bg-muted/10 border border-border/30">
                                            <div className="flex items-center justify-between">
                                                <label className="text-xs font-bold uppercase tracking-wider text-primary/80">{key}</label>
                                                <span className="text-[10px] text-muted-foreground font-mono">{typeof value}</span>
                                            </div>
                                            {typeof value === 'object' && value !== null ? (
                                                <textarea
                                                    value={JSON.stringify(value, null, 2)}
                                                    onChange={(e) => {
                                                        try {
                                                            const newVal = JSON.parse(e.target.value);
                                                            setConfig({ ...config, [key]: newVal });
                                                        } catch { }
                                                    }}
                                                    className="w-full bg-muted/30 border border-border/40 rounded-lg p-3 text-xs font-mono text-foreground min-h-[100px] focus:border-primary/50 outline-none transition-colors"
                                                />
                                            ) : typeof value === 'boolean' ? (
                                                <div className="flex items-center gap-2">
                                                    <button
                                                        onClick={() => setConfig({ ...config, [key]: !value })}
                                                        className={cn(
                                                            "px-3 py-1 rounded-md text-[10px] font-bold uppercase tracking-wider transition-all",
                                                            value ? "bg-green-500/20 text-green-500 border border-green-500/20" : "bg-red-500/20 text-red-500 border border-red-500/20"
                                                        )}
                                                    >
                                                        {value ? 'True' : 'False'}
                                                    </button>
                                                </div>
                                            ) : (
                                                <input
                                                    type="text"
                                                    value={value}
                                                    onChange={(e) => setConfig({ ...config, [key]: e.target.value })}
                                                    className="w-full bg-muted/30 border border-border/40 rounded-lg px-3 py-2 text-xs font-mono text-foreground focus:border-primary/50 outline-none transition-colors"
                                                />
                                            )}
                                            {schema?.[key]?.description && (
                                                <p className="text-[10px] text-muted-foreground italic">{schema[key].description}</p>
                                            )}
                                        </div>
                                    ))
                                ) : (
                                    <div className="h-full flex flex-col items-center justify-center p-12 text-center opacity-40">
                                        <AlertTriangle className="w-8 h-8 mb-4" />
                                        <p className="text-sm font-semibold uppercase tracking-widest">No Node Configuration</p>
                                        <p className="text-xs mt-2 text-muted-foreground">The registry returned an empty configuration set.</p>
                                        <button
                                            onClick={() => { setIsLoading(true); fetchData(); }}
                                            className="mt-6 flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-primary hover:underline"
                                        >
                                            <RefreshCw className="w-3 h-3" />
                                            Force Re-Sync
                                        </button>
                                    </div>
                                )}
                            </div>
                            <div className="pt-6 border-t border-border/40 flex justify-end">
                                <button
                                    onClick={handleSaveConfig}
                                    disabled={isSaving}
                                    className="flex items-center gap-2 px-6 py-2.5 rounded-xl bg-primary text-primary-foreground text-sm font-bold shadow-lg shadow-primary/20 hover:opacity-90 disabled:opacity-50 transition-all"
                                >
                                    {isSaving ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
                                    Deploy Configuration
                                </button>
                            </div>
                        </motion.div>
                    )}

                    {activeTab === 'logs' && (
                        <motion.div
                            key="logs"
                            initial={{ opacity: 0, scale: 0.98 }}
                            animate={{ opacity: 1, scale: 1 }}
                            exit={{ opacity: 0, scale: 0.98 }}
                            className="flex-1 bg-zinc-950 dark:bg-[#040404] rounded-2xl border border-border/40 shadow-2xl flex flex-col overflow-hidden"
                        >
                            {/* Toolbar */}
                            <div className="p-3 border-b border-border/30 bg-muted/10 flex items-center gap-3 flex-wrap">
                                <div className="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-muted-foreground">
                                    <Terminal className="w-3.5 h-3.5" />
                                    Agent Internals
                                </div>

                                {/* Live indicator */}
                                <div className="flex items-center gap-1.5 text-[9px] font-bold uppercase tracking-widest">
                                    <span className={cn(
                                        "w-1.5 h-1.5 rounded-full",
                                        isLive ? "bg-emerald-500 animate-pulse" : "bg-muted-foreground/30"
                                    )} />
                                    <span className={isLive ? "text-primary" : "text-muted-foreground/40"}>
                                        {isLive ? 'Live' : 'Idle'}
                                    </span>
                                </div>

                                <span className="text-muted-foreground/30 text-[9px]">|</span>

                                {/* Level filter */}
                                <div className="flex items-center gap-1">
                                    {(['ALL', 'DEBUG', 'INFO', 'WARN', 'ERROR'] as const).map(lvl => (
                                        <button
                                            key={lvl}
                                            onClick={() => setLevelFilter(lvl)}
                                            className={cn(
                                                "px-2 py-0.5 rounded text-[9px] font-bold uppercase tracking-wider transition-all",
                                                levelFilter === lvl
                                                    ? lvl === 'ALL'
                                                        ? 'bg-foreground/10 text-foreground'
                                                        : (LEVEL_STYLES[lvl]?.badge ?? 'bg-foreground/10 text-foreground')
                                                    : 'text-muted-foreground/40 hover:text-muted-foreground/70'
                                            )}
                                        >
                                            {lvl}
                                        </button>
                                    ))}
                                </div>

                                <span className="text-muted-foreground/30 text-[9px]">|</span>

                                {/* Search */}
                                <div className="relative flex items-center flex-1 min-w-[120px] max-w-[220px]">
                                    <Search className="absolute left-2 w-3 h-3 text-muted-foreground/40" />
                                    <input
                                        value={filter}
                                        onChange={e => setFilter(e.target.value)}
                                        placeholder="Filter logs…"
                                        className="w-full bg-muted/30 border border-border/40 rounded-lg pl-7 pr-7 py-1 text-[10px] font-mono text-foreground placeholder-muted-foreground/40 focus:outline-none focus:border-primary/40"
                                    />
                                    {filter && (
                                        <button onClick={() => setFilter('')} className="absolute right-2 text-muted-foreground/40 hover:text-muted-foreground/70">
                                            <X className="w-3 h-3" />
                                        </button>
                                    )}
                                </div>

                                {/* Log count */}
                                <span className="text-[9px] text-muted-foreground/40 font-mono ml-auto">
                                    {filteredLogs.length.toLocaleString()} / {logs.length.toLocaleString()} lines
                                </span>

                                {/* Actions */}
                                <button
                                    onClick={() => setAutoScroll(v => !v)}
                                    title={autoScroll ? 'Auto-scroll ON — click to disable' : 'Auto-scroll OFF — click to enable'}
                                    className={cn(
                                        "p-1.5 rounded-lg transition-all",
                                        autoScroll ? "bg-primary/20 text-primary" : "text-muted-foreground/40 hover:text-muted-foreground/70"
                                    )}
                                >
                                    {autoScroll ? <ChevronDown className="w-3.5 h-3.5" /> : <ChevronUp className="w-3.5 h-3.5" />}
                                </button>
                                <button
                                    onClick={handleCopyLogs}
                                    title="Copy all logs"
                                    className="p-1.5 rounded-lg text-muted-foreground/40 hover:text-muted-foreground/70 transition-all"
                                >
                                    <Copy className="w-3.5 h-3.5" />
                                </button>
                                <button
                                    onClick={fetchLogHistory}
                                    title="Reload history"
                                    className="p-1.5 rounded-lg text-muted-foreground/40 hover:text-muted-foreground/70 transition-all"
                                >
                                    <RefreshCw className="w-3.5 h-3.5" />
                                </button>
                                <button
                                    onClick={() => setLogs([])}
                                    title="Clear logs"
                                    className="p-1.5 rounded-lg text-muted-foreground/40 hover:text-red-400 transition-all"
                                >
                                    <Trash2 className="w-3.5 h-3.5" />
                                </button>
                            </div>

                            {/* Log stream */}
                            <div
                                ref={logContainerRef}
                                onScroll={handleScroll}
                                className="flex-1 overflow-y-auto scrollbar-thin py-2"
                            >
                                {filteredLogs.length === 0 ? (
                                    <div className="h-full flex flex-col items-center justify-center gap-3 opacity-30">
                                        <Terminal className="w-8 h-8" />
                                        <p className="text-xs font-mono uppercase tracking-widest">
                                            {logs.length === 0
                                                ? 'No logs yet — start the engine to see activity'
                                                : 'No entries match your filter'}
                                        </p>
                                    </div>
                                ) : (
                                    filteredLogs.map((entry, i) => (
                                        <LogRow key={i} entry={entry} idx={i} />
                                    ))
                                )}
                                <div ref={logEndRef} />
                            </div>
                        </motion.div>
                    )}

                    {activeTab === 'system' && (
                        <motion.div
                            key="system"
                            initial={{ opacity: 0, y: 20 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: -20 }}
                            className="space-y-6"
                        >
                            <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                                <div className="p-6 rounded-2xl border bg-card/30 border-border/40">
                                    <h3 className="text-sm font-semibold mb-4 flex items-center gap-2">
                                        <RefreshCw className="w-4 h-4 text-primary" />
                                        Process Lifecycle
                                    </h3>
                                    <p className="text-xs text-muted-foreground mb-6">
                                        Stop or restart the OpenClaw Gateway process. This will disconnect all active UI sessions and halt background runners.
                                    </p>
                                    <button
                                        onClick={handleToggleGateway}
                                        className={cn(
                                            "w-full py-2.5 rounded-xl text-xs font-bold uppercase tracking-widest transition-all flex items-center justify-center gap-2 shadow-lg",
                                            status?.engine_running
                                                ? "bg-red-500/10 text-red-500 border border-red-500/20 hover:bg-red-500/20 shadow-red-500/5"
                                                : "bg-green-500/10 text-green-500 border border-green-500/20 hover:bg-green-500/20 shadow-green-500/5"
                                        )}
                                    >
                                        {status?.engine_running ? <Square className="w-4 h-4 fill-current" /> : <Play className="w-4 h-4 fill-current" />}
                                        {status?.engine_running ? 'Emergency Shutdown' : 'Initialize Engine'}
                                    </button>
                                </div>

                                <div className="p-6 rounded-2xl border bg-card/30 border-border/40">
                                    <h3 className="text-sm font-semibold mb-4 flex items-center gap-2">
                                        <Download className="w-4 h-4 text-primary" />
                                        Node Maintenance
                                    </h3>
                                    <p className="text-xs text-muted-foreground mb-6">
                                        Trigger a binary update and component rebuild. The gateway will attempt to recompile skills and internal plugins.
                                    </p>
                                    <button
                                        onClick={handleUpdate}
                                        className="w-full py-2.5 rounded-xl text-xs font-bold uppercase tracking-widest bg-muted/30 border border-border/40 hover:bg-muted/50 transition-all flex items-center justify-center gap-2"
                                    >
                                        <Binary className="w-4 h-4" />
                                        Run Update &amp; Rebuild
                                    </button>
                                </div>
                            </div>

                            <div className="p-6 rounded-2xl border bg-red-500/5 border-red-500/10 flex gap-4">
                                <div className="p-2 bg-red-500/10 rounded-xl h-fit">
                                    <AlertTriangle className="w-5 h-5 text-red-500" />
                                </div>
                                <div>
                                    <h4 className="text-sm font-semibold text-red-500 uppercase tracking-wider text-[10px]">Caution: Experimental Controls</h4>
                                    <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                                        Modifying core system parameters can lead to node instability or channel disconnection.
                                        Always backup your <span className="text-foreground font-mono">config.json</span> before applying wide-scale patches.
                                    </p>
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>
        </motion.div>
    );
}
