/**
 * OpenClawPresence — Agent Runtime Inspector
 *
 * Displays live runtime metrics from the embedded IronClaw engine:
 * sessions, sub-agents, tools, hooks, channels, uptime, and routine engine state.
 * Polls every 5 seconds for live updates.
 *
 * Styling: Uses the app theme system (--primary, --muted-foreground, etc.)
 * for all accent colors. Only semantic green/red for online/offline status.
 */

import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Activity,
    Cpu,
    Clock,
    Layers,
    Wrench,
    GitBranch,
    Radio,
    Zap,
    RefreshCw,
    Circle,
    Terminal,
    Users,
    CheckCircle2,
    XCircle,
    Timer,
    ChevronRight,
    BarChart3,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';

// ── Helpers ──────────────────────────────────────────────────────────

function formatUptime(secs: number | null): string {
    if (secs === null) return '—';
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h ${m}m`;
}

// ── Stat Card ────────────────────────────────────────────────────────

interface StatCardProps {
    icon: React.ElementType;
    label: string;
    value: string | number;
    sub?: string;
    /** Theme-derived chart color class, e.g. 'text-chart-1'. Falls back to 'text-foreground'. */
    color?: string;
    /** Only set for semantic status: 'success' or 'error'. Overrides color. */
    status?: 'success' | 'error';
    pulse?: boolean;
}

function StatCard({ icon: Icon, label, value, sub, color, status, pulse }: StatCardProps) {
    // Semantic status overrides chart color
    const valueColor = status === 'success' ? 'text-green-500' : status === 'error' ? 'text-red-500' : (color || 'text-foreground');
    const iconColor = status === 'success' ? 'text-green-500' : status === 'error' ? 'text-red-500' : (color || 'text-primary');

    return (
        <motion.div
            initial={{ opacity: 0, scale: 0.97 }}
            animate={{ opacity: 1, scale: 1 }}
            className="p-5 rounded-2xl bg-card/40 backdrop-blur-md border-none hover:bg-card/60 transition-all"
        >
            <div className="flex items-start justify-between mb-4">
                <div className={cn('p-2.5 rounded-xl bg-primary/10', iconColor)}>
                    <Icon className="w-5 h-5" />
                </div>
                {pulse && (
                    <div className="flex items-center gap-1.5">
                        <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
                        <span className="text-[10px] font-bold text-green-500 uppercase tracking-widest">Live</span>
                    </div>
                )}
            </div>
            <div className="space-y-0.5">
                <div className={cn('text-3xl font-bold tracking-tight', valueColor)}>{value}</div>
                <div className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{label}</div>
                {sub && <div className="text-[10px] text-muted-foreground/60 mt-1">{sub}</div>}
            </div>
        </motion.div>
    );
}

// ── Sub-agent row ────────────────────────────────────────────────────

interface SubAgentRowProps {
    info: openclaw.ChildSessionInfo;
}

function SubAgentRow({ info }: SubAgentRowProps) {
    // Semantic status colors only
    const statusColor =
        info.status === 'completed'
            ? 'text-green-500 bg-green-500/10'
            : info.status === 'failed'
                ? 'text-red-500 bg-red-500/10'
                : 'text-muted-foreground bg-primary/5';

    return (
        <div className="flex items-center gap-3 px-4 py-3 rounded-xl bg-white/[0.02] hover:bg-white/[0.04] transition-colors">
            <GitBranch className="w-3.5 h-3.5 text-muted-foreground/50 flex-shrink-0" />
            <div className="flex-1 min-w-0">
                <p className="text-xs font-medium text-foreground/80 truncate">{info.task || info.session_key}</p>
                <p className="text-[10px] font-mono text-muted-foreground/50 truncate">{info.session_key}</p>
            </div>
            <span className={cn('px-2 py-0.5 rounded-full text-[9px] font-bold uppercase', statusColor)}>
                {info.status}
            </span>
        </div>
    );
}

// ── Main Component ────────────────────────────────────────────────────

export function OpenClawPresence() {
    const [presence, setPresence] = useState<openclaw.AgentRuntimePresence | null>(null);
    const [sessions, setSessions] = useState<openclaw.OpenClawSession[]>([]);
    const [subAgents, setSubAgents] = useState<openclaw.ChildSessionInfo[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [lastFetched, setLastFetched] = useState<Date | null>(null);
    const [expandedSection, setExpandedSection] = useState<string | null>('sessions');

    const fetchAll = useCallback(async () => {
        try {
            const pres = await openclaw.getOpenClawSystemPresence();
            setPresence(pres);

            const sess = await openclaw.getOpenClawSessions();
            // Filter out internal subagent sessions (they use ":task-" keyed sessions)
            const allSessions = sess.sessions || [];
            const userSessions = allSessions.filter((s: any) => !String(s.session_key || '').includes(':task-'));
            setSessions(userSessions);

            try {
                const children = await openclaw.listChildSessions('agent:main');
                setSubAgents(children || []);
            } catch {
                setSubAgents([]);
            }

            setLastFetched(new Date());
        } catch (e) {
            console.error('[Presence] Failed to fetch:', e);
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        fetchAll();
        const interval = setInterval(fetchAll, 5000);
        return () => clearInterval(interval);
    }, [fetchAll]);

    const isOnline = presence?.online ?? false;

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 p-8 space-y-8 max-w-6xl mx-auto"
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div>
                    <h1 className="text-3xl font-bold tracking-tight">Agent Runtime</h1>
                    <p className="text-muted-foreground mt-1 text-sm">
                        Live introspection of the embedded IronClaw engine.
                        {lastFetched && (
                            <span className="ml-2 text-muted-foreground/50 font-mono text-[10px]">
                                Updated {lastFetched.toLocaleTimeString()}
                            </span>
                        )}
                    </p>
                </div>
                <div className="flex items-center gap-3">
                    {/* Engine status pill — semantic color only */}
                    <div className={cn(
                        'flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-bold uppercase tracking-wider',
                        isOnline
                            ? 'text-green-500 bg-green-500/10'
                            : 'text-red-500 bg-red-500/10'
                    )}>
                        <Circle className={cn('w-2 h-2 fill-current', isOnline && 'animate-pulse')} />
                        {isOnline ? 'Engine Online' : 'Engine Offline'}
                    </div>
                    <button
                        onClick={() => { setIsLoading(true); fetchAll(); }}
                        className="p-2.5 rounded-lg bg-card hover:bg-white/5 transition-colors"
                    >
                        <RefreshCw className={cn('w-4 h-4', isLoading && 'animate-spin')} />
                    </button>
                </div>
            </div>

            {/* Engine info bar */}
            {presence && (
                <div className="flex items-center gap-4 px-5 py-3 rounded-xl bg-white/[0.02] text-xs font-mono text-muted-foreground/70">
                    <span className="flex items-center gap-1.5">
                        <Cpu className="w-3 h-3" />
                        Engine: <span className="text-foreground/70">{presence.engine}</span>
                    </span>
                    <span className="w-px h-4 bg-muted-foreground/10" />
                    <span className="flex items-center gap-1.5">
                        <Terminal className="w-3 h-3" />
                        Mode: <span className="text-foreground/70">{presence.mode}</span>
                    </span>
                    <span className="w-px h-4 bg-muted-foreground/10" />
                    <span className="flex items-center gap-1.5">
                        <Clock className="w-3 h-3" />
                        Uptime: <span className="text-foreground/70">{formatUptime(presence.uptime_secs)}</span>
                    </span>
                    <span className="w-px h-4 bg-muted-foreground/10" />
                    <span className="flex items-center gap-1.5">
                        <Timer className="w-3 h-3" />
                        Scheduler:{' '}
                        <span className={presence.routine_engine_running ? 'text-green-500' : 'text-red-500'}>
                            {presence.routine_engine_running ? 'Running' : 'Stopped'}
                        </span>
                    </span>
                </div>
            )}

            {/* Stats grid */}
            {isLoading && !presence ? (
                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4">
                    {[...Array(6)].map((_, i) => (
                        <div key={i} className="h-36 rounded-2xl bg-white/[0.02] animate-pulse" />
                    ))}
                </div>
            ) : (
                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4">
                    <StatCard
                        icon={Activity}
                        label="Sessions"
                        value={presence?.session_count ?? 0}
                        sub="Active agent sessions"
                        color="text-chart-1"
                        pulse={isOnline}
                    />
                    <StatCard
                        icon={GitBranch}
                        label="Sub-Agents"
                        value={presence?.sub_agent_count ?? 0}
                        sub="Spawned child sessions"
                        color="text-chart-2"
                    />
                    <StatCard
                        icon={Wrench}
                        label="Tools"
                        value={presence?.tool_count ?? 0}
                        sub="Registered tool definitions"
                        color="text-chart-3"
                    />
                    <StatCard
                        icon={Zap}
                        label="Hooks"
                        value={presence?.hook_count ?? 0}
                        sub="Lifecycle hook handlers"
                        color="text-chart-4"
                    />
                    <StatCard
                        icon={Radio}
                        label="Channels"
                        value={presence?.channel_count ?? 0}
                        sub="Active message channels"
                        color="text-chart-5"
                    />
                    <StatCard
                        icon={Timer}
                        label="Scheduler"
                        value={presence?.routine_engine_running ? 'Running' : 'Stopped'}
                        sub="Background routine engine"
                        status={presence?.routine_engine_running ? 'success' : 'error'}
                    />
                    <StatCard
                        icon={Clock}
                        label="Uptime"
                        value={formatUptime(presence?.uptime_secs ?? null)}
                        sub="Since engine start"
                        color="text-chart-1"
                    />
                    <StatCard
                        icon={BarChart3}
                        label="Engine"
                        value={isOnline ? 'Online' : 'Offline'}
                        sub={presence?.mode ?? 'embedded'}
                        status={isOnline ? 'success' : 'error'}
                        pulse={isOnline}
                    />
                </div>
            )}

            {/* Live Sessions */}
            <div className="rounded-2xl bg-card/20 overflow-hidden">
                <button
                    className="w-full flex items-center justify-between px-6 py-4 hover:bg-white/[0.02] transition-colors"
                    onClick={() => setExpandedSection(prev => prev === 'sessions' ? null : 'sessions')}
                >
                    <div className="flex items-center gap-3">
                        <Users className="w-4 h-4 text-primary" />
                        <h2 className="text-sm font-bold uppercase tracking-widest text-muted-foreground">
                            Live Sessions
                        </h2>
                        <span className="text-[10px] text-muted-foreground/60 bg-white/5 px-1.5 py-0.5 rounded">
                            {sessions.length}
                        </span>
                    </div>
                    <ChevronRight className={cn('w-4 h-4 text-muted-foreground transition-transform',
                        expandedSection === 'sessions' && 'rotate-90')} />
                </button>

                <AnimatePresence>
                    {expandedSection === 'sessions' && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: 'auto', opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            className="overflow-hidden"
                        >
                            <div className="px-6 pb-5 space-y-2">
                                {sessions.length === 0 ? (
                                    <div className="py-8 text-center text-muted-foreground text-sm">
                                        No active sessions — start a chat to create one.
                                    </div>
                                ) : (
                                    sessions.map(sess => (
                                        <div
                                            key={sess.session_key}
                                            className="flex items-center gap-3 px-4 py-3 mt-3 rounded-xl bg-white/[0.02] hover:bg-white/[0.04] transition-colors"
                                        >
                                            <div className="flex items-center gap-2 flex-shrink-0">
                                                <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
                                            </div>
                                            <div className="flex-1 min-w-0">
                                                <p className="text-xs font-medium text-foreground/80 truncate">
                                                    {sess.title || sess.session_key}
                                                </p>
                                                <p className="text-[10px] font-mono text-muted-foreground/50">
                                                    {sess.session_key}
                                                    {sess.source && (
                                                        <span className="ml-2 text-muted-foreground/40">via {sess.source}</span>
                                                    )}
                                                </p>
                                            </div>
                                            {sess.updated_at_ms && (
                                                <span className="text-[10px] text-muted-foreground/50 font-mono flex-shrink-0">
                                                    {new Date(sess.updated_at_ms).toLocaleTimeString()}
                                                </span>
                                            )}
                                        </div>
                                    ))
                                )}
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>

            {/* Sub-agents */}
            {subAgents.length > 0 && (
                <div className="rounded-2xl bg-card/20 overflow-hidden">
                    <button
                        className="w-full flex items-center justify-between px-6 py-4 hover:bg-white/[0.02] transition-colors"
                        onClick={() => setExpandedSection(prev => prev === 'subagents' ? null : 'subagents')}
                    >
                        <div className="flex items-center gap-3">
                            <GitBranch className="w-4 h-4 text-primary" />
                            <h2 className="text-sm font-bold uppercase tracking-widest text-muted-foreground">
                                Sub-Agent Tasks
                            </h2>
                            <span className="text-[10px] text-muted-foreground/60 bg-white/5 px-1.5 py-0.5 rounded">
                                {subAgents.length}
                            </span>
                        </div>
                        <ChevronRight className={cn('w-4 h-4 text-muted-foreground transition-transform',
                            expandedSection === 'subagents' && 'rotate-90')} />
                    </button>

                    <AnimatePresence>
                        {expandedSection === 'subagents' && (
                            <motion.div
                                initial={{ height: 0, opacity: 0 }}
                                animate={{ height: 'auto', opacity: 1 }}
                                exit={{ height: 0, opacity: 0 }}
                                className="overflow-hidden"
                            >
                                <div className="px-6 pb-5 space-y-2 pt-3">
                                    {subAgents.map(agent => (
                                        <SubAgentRow key={agent.session_key} info={agent} />
                                    ))}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>
            )}

            {/* Capability summary */}
            {presence && (
                <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    <div className="p-5 rounded-2xl bg-white/[0.02] space-y-3">
                        <div className="flex items-center gap-2 text-xs font-bold uppercase tracking-widest text-muted-foreground">
                            <Layers className="w-3.5 h-3.5" />
                            Capabilities
                        </div>
                        {[
                            { label: 'Tools & Actions', ok: presence.tool_count > 0, detail: `${presence.tool_count} tools` },
                            { label: 'Lifecycle Hooks', ok: presence.hook_count > 0, detail: `${presence.hook_count} hooks` },
                            { label: 'Message Channels', ok: presence.channel_count > 0, detail: `${presence.channel_count} channels` },
                            { label: 'Background Scheduler', ok: presence.routine_engine_running, detail: presence.routine_engine_running ? 'Active' : 'Not running' },
                        ].map(({ label, ok, detail }) => (
                            <div key={label} className="flex items-center justify-between py-1.5">
                                <div className="flex items-center gap-2">
                                    {ok
                                        ? <CheckCircle2 className="w-3.5 h-3.5 text-green-500" />
                                        : <XCircle className="w-3.5 h-3.5 text-muted-foreground/30" />
                                    }
                                    <span className="text-xs text-muted-foreground">{label}</span>
                                </div>
                                <span className="text-[10px] font-mono text-muted-foreground/60">{detail}</span>
                            </div>
                        ))}
                    </div>

                    <div className="p-5 rounded-2xl bg-white/[0.02] space-y-3 md:col-span-2">
                        <div className="flex items-center gap-2 text-xs font-bold uppercase tracking-widest text-muted-foreground">
                            <Terminal className="w-3.5 h-3.5" />
                            Runtime Details
                        </div>
                        <div className="grid grid-cols-2 gap-x-8 gap-y-3 mt-2">
                            {[
                                { label: 'Engine', value: presence.engine },
                                { label: 'Mode', value: presence.mode },
                                { label: 'Status', value: presence.online ? 'Online' : 'Offline' },
                                { label: 'Uptime', value: formatUptime(presence.uptime_secs) },
                                { label: 'Sessions', value: String(presence.session_count) },
                                { label: 'Sub-Agents', value: String(presence.sub_agent_count) },
                            ].map(({ label, value }) => (
                                <div key={label} className="flex items-baseline gap-2">
                                    <span className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60 shrink-0">
                                        {label}
                                    </span>
                                    <span className="flex-1 border-b border-dotted border-muted-foreground/10" />
                                    <span className="text-xs font-mono text-foreground/70">{value}</span>
                                </div>
                            ))}
                        </div>
                    </div>
                </div>
            )}
        </motion.div>
    );
}
