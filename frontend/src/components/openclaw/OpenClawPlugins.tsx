import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    RefreshCw,
    Package,
    Plug,
    Trash2,
    Power,
    CheckCircle2,
    AlertCircle,
    XCircle,
    Wrench,
    ChevronDown,
    Puzzle,
    Globe,
    Shield,
    Search,
    Download,
    Clock,
    FileCheck2,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

const KIND_STYLES: Record<string, { icon: React.ReactNode; label: string; color: string }> = {
    wasm_tool: { icon: <Puzzle className="w-4 h-4" />, label: 'WASM Tool', color: 'bg-blue-500/15 text-blue-400 border-blue-500/20' },
    wasm_channel: { icon: <Globe className="w-4 h-4" />, label: 'WASM Channel', color: 'bg-purple-500/15 text-purple-400 border-purple-500/20' },
    mcp_server: { icon: <Plug className="w-4 h-4" />, label: 'MCP Server', color: 'bg-green-500/15 text-green-400 border-green-500/20' },
};

const STATUS_STYLES: Record<string, { color: string; label: string; icon: React.ReactNode }> = {
    active: { color: 'text-green-400', label: 'Active', icon: <CheckCircle2 className="w-3.5 h-3.5" /> },
    configured: { color: 'text-blue-400', label: 'Configured', icon: <Wrench className="w-3.5 h-3.5" /> },
    installed: { color: 'text-amber-400', label: 'Installed', icon: <Package className="w-3.5 h-3.5" /> },
    pairing: { color: 'text-purple-400', label: 'Pairing', icon: <Plug className="w-3.5 h-3.5" /> },
    failed: { color: 'text-red-400', label: 'Failed', icon: <XCircle className="w-3.5 h-3.5" /> },
};

function ExtensionCard({
    ext,
    onActivate,
    onRemove,
    channelHealth,
}: {
    ext: openclaw.ExtensionInfoItem;
    onActivate: (name: string) => void;
    onRemove: (name: string) => void;
    channelHealth?: openclaw.ChannelStatusEntry;
}) {
    const [expanded, setExpanded] = useState(false);
    const [activating, setActivating] = useState(false);
    const [removing, setRemoving] = useState(false);

    const kindStyle = KIND_STYLES[ext.kind] || {
        icon: <Package className="w-4 h-4" />,
        label: ext.kind,
        color: 'bg-white/5 text-muted-foreground border-white/10',
    };
    const statusStyle = ext.activation_status
        ? STATUS_STYLES[ext.activation_status] || STATUS_STYLES['installed']
        : ext.active
            ? STATUS_STYLES['active']
            : STATUS_STYLES['installed'];

    const handleActivate = async () => {
        setActivating(true);
        try {
            onActivate(ext.name);
        } finally {
            setTimeout(() => setActivating(false), 1000);
        }
    };

    const handleRemove = async () => {
        setRemoving(true);
        try {
            onRemove(ext.name);
        } finally {
            setTimeout(() => setRemoving(false), 1000);
        }
    };

    return (
        <motion.div
            layout
            className={cn(
                "rounded-2xl border transition-all duration-300",
                ext.active
                    ? "bg-primary/[0.03] border-primary/20 shadow-sm shadow-primary/5"
                    : "bg-white/[0.02] border-white/5",
                "hover:border-white/10"
            )}
        >
            <div className="p-5 flex items-start gap-4">
                {/* Icon */}
                <div className={cn(
                    "p-2.5 rounded-xl border transition-colors flex items-center justify-center",
                    ext.active
                        ? "bg-primary/10 border-primary/20 text-primary"
                        : "bg-white/5 border-white/10 text-muted-foreground"
                )}>
                    {kindStyle.icon}
                </div>

                {/* Content */}
                <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                        <h3 className="font-semibold text-sm">{ext.name}</h3>
                        <span className={cn(
                            'inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border',
                            kindStyle.color
                        )}>
                            {kindStyle.label}
                        </span>
                        <span className={cn(
                            'inline-flex items-center gap-1 text-[10px] font-medium',
                            statusStyle.color
                        )}>
                            {statusStyle.icon}
                            {statusStyle.label}
                        </span>
                        {channelHealth && (
                            <span className={cn(
                                'inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full text-[9px] font-bold uppercase tracking-wider border',
                                channelHealth.state === 'Running' ? 'text-emerald-400 bg-emerald-500/10 border-emerald-500/20' :
                                    channelHealth.state === 'Connecting' ? 'text-amber-400 bg-amber-500/10 border-amber-500/20' :
                                        channelHealth.state === 'Degraded' ? 'text-orange-400 bg-orange-500/10 border-orange-500/20' :
                                            channelHealth.state === 'Error' ? 'text-red-400 bg-red-500/10 border-red-500/20' :
                                                'text-zinc-400 bg-zinc-500/10 border-zinc-500/20'
                            )}>
                                <span className={cn("w-1.5 h-1.5 rounded-full",
                                    channelHealth.state === 'Running' ? 'bg-emerald-500' :
                                        channelHealth.state === 'Connecting' ? 'bg-amber-500 animate-pulse' :
                                            channelHealth.state === 'Degraded' ? 'bg-orange-500' :
                                                channelHealth.state === 'Error' ? 'bg-red-500' : 'bg-zinc-500'
                                )} />
                                {channelHealth.state}
                            </span>
                        )}
                    </div>

                    {ext.description && (
                        <p className="text-xs text-muted-foreground mt-1.5 line-clamp-2 leading-relaxed">
                            {ext.description}
                        </p>
                    )}

                    {ext.tools.length > 0 && (
                        <div className="mt-2 flex flex-wrap gap-1">
                            {ext.tools.slice(0, 5).map(tool => (
                                <span
                                    key={tool}
                                    className="px-2 py-0.5 rounded-md text-[10px] font-mono bg-white/5 border border-white/5 text-muted-foreground"
                                >
                                    {tool}
                                </span>
                            ))}
                            {ext.tools.length > 5 && (
                                <span className="px-2 py-0.5 rounded-md text-[10px] font-mono text-muted-foreground/50">
                                    +{ext.tools.length - 5} more
                                </span>
                            )}
                        </div>
                    )}

                    {ext.activation_error && (
                        <div className="mt-2 flex items-start gap-1.5 text-[10px] text-red-400 font-medium">
                            <XCircle className="w-3 h-3 mt-0.5 flex-none" />
                            <span className="line-clamp-2">{ext.activation_error}</span>
                        </div>
                    )}
                </div>

                {/* Actions */}
                <div className="flex items-center gap-1.5 flex-none">
                    {!ext.active && (
                        <button
                            onClick={handleActivate}
                            disabled={activating}
                            className="p-2 rounded-lg hover:bg-green-500/10 text-muted-foreground hover:text-green-400 transition-colors"
                            title="Activate"
                        >
                            {activating ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Power className="w-4 h-4" />}
                        </button>
                    )}
                    <button
                        onClick={handleRemove}
                        disabled={removing}
                        className="p-2 rounded-lg hover:bg-red-500/10 text-muted-foreground hover:text-red-400 transition-colors"
                        title="Remove"
                    >
                        {removing ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Trash2 className="w-4 h-4" />}
                    </button>
                    <button
                        onClick={() => setExpanded(!expanded)}
                        className="p-2 rounded-lg hover:bg-white/5 text-muted-foreground transition-colors"
                    >
                        <ChevronDown className={cn("w-4 h-4 transition-transform", expanded && "rotate-180")} />
                    </button>
                </div>
            </div>

            <AnimatePresence>
                {expanded && (
                    <motion.div
                        initial={{ opacity: 0, height: 0 }}
                        animate={{ opacity: 1, height: 'auto' }}
                        exit={{ opacity: 0, height: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="px-5 pb-5 pt-0 border-t border-white/5">
                            <div className="mt-4 grid grid-cols-3 gap-3">
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1 flex items-center gap-1">
                                        <Shield className="w-3 h-3" />
                                        Auth
                                    </div>
                                    <p className={cn(
                                        "text-sm font-medium",
                                        ext.authenticated ? "text-green-400" : "text-amber-400"
                                    )}>
                                        {ext.authenticated ? 'Authenticated' : 'Not Authenticated'}
                                    </p>
                                </div>
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1 flex items-center gap-1">
                                        <Wrench className="w-3 h-3" />
                                        Setup
                                    </div>
                                    <p className={cn(
                                        "text-sm font-medium",
                                        ext.needs_setup ? "text-amber-400" : "text-green-400"
                                    )}>
                                        {ext.needs_setup ? 'Needs Setup' : 'Ready'}
                                    </p>
                                </div>
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1 flex items-center gap-1">
                                        <Puzzle className="w-3 h-3" />
                                        Tools
                                    </div>
                                    <p className="text-sm font-mono font-medium">
                                        {ext.tools.length}
                                    </p>
                                </div>
                            </div>
                            {ext.tools.length > 5 && (
                                <div className="mt-3">
                                    <div className="text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-2">All Tools</div>
                                    <div className="flex flex-wrap gap-1">
                                        {ext.tools.map(tool => (
                                            <span
                                                key={tool}
                                                className="px-2 py-0.5 rounded-md text-[10px] font-mono bg-white/5 border border-white/5 text-muted-foreground"
                                            >
                                                {tool}
                                            </span>
                                        ))}
                                    </div>
                                </div>
                            )}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}

export function OpenClawPlugins() {
    const [extensions, setExtensions] = useState<openclaw.ExtensionInfoItem[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [activeTab, setActiveTab] = useState<'installed' | 'clawhub' | 'lifecycle'>('installed');

    // ClawHub state
    const [hubQuery, setHubQuery] = useState('');
    const [hubResults, setHubResults] = useState<openclaw.ClawHubEntry[]>([]);
    const [hubSearching, setHubSearching] = useState(false);

    // Lifecycle state
    const [lifecycleEvents, setLifecycleEvents] = useState<openclaw.LifecycleEventItem[]>([]);
    const [lifecycleLoading, setLifecycleLoading] = useState(false);

    // Manifest validation
    const [validating, setValidating] = useState<string | null>(null);
    const [validationResult, setValidationResult] = useState<openclaw.ManifestValidation | null>(null);

    // Channel health
    const [channelStatuses, setChannelStatuses] = useState<openclaw.ChannelStatusEntry[]>([]);

    const fetchChannelHealth = useCallback(async () => {
        try {
            const statuses = await openclaw.getChannelStatusList();
            setChannelStatuses(statuses);
        } catch { /* silently fail — health badges are supplementary */ }
    }, []);

    useEffect(() => { fetchChannelHealth(); }, [fetchChannelHealth]);

    const fetchExtensions = async () => {
        try {
            const data = await openclaw.listExtensions();
            setExtensions(data.extensions || []);
        } catch (e) {
            console.error('Failed to fetch extensions:', e);
            toast.error('Failed to load extensions');
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchExtensions();
    }, []);

    const handleActivate = async (name: string) => {
        try {
            const resp = await openclaw.activateExtension(name);
            if (resp.ok) {
                toast.success(`Activated ${name}`);
            } else {
                toast.error(resp.message || `Failed to activate ${name}`);
            }
            fetchExtensions();
        } catch (e) {
            toast.error(`Activation error: ${e}`);
        }
    };

    const handleRemove = async (name: string) => {
        try {
            const resp = await openclaw.removeExtension(name);
            if (resp.ok) {
                toast.success(`Removed ${name}`);
            } else {
                toast.error(resp.message || `Failed to remove ${name}`);
            }
            fetchExtensions();
        } catch (e) {
            toast.error(`Remove error: ${e}`);
        }
    };

    const handleSearchClawHub = async () => {
        if (!hubQuery.trim()) return;
        setHubSearching(true);
        try {
            const result = await openclaw.searchClawHub(hubQuery.trim());
            setHubResults(result.entries || []);
        } catch (e) {
            toast.error(`ClawHub search failed: ${e}`);
        } finally {
            setHubSearching(false);
        }
    };

    const handleInstallFromHub = async (pluginId: string) => {
        try {
            await openclaw.installFromClawHub(pluginId);
            toast.success(`Installed ${pluginId}`);
            fetchExtensions();
        } catch (e) {
            toast.error(`Install failed: ${e}`);
        }
    };

    const fetchLifecycle = useCallback(async () => {
        setLifecycleLoading(true);
        try {
            const events = await openclaw.getPluginLifecycleList();
            setLifecycleEvents(events);
        } catch (e) {
            console.error('Failed to fetch lifecycle events:', e);
        } finally {
            setLifecycleLoading(false);
        }
    }, []);

    useEffect(() => {
        if (activeTab === 'lifecycle') fetchLifecycle();
    }, [activeTab, fetchLifecycle]);

    const handleValidateManifest = async (pluginId: string) => {
        setValidating(pluginId);
        try {
            const result = await openclaw.validateManifest(pluginId);
            setValidationResult(result);
            if (result.errors.length === 0 && result.warnings.length === 0) {
                toast.success('Manifest is valid');
            } else {
                toast.warning(`${result.errors.length} errors, ${result.warnings.length} warnings`);
            }
        } catch (e) {
            toast.error(`Validation failed: ${e}`);
        } finally {
            setValidating(null);
        }
    };

    const activeCount = extensions.filter(e => e.active).length;

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 flex flex-col h-full overflow-hidden"
        >
            <div className="p-8 pb-4 space-y-6 flex-none max-w-5xl w-full mx-auto">
                <div className="flex items-center justify-between gap-4 flex-wrap">
                    <div>
                        <h1 className="text-3xl font-bold tracking-tight">Extensions & Plugins</h1>
                        <p className="text-muted-foreground mt-1">
                            Manage WASM tools, channels, and MCP servers that extend agent capabilities.
                        </p>
                    </div>

                    <div className="flex items-center gap-3">
                        <div className="px-4 py-2 rounded-xl bg-primary/10 border border-primary/20 text-primary flex items-center gap-2 text-sm font-bold shadow-lg shadow-primary/5">
                            <Plug className="w-4 h-4" />
                            {activeCount} / {extensions.length} active
                        </div>
                        <button
                            onClick={() => {
                                setIsLoading(true);
                                fetchExtensions();
                            }}
                            className="p-2.5 rounded-xl bg-card border border-white/10 hover:bg-white/5 transition-colors shadow-sm"
                        >
                            <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                        </button>
                    </div>
                </div>

                {/* Tabs */}
                <div className="flex items-center gap-1 p-1 rounded-xl bg-white/[0.03] border border-white/5 w-fit">
                    {(['installed', 'clawhub', 'lifecycle'] as const).map(tab => (
                        <button
                            key={tab}
                            onClick={() => setActiveTab(tab)}
                            className={cn(
                                "px-4 py-1.5 rounded-lg text-xs font-medium transition-all",
                                activeTab === tab
                                    ? "bg-primary/15 text-primary"
                                    : "text-muted-foreground hover:text-foreground"
                            )}
                        >
                            {tab === 'installed' ? `Installed (${extensions.length})` : tab === 'clawhub' ? 'ClawHub Browser' : 'Lifecycle'}
                        </button>
                    ))}
                </div>

                {/* Extension kind summary */}
                {activeTab === 'installed' && extensions.length > 0 && (
                    <div className="flex flex-wrap gap-2">
                        {Object.entries(
                            extensions.reduce((acc, e) => {
                                acc[e.kind] = (acc[e.kind] || 0) + 1;
                                return acc;
                            }, {} as Record<string, number>)
                        )
                            .sort(([, a], [, b]) => b - a)
                            .map(([kind, count]) => {
                                const style = KIND_STYLES[kind] || {
                                    icon: <Package className="w-3.5 h-3.5" />,
                                    label: kind,
                                    color: 'bg-white/5 text-muted-foreground border-white/10',
                                };
                                return (
                                    <div
                                        key={kind}
                                        className={cn(
                                            "inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium border",
                                            style.color
                                        )}
                                    >
                                        {style.icon}
                                        {style.label}
                                        <span className="font-bold ml-0.5">× {count}</span>
                                    </div>
                                );
                            })}
                    </div>
                )}
            </div>

            <div className="flex-1 overflow-y-auto px-8 pb-8 scrollbar-hide">
                <div className="max-w-5xl mx-auto space-y-3">
                    {/* ═══ Installed Tab ═══ */}
                    {activeTab === 'installed' && (
                        <>
                            {isLoading && extensions.length === 0 ? (
                                <div className="space-y-3">
                                    {[1, 2, 3].map(i => (
                                        <div key={i} className="h-28 rounded-2xl border border-white/5 bg-white/[0.02] animate-pulse" />
                                    ))}
                                </div>
                            ) : extensions.length > 0 ? (
                                <AnimatePresence mode="popLayout">
                                    {extensions.map(ext => {
                                        const health = channelStatuses.find(
                                            ch => ch.id === ext.name.toLowerCase() || ch.name.toLowerCase() === ext.name.toLowerCase()
                                        );
                                        return (
                                            <div key={ext.name} className="space-y-2">
                                                <ExtensionCard
                                                    ext={ext}
                                                    onActivate={handleActivate}
                                                    onRemove={handleRemove}
                                                    channelHealth={health}
                                                />
                                                {/* Validate manifest button */}
                                                <div className="flex items-center gap-2 pl-4">
                                                    <button
                                                        onClick={() => handleValidateManifest(ext.name)}
                                                        disabled={validating === ext.name}
                                                        className="flex items-center gap-1.5 text-[10px] font-medium text-muted-foreground hover:text-foreground transition-colors"
                                                    >
                                                        {validating === ext.name ? (
                                                            <RefreshCw className="w-3 h-3 animate-spin" />
                                                        ) : (
                                                            <FileCheck2 className="w-3 h-3" />
                                                        )}
                                                        Validate Manifest
                                                    </button>
                                                </div>
                                            </div>
                                        );
                                    })}
                                </AnimatePresence>
                            ) : (
                                <div className="py-20 flex flex-col items-center justify-center text-center space-y-4">
                                    <div className="p-4 rounded-full bg-white/5 border border-white/10">
                                        <Package className="w-8 h-8 text-muted-foreground" />
                                    </div>
                                    <div>
                                        <h3 className="text-lg font-semibold">No extensions installed</h3>
                                        <p className="text-sm text-muted-foreground mt-1">
                                            Browse ClawHub or install extensions via URL.
                                        </p>
                                    </div>
                                </div>
                            )}

                            {/* Validation result display */}
                            {validationResult && (
                                <motion.div
                                    initial={{ opacity: 0, y: -5 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    className="p-4 rounded-xl border border-white/10 bg-card/30 space-y-2"
                                >
                                    <h4 className="text-xs font-bold uppercase tracking-widest text-muted-foreground">Manifest Validation</h4>
                                    {validationResult.errors.length > 0 && (
                                        <div className="space-y-1">
                                            {validationResult.errors.map((err, i) => (
                                                <p key={i} className="text-xs text-red-400 flex items-center gap-1.5">
                                                    <XCircle className="w-3 h-3" /> {err}
                                                </p>
                                            ))}
                                        </div>
                                    )}
                                    {validationResult.warnings.length > 0 && (
                                        <div className="space-y-1">
                                            {validationResult.warnings.map((w, i) => (
                                                <p key={i} className="text-xs text-amber-400 flex items-center gap-1.5">
                                                    <AlertCircle className="w-3 h-3" /> {w}
                                                </p>
                                            ))}
                                        </div>
                                    )}
                                    {validationResult.errors.length === 0 && validationResult.warnings.length === 0 && (
                                        <p className="text-xs text-emerald-400 flex items-center gap-1.5">
                                            <CheckCircle2 className="w-3 h-3" /> Manifest is valid — no issues found
                                        </p>
                                    )}
                                    <button
                                        onClick={() => setValidationResult(null)}
                                        className="text-[10px] text-muted-foreground hover:text-foreground"
                                    >
                                        Dismiss
                                    </button>
                                </motion.div>
                            )}
                        </>
                    )}

                    {/* ═══ ClawHub Browser Tab ═══ */}
                    {activeTab === 'clawhub' && (
                        <div className="space-y-4">
                            <div className="flex items-center gap-2">
                                <div className="flex-1 relative">
                                    <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
                                    <input
                                        value={hubQuery}
                                        onChange={(e) => setHubQuery(e.target.value)}
                                        onKeyDown={(e) => e.key === 'Enter' && handleSearchClawHub()}
                                        placeholder="Search ClawHub for plugins..."
                                        className="w-full pl-10 pr-4 py-2.5 rounded-xl bg-white/[0.03] border border-white/10 text-sm outline-none focus:ring-1 focus:ring-primary/30 placeholder:text-muted-foreground/50"
                                    />
                                </div>
                                <button
                                    onClick={handleSearchClawHub}
                                    disabled={hubSearching}
                                    className="px-4 py-2.5 rounded-xl bg-primary/15 text-primary text-xs font-bold border border-primary/20 hover:bg-primary/20 transition-all"
                                >
                                    {hubSearching ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : 'Search'}
                                </button>
                            </div>

                            {hubResults.length === 0 && !hubSearching ? (
                                <div className="py-16 text-center space-y-2">
                                    <Globe className="w-8 h-8 text-muted-foreground/30 mx-auto" />
                                    <p className="text-sm text-muted-foreground">Search ClawHub to discover plugins</p>
                                </div>
                            ) : (
                                <div className="space-y-3">
                                    {hubResults.map(entry => (
                                        <motion.div
                                            key={entry.id}
                                            initial={{ opacity: 0, y: 5 }}
                                            animate={{ opacity: 1, y: 0 }}
                                            className="p-4 rounded-2xl border border-white/10 bg-card/30 flex items-start gap-4"
                                        >
                                            <div className="p-2 rounded-lg bg-primary/10 border border-primary/20">
                                                <Puzzle className="w-5 h-5 text-primary" />
                                            </div>
                                            <div className="flex-1 min-w-0">
                                                <h3 className="font-semibold">{entry.name}</h3>
                                                <p className="text-xs text-muted-foreground mt-0.5">{entry.description}</p>
                                                <div className="flex items-center gap-3 mt-2 text-[10px] text-muted-foreground">
                                                    <span>v{entry.version}</span>
                                                    <span>by {entry.author}</span>
                                                    <span>{entry.install_count} installs</span>
                                                </div>
                                                {entry.tags.length > 0 && (
                                                    <div className="flex gap-1 mt-2">
                                                        {entry.tags.slice(0, 4).map(tag => (
                                                            <span key={tag} className="px-1.5 py-0.5 rounded text-[9px] bg-white/5 text-muted-foreground border border-white/5">
                                                                {tag}
                                                            </span>
                                                        ))}
                                                    </div>
                                                )}
                                            </div>
                                            <button
                                                onClick={() => handleInstallFromHub(entry.id)}
                                                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-primary/15 text-primary text-xs font-medium border border-primary/20 hover:bg-primary/20 transition-all shrink-0"
                                            >
                                                <Download className="w-3.5 h-3.5" />
                                                Install
                                            </button>
                                        </motion.div>
                                    ))}
                                </div>
                            )}
                        </div>
                    )}

                    {/* ═══ Lifecycle Tab ═══ */}
                    {activeTab === 'lifecycle' && (
                        <div className="space-y-4">
                            {lifecycleLoading ? (
                                <div className="py-16 flex items-center justify-center">
                                    <RefreshCw className="w-5 h-5 animate-spin text-muted-foreground" />
                                </div>
                            ) : lifecycleEvents.length === 0 ? (
                                <div className="py-16 text-center space-y-2">
                                    <Clock className="w-8 h-8 text-muted-foreground/30 mx-auto" />
                                    <p className="text-sm text-muted-foreground">No lifecycle events recorded</p>
                                    <p className="text-xs text-muted-foreground/60">Events appear when plugins are installed, activated, or removed</p>
                                </div>
                            ) : (
                                <div className="relative">
                                    {/* Timeline line */}
                                    <div className="absolute left-6 top-0 bottom-0 w-px bg-white/10" />

                                    {lifecycleEvents.map((event, i) => {
                                        const typeColors: Record<string, string> = {
                                            installed: 'bg-blue-500',
                                            activated: 'bg-emerald-500',
                                            deactivated: 'bg-amber-500',
                                            removed: 'bg-red-500',
                                            error: 'bg-red-600',
                                        };
                                        const dotColor = typeColors[event.event_type] || 'bg-zinc-500';

                                        return (
                                            <motion.div
                                                key={`${event.timestamp}-${i}`}
                                                initial={{ opacity: 0, x: -10 }}
                                                animate={{ opacity: 1, x: 0 }}
                                                transition={{ delay: i * 0.03 }}
                                                className="relative pl-14 pb-6"
                                            >
                                                {/* Dot */}
                                                <div className={cn("absolute left-[19px] w-3 h-3 rounded-full border-2 border-background", dotColor)} />

                                                <div className="p-3 rounded-xl bg-white/[0.02] border border-white/5">
                                                    <div className="flex items-center justify-between">
                                                        <span className="font-medium text-sm">{event.plugin_id}</span>
                                                        <span className={cn(
                                                            "text-[10px] font-bold uppercase tracking-wider px-2 py-0.5 rounded",
                                                            event.event_type === 'error' ? 'bg-red-500/10 text-red-400' :
                                                                event.event_type === 'removed' ? 'bg-red-500/10 text-red-300' :
                                                                    event.event_type === 'activated' ? 'bg-emerald-500/10 text-emerald-400' :
                                                                        'bg-blue-500/10 text-blue-400'
                                                        )}>
                                                            {event.event_type}
                                                        </span>
                                                    </div>
                                                    {event.details && (
                                                        <p className="text-xs text-muted-foreground mt-1">{event.details}</p>
                                                    )}
                                                    <p className="text-[10px] text-muted-foreground/60 mt-1">{event.timestamp}</p>
                                                </div>
                                            </motion.div>
                                        );
                                    })}
                                </div>
                            )}
                        </div>
                    )}

                    {/* Info section */}
                    {activeTab === 'installed' && (
                        <div className="mt-8 p-6 rounded-2xl border bg-primary/5 border-primary/10 flex gap-4">
                            <div className="p-2 bg-primary/10 rounded-xl h-fit">
                                <AlertCircle className="w-5 h-5 text-primary" />
                            </div>
                            <div>
                                <h4 className="text-sm font-semibold text-primary uppercase tracking-wider">Extension Types</h4>
                                <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                                    <strong>WASM Tools</strong> add new tool capabilities to the agent. <strong>WASM Channels</strong> enable
                                    messaging integrations (Telegram, Slack). <strong>MCP Servers</strong> connect external tool providers
                                    via the Model Context Protocol.
                                </p>
                            </div>
                        </div>
                    )}
                </div>
            </div>
        </motion.div>
    );
}
