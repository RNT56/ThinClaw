import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Terminal,
    RefreshCw,
    Save,
    Play,
    Square,
    AlertTriangle,
    Download,
    Binary
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

export function OpenClawSystemControl() {
    const [activeTab, setActiveTab] = useState<'config' | 'logs' | 'system'>('config');
    const [config, setConfig] = useState<any>(null);
    const [snapshotHash, setSnapshotHash] = useState<string | null>(null);
    const [schema, setSchema] = useState<any>(null);
    const [logs, setLogs] = useState<string[]>([]);
    const [status, setStatus] = useState<openclaw.OpenClawStatus | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [isSaving, setIsSaving] = useState(false);
    const logEndRef = useRef<HTMLDivElement>(null);

    const fetchData = async () => {
        try {
            const [s, c, sc] = await Promise.all([
                openclaw.getOpenClawStatus(),
                openclaw.getOpenClawConfig(),
                openclaw.getOpenClawConfigSchema()
            ]);
            setStatus(s);
            if (c) {
                // Handle both snapshot { config, hash } and direct config object
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

    const fetchLogs = async () => {
        try {
            const data = await openclaw.getOpenClawLogsTail(100);
            if (data && Array.isArray(data.lines)) {
                setLogs(data.lines);
            }
        } catch (e) {
            console.error('Failed to fetch logs:', e);
        }
    };

    useEffect(() => {
        fetchData();
    }, []);

    useEffect(() => {
        if (activeTab === 'logs') {
            fetchLogs();
            const interval = setInterval(fetchLogs, 2000);
            return () => clearInterval(interval);
        }
    }, [activeTab]);

    useEffect(() => {
        if (activeTab === 'logs' && logEndRef.current) {
            logEndRef.current.scrollIntoView({ behavior: 'smooth' });
        }
    }, [logs, activeTab]);

    const handleSaveConfig = async () => {
        setIsSaving(true);
        try {
            await openclaw.patchOpenClawConfig({
                raw: JSON.stringify(config),
                baseHash: snapshotHash
            });
            toast.success('Configuration saved and hot-reloaded');
            // Refresh hash after save
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
            if (status?.gateway_running) {
                await openclaw.stopOpenClawGateway();
                toast.success('Gateway stopped');
            } else {
                await openclaw.startOpenClawGateway();
                toast.success('Gateway started');
            }
            fetchData();
        } catch (e) {
            toast.error(`Operation failed: ${e}`);
        }
    };

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
                <div className="bg-white/5 border border-white/10 rounded-xl p-1 flex items-center gap-1 shadow-inner">
                    {(['config', 'logs', 'system'] as const).map((t) => (
                        <button
                            key={t}
                            onClick={() => setActiveTab(t)}
                            className={cn(
                                "px-4 py-2 rounded-lg text-xs font-bold uppercase tracking-widest transition-all",
                                activeTab === t ? "bg-primary text-primary-foreground shadow-lg shadow-primary/20" : "text-muted-foreground hover:text-foreground hover:bg-white/5"
                            )}
                        >
                            {t}
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
                                        <div key={key} className="space-y-2 p-4 rounded-xl bg-white/[0.02] border border-white/5">
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
                                                    className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-xs font-mono text-gray-300 min-h-[100px] focus:border-primary/50 outline-none transition-colors"
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
                                                    className="w-full bg-black/40 border border-white/10 rounded-lg px-3 py-2 text-xs font-mono text-gray-300 focus:border-primary/50 outline-none transition-colors"
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
                            <div className="pt-6 border-t border-white/10 flex justify-end">
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
                            className="flex-1 bg-[#050505] rounded-2xl border border-white/10 shadow-2xl flex flex-col overflow-hidden font-mono"
                        >
                            <div className="p-3 border-b border-white/5 bg-white/[0.02] flex items-center justify-between">
                                <div className="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-muted-foreground">
                                    <Terminal className="w-3.5 h-3.5" />
                                    Live Process Stream
                                </div>
                                <div className="flex items-center gap-4 text-[9px] text-muted-foreground uppercase font-bold tracking-tighter">
                                    <span>ANSI Support</span>
                                    <div className="flex items-center gap-1">
                                        <div className="w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse" />
                                        Streaming
                                    </div>
                                </div>
                            </div>
                            <div className="flex-1 p-4 overflow-y-auto scrollbar-thin text-xs text-gray-400 space-y-1">
                                {logs.map((log, i) => (
                                    <div key={i} className="break-all whitespace-pre-wrap selection:bg-primary/30">
                                        <span className="text-muted-foreground/30 mr-3 select-none">{(i + 1).toString().padStart(3, '0')}</span>
                                        {log}
                                    </div>
                                ))}
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
                                <div className="p-6 rounded-2xl border bg-card/30 border-white/10">
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
                                            status?.gateway_running
                                                ? "bg-red-500/10 text-red-500 border border-red-500/20 hover:bg-red-500/20 shadow-red-500/5"
                                                : "bg-green-500/10 text-green-500 border border-green-500/20 hover:bg-green-500/20 shadow-green-500/5"
                                        )}
                                    >
                                        {status?.gateway_running ? <Square className="w-4 h-4 fill-current" /> : <Play className="w-4 h-4 fill-current" />}
                                        {status?.gateway_running ? 'Emergency Shutdown' : 'Initialize Gateway'}
                                    </button>
                                </div>

                                <div className="p-6 rounded-2xl border bg-card/30 border-white/10">
                                    <h3 className="text-sm font-semibold mb-4 flex items-center gap-2">
                                        <Download className="w-4 h-4 text-primary" />
                                        Node Maintenance
                                    </h3>
                                    <p className="text-xs text-muted-foreground mb-6">
                                        Trigger a binary update and component rebuild. The gateway will attempt to recompile skills and internal plugins.
                                    </p>
                                    <button
                                        onClick={handleUpdate}
                                        className="w-full py-2.5 rounded-xl text-xs font-bold uppercase tracking-widest bg-white/5 border border-white/10 hover:bg-white/10 transition-all flex items-center justify-center gap-2"
                                    >
                                        <Binary className="w-4 h-4" />
                                        Run Update & Rebuild
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
