import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    DollarSign, TrendingUp, AlertTriangle, RefreshCw,
    Download, BarChart3, Cpu, Bot, Trash2, Zap, Hash
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

// ── Bar chart (inline SVG — no library dependency) ──────────────────
function MiniBar({ value, max, label, color }: { value: number; max: number; label: string; color: string }) {
    const pct = max > 0 ? Math.min(100, (value / max) * 100) : 0;
    return (
        <div className="flex items-center gap-3">
            <span className="text-[10px] text-muted-foreground w-28 truncate font-mono">{label}</span>
            <div className="flex-1 h-3 bg-white/[0.03] rounded-full overflow-hidden border border-white/5">
                <motion.div
                    initial={{ width: 0 }}
                    animate={{ width: `${pct}%` }}
                    transition={{ duration: 0.6, ease: 'easeOut' }}
                    className="h-full rounded-full"
                    style={{ background: color }}
                />
            </div>
            <span className="text-xs font-mono text-muted-foreground w-16 text-right">${value.toFixed(4)}</span>
        </div>
    );
}

// ── Format token count ──────────────────────────────────────────────
function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return n.toString();
}

export function OpenClawCostDashboard() {
    const [summary, setSummary] = useState<openclaw.CostSummary | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [showResetConfirm, setShowResetConfirm] = useState(false);
    const [activeTab, setActiveTab] = useState<'overview' | 'by-model' | 'by-agent'>('overview');

    const fetchData = useCallback(async () => {
        try {
            const data = await openclaw.getCostSummary();
            setSummary(data);
        } catch (e) {
            console.error('Failed to fetch cost summary:', e);
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        fetchData();
        const interval = setInterval(fetchData, 10000); // 10s for near-real-time
        return () => clearInterval(interval);
    }, [fetchData]);

    const handleExportCsv = async () => {
        try {
            const csv = await openclaw.exportCostCsv();
            await navigator.clipboard.writeText(csv);
            toast.success('Cost data copied to clipboard as CSV');
        } catch (e) {
            toast.error(`Export failed: ${e}`);
        }
    };

    const handleReset = async () => {
        try {
            await openclaw.resetCostData();
            toast.success('Cost data reset successfully');
            setShowResetConfirm(false);
            await fetchData();
        } catch (e) {
            toast.error(`Reset failed: ${e}`);
        }
    };

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center">
                <RefreshCw className="w-5 h-5 animate-spin text-muted-foreground" />
            </div>
        );
    }

    const totalCost = summary?.total_cost_usd ?? 0;
    const totalRequests = summary?.total_requests ?? 0;
    const totalInputTokens = summary?.total_input_tokens ?? 0;
    const totalOutputTokens = summary?.total_output_tokens ?? 0;
    const avgCost = summary?.avg_cost_per_request ?? 0;
    const dailyEntries = Object.entries(summary?.daily ?? {}).sort(([a], [b]) => b.localeCompare(a)).slice(0, 14);
    const monthlyEntries = Object.entries(summary?.monthly ?? {}).sort(([a], [b]) => b.localeCompare(a)).slice(0, 6);
    const modelEntries = Object.entries(summary?.by_model ?? {}).sort(([, a], [, b]) => b - a);
    const agentEntries = Object.entries(summary?.by_agent ?? {}).sort(([, a], [, b]) => b - a);
    const maxModel = modelEntries.length > 0 ? modelEntries[0][1] : 1;
    const maxAgent = agentEntries.length > 0 ? agentEntries[0][1] : 1;

    const modelColors = ['#a78bfa', '#818cf8', '#60a5fa', '#22d3ee', '#34d399', '#fbbf24', '#f87171'];
    const agentColors = ['#fb923c', '#f472b6', '#c084fc', '#22d3ee', '#10b981'];

    return (
        <motion.div
            className="flex-1 overflow-y-auto p-8 space-y-8"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-emerald-500/10 border border-emerald-500/20">
                        <DollarSign className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Cost Dashboard</h1>
                        <p className="text-xs text-muted-foreground">LLM usage spend across all agents and sessions</p>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        onClick={handleExportCsv}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                    >
                        <Download className="w-3.5 h-3.5" />
                        Export CSV
                    </button>
                    <button
                        onClick={() => setShowResetConfirm(true)}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-red-400/70 hover:text-red-400 bg-white/[0.03] hover:bg-red-500/10 border border-white/5 hover:border-red-500/20 transition-all"
                        title="Reset all cost data"
                    >
                        <Trash2 className="w-3.5 h-3.5" />
                        Reset
                    </button>
                    <button
                        onClick={fetchData}
                        className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                    >
                        <RefreshCw className={cn("w-3.5 h-3.5", isLoading && "animate-spin")} />
                    </button>
                </div>
            </div>

            {/* Reset confirmation modal */}
            <AnimatePresence>
                {showResetConfirm && (
                    <motion.div
                        initial={{ opacity: 0, y: -10 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: -10 }}
                        className="flex items-center justify-between gap-4 p-4 rounded-2xl bg-red-500/10 border border-red-500/20"
                    >
                        <div className="flex items-center gap-3">
                            <Trash2 className="w-5 h-5 text-red-400 shrink-0" />
                            <div>
                                <p className="text-sm font-semibold text-red-300">Reset all cost data?</p>
                                <p className="text-xs text-muted-foreground/70">
                                    This will permanently delete {totalRequests} entries totaling ${totalCost.toFixed(4)}. This cannot be undone.
                                </p>
                            </div>
                        </div>
                        <div className="flex items-center gap-2 shrink-0">
                            <button
                                onClick={() => setShowResetConfirm(false)}
                                className="px-3 py-1.5 rounded-lg text-xs font-medium text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                            >
                                Cancel
                            </button>
                            <button
                                onClick={handleReset}
                                className="px-3 py-1.5 rounded-lg text-xs font-medium text-red-300 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 transition-all"
                            >
                                Confirm Reset
                            </button>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Alert banner */}
            {summary?.alert_triggered && (
                <motion.div
                    initial={{ opacity: 0, y: -10 }}
                    animate={{ opacity: 1, y: 0 }}
                    className="flex items-center gap-3 p-4 rounded-2xl bg-amber-500/10 border border-amber-500/20"
                >
                    <AlertTriangle className="w-5 h-5 text-muted-foreground shrink-0" />
                    <div>
                        <p className="text-sm font-semibold text-amber-300">Cost Alert Triggered</p>
                        <p className="text-xs text-muted-foreground/70">
                            Spend has exceeded ${summary.alert_threshold_usd.toFixed(2)} threshold
                        </p>
                    </div>
                </motion.div>
            )}

            {/* Stat cards — 2 rows */}
            <div className="grid grid-cols-3 gap-4">
                {/* Row 1: Total, This Month, Alert */}
                <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-5">
                    <div className="flex items-center gap-2 mb-3">
                        <DollarSign className="w-4 h-4 text-primary" />
                        <span className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">Total Spend</span>
                    </div>
                    <p className="text-3xl font-bold text-primary tabular-nums">
                        ${totalCost.toFixed(4)}
                    </p>
                    <p className="text-[10px] text-muted-foreground mt-1">All time</p>
                </div>
                <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-5">
                    <div className="flex items-center gap-2 mb-3">
                        <TrendingUp className="w-4 h-4 text-blue-400" />
                        <span className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">This Month</span>
                    </div>
                    <p className="text-3xl font-bold text-blue-400 tabular-nums">
                        ${(monthlyEntries[0]?.[1] ?? 0).toFixed(4)}
                    </p>
                    <p className="text-[10px] text-muted-foreground mt-1">{monthlyEntries[0]?.[0] ?? 'No data'}</p>
                </div>
                <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-5">
                    <div className="flex items-center gap-2 mb-3">
                        <BarChart3 className="w-4 h-4 text-primary" />
                        <span className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">Alert Threshold</span>
                    </div>
                    <p className="text-3xl font-bold text-primary tabular-nums">
                        ${(summary?.alert_threshold_usd ?? 50).toFixed(2)}
                    </p>
                    <p className="text-[10px] text-muted-foreground mt-1">Daily limit</p>
                </div>

                {/* Row 2: Requests, Tokens, Avg Cost */}
                <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-5">
                    <div className="flex items-center gap-2 mb-3">
                        <Hash className="w-4 h-4 text-violet-400" />
                        <span className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">Total Requests</span>
                    </div>
                    <p className="text-3xl font-bold text-violet-400 tabular-nums">
                        {totalRequests.toLocaleString()}
                    </p>
                    <p className="text-[10px] text-muted-foreground mt-1">LLM API calls</p>
                </div>
                <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-5">
                    <div className="flex items-center gap-2 mb-3">
                        <Zap className="w-4 h-4 text-amber-400" />
                        <span className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">Total Tokens</span>
                    </div>
                    <p className="text-2xl font-bold text-amber-400 tabular-nums">
                        {formatTokens(totalInputTokens + totalOutputTokens)}
                    </p>
                    <p className="text-[10px] text-muted-foreground mt-1">
                        {formatTokens(totalInputTokens)} in · {formatTokens(totalOutputTokens)} out
                    </p>
                </div>
                <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-5">
                    <div className="flex items-center gap-2 mb-3">
                        <DollarSign className="w-4 h-4 text-emerald-400" />
                        <span className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">Avg per Request</span>
                    </div>
                    <p className="text-3xl font-bold text-emerald-400 tabular-nums">
                        ${avgCost.toFixed(4)}
                    </p>
                    <p className="text-[10px] text-muted-foreground mt-1">Mean cost / call</p>
                </div>
            </div>

            {/* Tabs */}
            <div className="flex items-center gap-1 p-1 rounded-xl bg-white/[0.03] border border-white/5 w-fit">
                {(['overview', 'by-model', 'by-agent'] as const).map(tab => (
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
                        {tab === 'overview' ? 'Daily Trend' : tab === 'by-model' ? 'By Model' : 'By Agent'}
                    </button>
                ))}
            </div>

            {/* Tab content */}
            <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-6">
                {activeTab === 'overview' && (
                    <div className="space-y-3">
                        <h3 className="text-sm font-bold text-muted-foreground flex items-center gap-2">
                            <TrendingUp className="w-4 h-4" />
                            Last 14 Days
                        </h3>
                        {dailyEntries.length === 0 ? (
                            <p className="text-xs text-muted-foreground text-center py-8">No cost data collected yet</p>
                        ) : (
                            <div className="space-y-2">
                                {dailyEntries.map(([date, cost]) => (
                                    <MiniBar
                                        key={date}
                                        label={date}
                                        value={cost}
                                        max={Math.max(...dailyEntries.map(([, v]) => v))}
                                        color="#34d399"
                                    />
                                ))}
                            </div>
                        )}
                    </div>
                )}

                {activeTab === 'by-model' && (
                    <div className="space-y-3">
                        <h3 className="text-sm font-bold text-muted-foreground flex items-center gap-2">
                            <Cpu className="w-4 h-4" />
                            Spend by Model
                        </h3>
                        {modelEntries.length === 0 ? (
                            <p className="text-xs text-muted-foreground text-center py-8">No per-model data yet</p>
                        ) : (
                            <div className="space-y-2">
                                {modelEntries.map(([model, cost], i) => (
                                    <MiniBar
                                        key={model}
                                        label={model}
                                        value={cost}
                                        max={maxModel}
                                        color={modelColors[i % modelColors.length]}
                                    />
                                ))}
                            </div>
                        )}
                    </div>
                )}

                {activeTab === 'by-agent' && (
                    <div className="space-y-3">
                        <h3 className="text-sm font-bold text-muted-foreground flex items-center gap-2">
                            <Bot className="w-4 h-4" />
                            Spend by Agent
                        </h3>
                        {agentEntries.length === 0 ? (
                            <p className="text-xs text-muted-foreground text-center py-8">No per-agent data yet</p>
                        ) : (
                            <div className="space-y-2">
                                {agentEntries.map(([agent, cost], i) => (
                                    <MiniBar
                                        key={agent}
                                        label={agent}
                                        value={cost}
                                        max={maxAgent}
                                        color={agentColors[i % agentColors.length]}
                                    />
                                ))}
                            </div>
                        )}
                    </div>
                )}
            </div>
        </motion.div>
    );
}
