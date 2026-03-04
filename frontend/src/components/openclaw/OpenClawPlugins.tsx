import { useState, useEffect } from 'react';
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
}: {
    ext: openclaw.ExtensionInfoItem;
    onActivate: (name: string) => void;
    onRemove: (name: string) => void;
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

                {/* Extension kind summary */}
                {extensions.length > 0 && (
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
                    {isLoading && extensions.length === 0 ? (
                        <div className="space-y-3">
                            {[1, 2, 3].map(i => (
                                <div key={i} className="h-28 rounded-2xl border border-white/5 bg-white/[0.02] animate-pulse" />
                            ))}
                        </div>
                    ) : extensions.length > 0 ? (
                        <AnimatePresence mode="popLayout">
                            {extensions.map(ext => (
                                <ExtensionCard
                                    key={ext.name}
                                    ext={ext}
                                    onActivate={handleActivate}
                                    onRemove={handleRemove}
                                />
                            ))}
                        </AnimatePresence>
                    ) : (
                        <div className="py-20 flex flex-col items-center justify-center text-center space-y-4">
                            <div className="p-4 rounded-full bg-white/5 border border-white/10">
                                <Package className="w-8 h-8 text-muted-foreground" />
                            </div>
                            <div>
                                <h3 className="text-lg font-semibold">No extensions installed</h3>
                                <p className="text-sm text-muted-foreground mt-1">
                                    Extensions can be installed from the Skills Registry or directly via URL.
                                </p>
                            </div>
                        </div>
                    )}

                    {/* Info section */}
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
                </div>
            </div>
        </motion.div>
    );
}
