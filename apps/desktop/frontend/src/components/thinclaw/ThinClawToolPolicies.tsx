import { useState, useEffect, useCallback, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Wrench, RefreshCw, Search, Shield, ChevronDown, ChevronRight,
    AlertTriangle, Cpu, HardDrive, Puzzle, Settings2, Package, Filter, ToggleLeft, ToggleRight
} from 'lucide-react';
import * as thinclawApi from '../../lib/thinclaw';
import { ConfirmDialog, Notice } from '../ui';

const SOURCE_CONFIG: Record<string, { icon: typeof Wrench; color: string; label: string }> = {
    builtin: { icon: Cpu, color: 'text-blue-400', label: 'Built-in' },
    container: { icon: HardDrive, color: 'text-muted-foreground', label: 'Container' },
    memory: { icon: Settings2, color: 'text-primary', label: 'Memory' },
    management: { icon: Package, color: 'text-primary', label: 'Management' },
    extension: { icon: Puzzle, color: 'text-primary', label: 'Extension' },
};

const RISK_REASON_COPY: Record<string, string> = {
    command_execution: 'execute commands or processes',
    network_egress: 'send requests or transfer data over the network',
    browser_control: 'control a browser or interact with web pages',
    filesystem_mutation: 'create, edit, move, or delete files',
    deploy_or_publish: 'install, deploy, commit, or publish changes',
    secrets_or_messaging: 'access credentials or send messages externally',
    read_only: 'read or inspect information without making changes',
    unknown: 'perform actions that are not classified as read-only',
};

function describeRisk(tool: thinclawApi.ToolInfoItem): string {
    const reasons = tool.risk_reasons
        .map(reason => RISK_REASON_COPY[reason] ?? reason.split('_').join(' '));
    return reasons.length > 0
        ? reasons.join('; ')
        : 'perform actions that are not classified as read-only';
}

export function ThinClawToolPolicies() {
    const [tools, setTools] = useState<thinclawApi.ToolInfoItem[]>([]);
    const [loading, setLoading] = useState(true);
    const [toggling, setToggling] = useState<string | null>(null);
    const [search, setSearch] = useState('');
    const [expandedTool, setExpandedTool] = useState<string | null>(null);
    const [filterSource, setFilterSource] = useState<string | null>(null);
    const [showFilters, setShowFilters] = useState(false);
    const [pendingEnable, setPendingEnable] = useState<thinclawApi.ToolInfoItem | null>(null);
    const [error, setError] = useState<string | null>(null);
    const mutationInFlight = useRef(false);
    const enableTrigger = useRef<HTMLButtonElement | null>(null);

    const loadTools = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const resp = await thinclawApi.listTools();
            setTools(resp.tools || []);
        } catch (e) {
            console.error('Failed to list tools', e);
            setTools([]);
            setError(e instanceof Error ? e.message : 'Failed to load tool policies.');
        } finally {
            setLoading(false);
        }
    }, []);

    const applyToggle = async (tool: thinclawApi.ToolInfoItem): Promise<boolean> => {
        if (mutationInFlight.current) return false;
        mutationInFlight.current = true;
        setError(null);
        setToggling(tool.name);
        try {
            const enabled = await thinclawApi.toggleTool(tool.name, tool.enabled);
            setTools(prev => prev.map(item => item.name === tool.name ? { ...item, enabled } : item));
            return true;
        } catch (e) {
            console.error('Failed to toggle tool', e);
            setError(e instanceof Error ? e.message : `Failed to update ${tool.name}.`);
            return false;
        } finally {
            mutationInFlight.current = false;
            setToggling(null);
        }
    };

    const closePendingEnable = () => {
        const trigger = enableTrigger.current;
        setPendingEnable(null);
        window.setTimeout(() => trigger?.focus(), 0);
    };

    const handleToggle = (e: React.MouseEvent<HTMLButtonElement>, tool: thinclawApi.ToolInfoItem) => {
        e.stopPropagation();
        if (mutationInFlight.current) return;
        if (!tool.enabled && tool.risk !== 'low') {
            enableTrigger.current = e.currentTarget;
            setPendingEnable(tool);
            return;
        }
        void applyToggle(tool);
    };

    const confirmEnable = async () => {
        if (!pendingEnable) return;
        if (await applyToggle(pendingEnable)) closePendingEnable();
    };

    useEffect(() => { loadTools(); }, [loadTools]);

    const sources = [...new Set(tools.map(t => t.source))];

    const filtered = tools.filter(t => {
        if (filterSource && t.source !== filterSource) return false;
        if (search) {
            const s = search.toLowerCase();
            return t.name.toLowerCase().includes(s) || t.description.toLowerCase().includes(s);
        }
        return true;
    });

    // Group by source
    const grouped = filtered.reduce((acc, t) => {
        if (!acc[t.source]) acc[t.source] = [];
        acc[t.source].push(t);
        return acc;
    }, {} as Record<string, thinclawApi.ToolInfoItem[]>);

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Header */}
            <div className="shrink-0 px-5 pt-5 pb-3">
                <div className="flex items-center justify-between mb-4">
                    <div className="flex items-center gap-3">
                        <div className="w-9 h-9 rounded-xl bg-linear-to-br from-indigo-500/20 to-violet-500/20 border border-indigo-500/30 flex items-center justify-center">
                            <Shield className="w-4.5 h-4.5 text-primary" />
                        </div>
                        <div>
                            <h2 className="text-base font-semibold text-foreground">Tool Policies</h2>
                            <p className="text-xs text-muted-foreground/60">{tools.length} tools registered</p>
                        </div>
                    </div>
                    <div className="flex items-center gap-2">
                        <button
                            onClick={() => setShowFilters(!showFilters)}
                            className={`p-2 rounded-lg border transition-all ${showFilters || filterSource ? 'bg-indigo-500/10 border-indigo-500/30 text-primary' : 'bg-muted/30 border-border/40 text-muted-foreground hover:text-foreground'
                                }`}
                        >
                            <Filter className="w-3.5 h-3.5" />
                        </button>
                        <button
                            onClick={loadTools}
                            className="p-2 rounded-lg bg-muted/30 border border-border/40 text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-all"
                        >
                            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
                        </button>
                    </div>
                </div>

                {/* Search */}
                <div className="relative mb-2">
                    <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/60" />
                    <input
                        type="text"
                        value={search}
                        onChange={e => setSearch(e.target.value)}
                        placeholder="Search tools..."
                        className="w-full pl-9 pr-3 py-2 rounded-lg bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-hidden focus:border-indigo-500/50"
                    />
                </div>

                {error && (
                    <Notice tone="error" title="Tool policy update failed" className="mb-2">
                        {error}
                    </Notice>
                )}

                {/* Source Filters */}
                <AnimatePresence>
                    {showFilters && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: 'auto', opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            className="overflow-hidden"
                        >
                            <div className="flex flex-wrap gap-1.5 mb-2">
                                <button
                                    onClick={() => setFilterSource(null)}
                                    className={`px-2.5 py-1 rounded text-xs border transition-all ${!filterSource ? 'bg-indigo-500/10 border-indigo-500/30 text-indigo-500 dark:text-indigo-300' : 'bg-muted/30 border-border/40 text-muted-foreground/60 hover:text-foreground/70'
                                        }`}
                                >
                                    All ({tools.length})
                                </button>
                                {sources.map(src => {
                                    const cfg = SOURCE_CONFIG[src] || SOURCE_CONFIG.extension;
                                    const count = tools.filter(t => t.source === src).length;
                                    return (
                                        <button
                                            key={src}
                                            onClick={() => setFilterSource(filterSource === src ? null : src)}
                                            className={`px-2.5 py-1 rounded text-xs border transition-all flex items-center gap-1.5 ${filterSource === src ? `${cfg.color} bg-muted/30 border-border/40` : 'text-muted-foreground/60 bg-transparent border-transparent hover:text-foreground/70'
                                                }`}
                                        >
                                            <cfg.icon className="w-3 h-3" />
                                            {cfg.label} ({count})
                                        </button>
                                    );
                                })}
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>

            {/* Tools List */}
            <div className="flex-1 overflow-y-auto px-5 pb-5 space-y-4">
                {loading ? (
                    <div className="flex items-center justify-center py-16 text-muted-foreground/60">
                        <RefreshCw className="w-5 h-5 animate-spin mr-2" />
                        Loading tools...
                    </div>
                ) : filtered.length === 0 ? (
                    <div className="flex flex-col items-center justify-center py-16 text-muted-foreground/60">
                        <Wrench className="w-8 h-8 mb-3 opacity-30" />
                        <p className="text-sm">{search ? 'No matching tools' : 'No tools registered'}</p>
                    </div>
                ) : (
                    Object.entries(grouped).map(([source, sourceTools]) => {
                        const cfg = SOURCE_CONFIG[source] || SOURCE_CONFIG.extension;
                        return (
                            <div key={source}>
                                <div className="flex items-center gap-2 mb-2">
                                    <cfg.icon className={`w-3.5 h-3.5 ${cfg.color}`} />
                                    <span className={`text-xs font-semibold uppercase tracking-wider ${cfg.color}`}>
                                        {cfg.label}
                                    </span>
                                    <span className="text-[10px] text-muted-foreground/60">({sourceTools.length})</span>
                                </div>
                                <div className="space-y-1">
                                    {sourceTools.map((tool, i) => {
                                        const expanded = expandedTool === tool.name;
                                        return (
                                            <motion.div
                                                key={tool.name}
                                                initial={{ opacity: 0, y: 5 }}
                                                animate={{ opacity: 1, y: 0 }}
                                                transition={{ delay: i * 0.02 }}
                                                className="rounded-lg bg-muted/10 border border-border/30 hover:border-border/50 transition-all cursor-pointer"
                                                onClick={() => setExpandedTool(expanded ? null : tool.name)}
                                            >
                                                <div className="flex items-center gap-3 px-3 py-2">
                                                    {expanded
                                                        ? <ChevronDown className="w-3 h-3 text-muted-foreground/60" />
                                                        : <ChevronRight className="w-3 h-3 text-muted-foreground/60" />
                                                    }
                                                    <span className="text-sm font-mono text-foreground flex-1">
                                                        {tool.name}
                                                    </span>
                                                    {tool.risk !== 'low' && (
                                                        <span className="flex items-center gap-1 rounded-full border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-[10px] font-semibold text-amber-800 dark:text-amber-300">
                                                            <AlertTriangle className="size-3" aria-hidden="true" />
                                                            High risk
                                                        </span>
                                                    )}
                                                    {/* Toggle switch */}
                                                    <button
                                                        onClick={(e) => handleToggle(e, tool)}
                                                        disabled={toggling !== null}
                                                        aria-label={`${tool.enabled ? 'Disable' : 'Enable'} ${tool.name}`}
                                                        title={tool.enabled ? 'Click to disable' : 'Click to enable'}
                                                        className={`flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-bold transition-all border ${tool.enabled
                                                                ? 'border-emerald-500/30 bg-emerald-500/10 text-primary hover:bg-emerald-500/20'
                                                                : 'border-border/30 bg-muted/20 text-muted-foreground/60 hover:bg-muted/40'
                                                            } ${toggling === tool.name ? 'opacity-50 cursor-wait' : 'cursor-pointer'}`}
                                                    >
                                                        {tool.enabled
                                                            ? <ToggleRight className="w-3 h-3" />
                                                            : <ToggleLeft className="w-3 h-3" />
                                                        }
                                                        {tool.enabled ? 'ON' : 'OFF'}
                                                    </button>
                                                </div>
                                                <AnimatePresence>
                                                    {expanded && (
                                                        <motion.div
                                                            initial={{ height: 0, opacity: 0 }}
                                                            animate={{ height: 'auto', opacity: 1 }}
                                                            exit={{ height: 0, opacity: 0 }}
                                                            className="overflow-hidden"
                                                        >
                                                            <div className="px-3 pb-3 pt-1 border-t border-border/20">
                                                                <p className="text-xs text-muted-foreground leading-relaxed">
                                                                    {tool.description || 'No description available'}
                                                                </p>
                                                                <div className="flex items-center gap-3 mt-2 text-[10px] text-muted-foreground/60">
                                                                    <span>Source: {cfg.label}</span>
                                                                    <span>Status: {tool.enabled ? 'Active' : 'Disabled'}</span>
                                                                    <span>Risk: {tool.risk === 'low' ? 'Low' : 'High'}</span>
                                                                </div>
                                                                <p className="mt-2 text-[10px] text-muted-foreground/70">
                                                                    Risk basis: {describeRisk(tool)}.
                                                                </p>
                                                            </div>
                                                        </motion.div>
                                                    )}
                                                </AnimatePresence>
                                            </motion.div>
                                        );
                                    })}
                                </div>
                            </div>
                        );
                    })
                )}
            </div>
            <ConfirmDialog
                open={pendingEnable !== null}
                onOpenChange={(open) => {
                    if (!open && !mutationInFlight.current) closePendingEnable();
                }}
                title={pendingEnable ? `Enable high-risk tool ${pendingEnable.name}?` : 'Enable high-risk tool?'}
                description={pendingEnable ? (
                    <>
                        <span className="font-mono text-content-primary">{pendingEnable.name}</span>
                        {' '}from {SOURCE_CONFIG[pendingEnable.source]?.label ?? pendingEnable.source} can {describeRisk(pendingEnable)}.
                        Enabling it restores this capability to agent executions until you disable it again.
                    </>
                ) : ''}
                confirmLabel="Enable high-risk tool"
                onConfirm={confirmEnable}
                isConfirming={pendingEnable !== null && toggling === pendingEnable.name}
                tone="warning"
            />
        </div>
    );
}
