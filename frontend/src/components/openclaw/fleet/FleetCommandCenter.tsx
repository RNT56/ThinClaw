
import { useState, useEffect, useMemo, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Users, LayoutGrid, Terminal as TerminalIcon,
    Cpu, Zap, Globe, Brain, Shield, HardDrive, Cloud,
    Activity, Clock, AlertTriangle
} from 'lucide-react';
import { cn } from '../../../lib/utils';
import * as openclaw from '../../../lib/openclaw';
import { FleetGraph } from './FleetGraph';
import { FleetTerminal } from './FleetTerminal';
import { Node, Edge } from '@xyflow/react';
import { toast } from 'sonner';

// Real-time state per agent derived from events
interface AgentRealtimeState {
    runStatus: 'idle' | 'processing' | 'waiting_approval' | 'error';
    currentRunId: string | null;
    currentTool: string | null;
    lastActivity: number; // timestamp
    toolsCompleted: number;
    toolsStarted: number;
}

const CAPABILITY_ICONS: Record<string, { icon: typeof Cpu; color: string }> = {
    'inference': { icon: Brain, color: 'text-purple-400' },
    'chat': { icon: Activity, color: 'text-blue-400' },
    'web_search': { icon: Globe, color: 'text-cyan-400' },
    'local_inference': { icon: HardDrive, color: 'text-amber-400' },
    'ui_automation': { icon: Zap, color: 'text-yellow-400' },
    'filesystem': { icon: HardDrive, color: 'text-green-400' },
    'tool_use': { icon: Shield, color: 'text-indigo-400' },
};

function capabilityLabel(cap: string): string {
    if (cap.startsWith('cloud:')) return cap.replace('cloud:', '').toUpperCase();
    return cap.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}

function capabilityIcon(cap: string) {
    if (cap.startsWith('cloud:')) return { icon: Cloud, color: 'text-sky-400' };
    return CAPABILITY_ICONS[cap] || { icon: Cpu, color: 'text-zinc-400' };
}

function statusColor(status: string | null): string {
    switch (status) {
        case 'processing': return 'text-indigo-400';
        case 'waiting_approval': return 'text-amber-400';
        case 'error': return 'text-red-400';
        case 'offline': return 'text-red-500';
        case 'idle': return 'text-emerald-400';
        default: return 'text-zinc-500';
    }
}

function statusLabel(status: string | null): string {
    switch (status) {
        case 'processing': return 'Processing';
        case 'waiting_approval': return 'Awaiting Approval';
        case 'error': return 'Error';
        case 'offline': return 'Offline';
        case 'idle': return 'Idle';
        default: return 'Unknown';
    }
}

function statusDotColor(status: string | null): string {
    switch (status) {
        case 'processing': return 'bg-indigo-500 shadow-[0_0_8px_rgba(99,102,241,0.8)]';
        case 'waiting_approval': return 'bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.8)] animate-pulse';
        case 'error': return 'bg-red-500';
        case 'offline': return 'bg-red-500';
        case 'idle': return 'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.8)]';
        default: return 'bg-zinc-500';
    }
}

export function FleetCommandCenter() {
    const [agents, setAgents] = useState<openclaw.AgentStatusSummary[]>([]);
    const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
    const [showTerminal, setShowTerminal] = useState(true);
    const [isSpawning, setIsSpawning] = useState(false);
    const [realtimeLogs, setRealtimeLogs] = useState<Record<string, string[]>>({}); // agentId -> logs
    const [agentStates, setAgentStates] = useState<Record<string, AgentRealtimeState>>({});

    // Ref for agents to access inside event listener without re-subscribing
    const agentsRef = useRef(agents);
    useEffect(() => { agentsRef.current = agents; }, [agents]);

    // Manual Refresh
    const refreshFleet = useCallback(async () => {
        try {
            const data = await openclaw.getFleetStatus();
            setAgents(data);
        } catch (e) {
            console.error("Fleet fetch error:", e);
        }
    }, []);

    // Poll logic
    useEffect(() => {
        refreshFleet();
        const interval = setInterval(refreshFleet, 3000);
        return () => clearInterval(interval);
    }, [refreshFleet]);

    // Real-time Event Listener — updates both logs AND agent states
    useEffect(() => {
        import('@tauri-apps/api/event').then(({ listen }) => {
            const unlisten = listen<any>('openclaw-event', (event) => {
                const payload = event.payload;
                if (!payload || !payload.kind) return;

                let sessionKey: string | null = null;
                let logLine: string | null = null;

                if (payload.session_key) {
                    sessionKey = payload.session_key;
                }

                // Format log line based on event type
                if (payload.kind === 'ToolUpdate') {
                    const status = payload.status;
                    const toolName = payload.tool_name;
                    if (status === 'started') {
                        logLine = `[TOOL] ▶ ${toolName}`;
                    } else if (status === 'stream') {
                        // Skip noisy stream events from terminal
                        logLine = null;
                    } else if (status === 'ok') {
                        logLine = `[TOOL] ✓ ${toolName}`;
                        if (payload.output) {
                            const outStr = typeof payload.output === 'string'
                                ? payload.output
                                : JSON.stringify(payload.output).substring(0, 100);
                            logLine += ` → ${outStr}`;
                        }
                    } else if (status === 'error') {
                        logLine = `[ERROR] ✗ ${toolName} failed`;
                    }
                } else if (payload.kind === 'AssistantFinal') {
                    logLine = `[RESPONSE] ${payload.text?.substring(0, 80)}${payload.text?.length > 80 ? '…' : ''}`;
                } else if (payload.kind === 'AssistantSnapshot') {
                    if (payload.text && payload.text.length > 20) {
                        logLine = `[THINKING] ${payload.text.substring(0, 60)}${payload.text.length > 60 ? '…' : ''}`;
                    }
                } else if (payload.kind === 'RunStatus') {
                    const s = payload.status;
                    const icon = s === 'ok' ? '✓' : s === 'error' ? '✗' : s === 'started' ? '▶' : '●';
                    logLine = `[RUN] ${icon} ${s}`;
                    if (payload.error) logLine += `: ${payload.error}`;
                } else if (payload.kind === 'ApprovalRequested') {
                    logLine = `[APPROVAL] ⏳ Awaiting: ${payload.tool_name}`;
                } else if (payload.kind === 'ApprovalResolved') {
                    logLine = `[APPROVAL] ${payload.approved ? '✓ Approved' : '✗ Denied'}`;
                } else if (payload.kind === 'AssistantDelta') {
                    // Skip deltas from terminal (too noisy)
                    logLine = null;
                }

                // Find matching agent
                const agent = sessionKey ? (
                    agentsRef.current.find(a =>
                        sessionKey!.startsWith(`agent:${a.id}:`) ||
                        a.active_session_id === sessionKey
                    ) || (
                        sessionKey!.startsWith('agent:main') ? agentsRef.current[0] : null
                    )
                ) : null;

                if (agent) {
                    // Update logs
                    if (logLine) {
                        const timestamp = new Date().toLocaleTimeString('en', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
                        const taggedLine = `${timestamp} ${logLine}`;
                        setRealtimeLogs(prev => {
                            const existing = prev[agent.id] || [];
                            const newLogs = [...existing, taggedLine].slice(-100);
                            return { ...prev, [agent.id]: newLogs };
                        });
                    }

                    // Update real-time state
                    setAgentStates(prev => {
                        const current = prev[agent.id] || {
                            runStatus: 'idle' as const,
                            currentRunId: null,
                            currentTool: null,
                            lastActivity: Date.now(),
                            toolsCompleted: 0,
                            toolsStarted: 0,
                        };

                        let updated = { ...current, lastActivity: Date.now() };

                        if (payload.kind === 'RunStatus') {
                            if (payload.status === 'started' || payload.status === 'in_flight') {
                                updated.runStatus = 'processing';
                                updated.currentRunId = payload.run_id || null;
                                updated.toolsCompleted = 0;
                                updated.toolsStarted = 0;
                            } else if (payload.status === 'ok' || payload.status === 'aborted') {
                                updated.runStatus = 'idle';
                                updated.currentRunId = null;
                                updated.currentTool = null;
                            } else if (payload.status === 'error') {
                                updated.runStatus = 'error';
                                updated.currentRunId = null;
                                updated.currentTool = null;
                            }
                        } else if (payload.kind === 'ToolUpdate') {
                            if (payload.status === 'started') {
                                updated.runStatus = 'processing';
                                updated.currentTool = payload.tool_name;
                                updated.toolsStarted = (updated.toolsStarted || 0) + 1;
                            } else if (payload.status === 'ok' || payload.status === 'error') {
                                updated.currentTool = null;
                                updated.toolsCompleted = (updated.toolsCompleted || 0) + 1;
                            }
                        } else if (payload.kind === 'ApprovalRequested') {
                            updated.runStatus = 'waiting_approval';
                            updated.currentTool = payload.tool_name;
                        } else if (payload.kind === 'ApprovalResolved') {
                            updated.runStatus = 'processing';
                        } else if (payload.kind === 'AssistantDelta' || payload.kind === 'AssistantSnapshot') {
                            updated.runStatus = 'processing';
                        }

                        return { ...prev, [agent.id]: updated };
                    });
                }
            });

            return () => {
                unlisten.then(f => f());
            };
        });
    }, []);

    // Merge backend run_status with frontend real-time states
    const getEffectiveStatus = useCallback((agent: openclaw.AgentStatusSummary): string => {
        const rtState = agentStates[agent.id];
        if (rtState) return rtState.runStatus;
        return agent.run_status || (agent.online ? 'idle' : 'offline');
    }, [agentStates]);

    // Compute progress from tool counts
    const getProgress = useCallback((agent: openclaw.AgentStatusSummary): number => {
        const rtState = agentStates[agent.id];
        if (!rtState || rtState.runStatus !== 'processing') return 0;
        if (rtState.toolsStarted === 0) return 0;
        // Indeterminate progress — show pulse between 10-90% based on tools completed
        return Math.min(0.9, rtState.toolsCompleted / Math.max(rtState.toolsStarted, 1));
    }, [agentStates]);

    // Transform agents to Nodes/Edges
    const { nodes, edges } = useMemo(() => {
        if (!agents.length) return { nodes: [], edges: [] };

        const nodes: Node[] = [];
        const edges: Edge[] = [];

        const hierarchy = new Map<string, string[]>();
        agents.forEach(a => {
            if (a.parent_id) {
                const existing = hierarchy.get(a.parent_id) || [];
                existing.push(a.id);
                hierarchy.set(a.parent_id, existing);
            }
        });

        const roots = agents.filter(a => !a.parent_id);

        let yOffset = 50;
        const placeNode = (agent: openclaw.AgentStatusSummary, depth: number, x: number) => {
            const effectiveStatus = getEffectiveStatus(agent);
            const progress = getProgress(agent);
            nodes.push({
                id: agent.id,
                type: 'agent',
                position: { x, y: yOffset },
                data: {
                    label: agent.name,
                    online: agent.online,
                    active: agent.active || effectiveStatus === 'processing',
                    task: agent.current_task,
                    progress: progress,
                    status: effectiveStatus,
                    model: agent.model,
                }
            });
            yOffset += 120;

            const childrenIds = hierarchy.get(agent.id) || [];
            childrenIds.forEach((childId) => {
                const child = agents.find(a => a.id === childId);
                if (child) {
                    placeNode(child, depth + 1, x + 250);
                    const childStatus = getEffectiveStatus(child);
                    edges.push({
                        id: `e-${agent.id}-${child.id}`,
                        source: agent.id,
                        target: child.id,
                        animated: childStatus === 'processing',
                        style: {
                            stroke: childStatus === 'processing' ? '#6366f1'
                                : childStatus === 'waiting_approval' ? '#f59e0b'
                                    : child.online ? '#10b981' : '#3f3f46',
                            strokeWidth: childStatus === 'processing' ? 2 : 1,
                        }
                    });
                }
            });
        };

        roots.forEach(root => placeNode(root, 0, 50));

        if (roots.length === 0 && agents.length > 0) {
            agents.forEach((agent, index) => {
                nodes.push({
                    id: agent.id,
                    type: 'agent',
                    position: { x: (index % 3) * 300 + 50, y: Math.floor(index / 3) * 200 + 50 },
                    data: {
                        label: agent.name,
                        online: agent.online,
                        active: agent.active,
                        task: agent.current_task,
                        progress: getProgress(agent),
                        status: getEffectiveStatus(agent),
                        model: agent.model,
                    }
                });
            });
        }

        return { nodes, edges };
    }, [agents, agentStates, getEffectiveStatus, getProgress]);

    // Aggregate Logs
    const logs = useMemo(() => {
        return agents.map(a => {
            const dynamicLogs = realtimeLogs[a.id] || [];
            return {
                id: a.id,
                lines: dynamicLogs
            };
        });
    }, [agents, realtimeLogs]);

    const activeIds = selectedAgentId ? [selectedAgentId] : agents.map(a => a.id);

    const handleSpawnTask = async (e: React.FormEvent) => {
        e.preventDefault();
        const form = e.target as HTMLFormElement;
        const input = form.elements.namedItem('task') as HTMLInputElement;
        const task = input.value.trim();

        if (task && selectedAgentId) {
            setIsSpawning(true);
            try {
                await openclaw.spawnSession(selectedAgentId, task);
                input.value = '';
                toast.success(`Task spawned on ${selectedAgentId}`);
                refreshFleet();
            } catch (err) {
                console.error("Failed to spawn", err);
                toast.error('Failed to spawn task');
            } finally {
                setIsSpawning(false);
            }
        }
    };

    const handleBroadcast = async (command: string) => {
        try {
            await openclaw.broadcastCommand(command);
            toast.success('Broadcast sent to all sessions');
        } catch (e) {
            toast.error('Broadcast failed');
            console.error("Broadcast error:", e);
        }
    };

    // Derived stats
    const onlineCount = agents.filter(a => a.online).length;
    const processingCount = agents.filter(a => getEffectiveStatus(a) === 'processing').length;
    const approvalCount = agents.filter(a => getEffectiveStatus(a) === 'waiting_approval').length;

    return (
        <div className="flex flex-col h-full bg-[#050505] text-zinc-100 overflow-hidden">
            {/* Top Bar */}
            <div className="h-14 border-b border-white/10 bg-zinc-900/50 backdrop-blur flex items-center justify-between px-6 z-10">
                <div className="flex items-center gap-4">
                    <div className="p-2 bg-indigo-500/10 rounded-lg">
                        <Users className="w-5 h-5 text-indigo-500" />
                    </div>
                    <h1 className="font-bold text-lg tracking-tight">FLEET COMMAND</h1>
                    {/* Stats Pills */}
                    <div className="flex items-center gap-2">
                        <div className="px-3 py-1 rounded-full bg-white/5 border border-white/5 text-xs text-zinc-400 flex items-center gap-2">
                            <span className="w-2 h-2 rounded-full bg-emerald-500" />
                            {onlineCount} / {agents.length} Online
                        </div>
                        {processingCount > 0 && (
                            <div className="px-3 py-1 rounded-full bg-indigo-500/10 border border-indigo-500/20 text-xs text-indigo-300 flex items-center gap-2">
                                <Activity className="w-3 h-3 animate-spin" />
                                {processingCount} Active
                            </div>
                        )}
                        {approvalCount > 0 && (
                            <div className="px-3 py-1 rounded-full bg-amber-500/10 border border-amber-500/20 text-xs text-amber-300 flex items-center gap-2 animate-pulse">
                                <AlertTriangle className="w-3 h-3" />
                                {approvalCount} Pending
                            </div>
                        )}
                    </div>
                    <button
                        onClick={refreshFleet}
                        className="p-1.5 rounded-md hover:bg-white/10 text-zinc-500 hover:text-white transition-colors"
                        title="Refresh Fleet Status"
                    >
                        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" /><path d="M21 3v5h-5" /></svg>
                    </button>
                </div>

                <div className="flex items-center gap-3">
                    <button
                        onClick={() => setShowTerminal(prev => !prev)}
                        className={cn(
                            "p-2 rounded-lg transition-colors",
                            showTerminal ? "bg-white/10 text-white" : "text-zinc-500 hover:text-white"
                        )}
                    >
                        <TerminalIcon className="w-5 h-5" />
                    </button>
                    <button className="p-2 text-zinc-500 hover:text-white transition-colors">
                        <LayoutGrid className="w-5 h-5" />
                    </button>
                </div>
            </div>

            {/* Main Area */}
            <div className="flex-1 relative">
                {/* Graph View */}
                <div className="absolute inset-0">
                    <FleetGraph
                        nodes={nodes}
                        edges={edges}
                        onNodeClick={(_, node) => setSelectedAgentId(node.id)}
                    />
                </div>

                {/* Sidebar / Inspector (Overlay) */}
                <AnimatePresence>
                    {selectedAgentId && (
                        <motion.div
                            initial={{ x: 300, opacity: 0 }}
                            animate={{ x: 0, opacity: 1 }}
                            exit={{ x: 300, opacity: 0 }}
                            className="absolute right-0 top-0 bottom-0 w-80 bg-zinc-900/90 backdrop-blur-xl border-l border-white/10 p-6 z-20 shadow-2xl flex flex-col"
                        >
                            <div className="flex items-center justify-between mb-8">
                                <h2 className="font-bold text-zinc-100">AGENT DETAILS</h2>
                                <button
                                    onClick={() => setSelectedAgentId(null)}
                                    className="text-zinc-500 hover:text-white"
                                >
                                    ✕
                                </button>
                            </div>

                            {/* Selected Agent Details */}
                            {agents.find(a => a.id === selectedAgentId) ? (
                                (() => {
                                    const agent = agents.find(a => a.id === selectedAgentId)!;
                                    const effectiveStatus = getEffectiveStatus(agent);
                                    const rtState = agentStates[agent.id];
                                    const progress = getProgress(agent);
                                    return (
                                        <div className="space-y-6 flex-1 overflow-y-auto">
                                            {/* Identity Card */}
                                            <div className="p-4 rounded-xl bg-white/5 border border-white/5 space-y-4">
                                                <div className="flex items-center gap-3">
                                                    <div className="w-12 h-12 rounded-full bg-indigo-500/20 flex items-center justify-center relative">
                                                        <Cpu className="w-6 h-6 text-indigo-400" />
                                                        <div className={cn(
                                                            "absolute -bottom-0.5 -right-0.5 w-3.5 h-3.5 rounded-full border-2 border-zinc-900",
                                                            statusDotColor(effectiveStatus)
                                                        )} />
                                                    </div>
                                                    <div>
                                                        <div className="font-bold">{agent.name}</div>
                                                        <div className="text-xs text-zinc-500 font-mono truncate w-40">{agent.id}</div>
                                                    </div>
                                                </div>

                                                {/* Status */}
                                                <div className="flex items-center gap-2">
                                                    <div className={cn("w-2 h-2 rounded-full", statusDotColor(effectiveStatus))} />
                                                    <span className={cn("text-xs font-semibold uppercase tracking-wider", statusColor(effectiveStatus))}>
                                                        {statusLabel(effectiveStatus)}
                                                    </span>
                                                    {rtState?.currentTool && (
                                                        <span className="text-xs text-zinc-500 font-mono ml-1">
                                                            → {rtState.currentTool}
                                                        </span>
                                                    )}
                                                </div>

                                                {/* Stats Grid */}
                                                <div className="grid grid-cols-2 gap-2 text-xs">
                                                    <div className="p-2 bg-black/20 rounded">
                                                        <div className="text-zinc-500">Latency</div>
                                                        <div className="font-mono text-emerald-400">
                                                            {agent.latency_ms !== null ? `${agent.latency_ms}ms` : '—'}
                                                        </div>
                                                    </div>
                                                    <div className="p-2 bg-black/20 rounded">
                                                        <div className="text-zinc-500">Version</div>
                                                        <div className="font-mono text-zinc-300">{agent.version || '—'}</div>
                                                    </div>
                                                    <div className="p-2 bg-black/20 rounded col-span-2">
                                                        <div className="text-zinc-500">Model</div>
                                                        <div className="font-mono text-zinc-300 truncate">{agent.model || '—'}</div>
                                                    </div>
                                                </div>

                                                {/* Activity */}
                                                {rtState && (
                                                    <div className="grid grid-cols-2 gap-2 text-xs">
                                                        <div className="p-2 bg-black/20 rounded">
                                                            <div className="text-zinc-500">Tools Run</div>
                                                            <div className="font-mono text-zinc-300">{rtState.toolsCompleted}</div>
                                                        </div>
                                                        <div className="p-2 bg-black/20 rounded">
                                                            <div className="text-zinc-500 flex items-center gap-1">
                                                                <Clock className="w-3 h-3" /> Last Active
                                                            </div>
                                                            <div className="font-mono text-zinc-300">
                                                                {new Date(rtState.lastActivity).toLocaleTimeString('en', { hour12: false })}
                                                            </div>
                                                        </div>
                                                    </div>
                                                )}
                                            </div>

                                            {/* Capabilities */}
                                            <div>
                                                <h3 className="text-xs font-bold text-zinc-500 uppercase mb-3">Capabilities</h3>
                                                <div className="flex flex-wrap gap-2">
                                                    {(agent.capabilities || []).map(cap => {
                                                        const { icon: Icon, color } = capabilityIcon(cap);
                                                        return (
                                                            <span key={cap} className="px-2 py-1 rounded bg-zinc-800 text-[10px] text-zinc-400 border border-white/5 flex items-center gap-1">
                                                                <Icon className={cn("w-3 h-3", color)} />
                                                                {capabilityLabel(cap)}
                                                            </span>
                                                        );
                                                    })}
                                                    {(!agent.capabilities || agent.capabilities.length === 0) && (
                                                        <span className="text-xs text-zinc-600 italic">No data</span>
                                                    )}
                                                </div>
                                            </div>

                                            {/* Progress */}
                                            {progress > 0 && (
                                                <div className="space-y-1">
                                                    <div className="flex justify-between text-[10px] text-zinc-500">
                                                        <span>Run Progress</span>
                                                        <span>{rtState?.toolsCompleted || 0} tools completed</span>
                                                    </div>
                                                    <div className="h-1.5 w-full bg-zinc-800 rounded-full overflow-hidden">
                                                        <motion.div
                                                            className="h-full bg-gradient-to-r from-indigo-500 to-indigo-400"
                                                            initial={{ width: 0 }}
                                                            animate={{ width: `${Math.max(progress * 100, 10)}%` }}
                                                            transition={{ type: "spring", stiffness: 50 }}
                                                        />
                                                    </div>
                                                </div>
                                            )}

                                            {/* Orchestration */}
                                            <div className="pt-4 border-t border-white/10">
                                                <h3 className="text-xs font-bold text-zinc-500 uppercase mb-3">Orchestration</h3>

                                                {agent.active && agent.active_session_id && (
                                                    <div className="mb-4 p-3 bg-indigo-500/10 border border-indigo-500/20 rounded-lg">
                                                        <div className="text-xs text-indigo-300 font-semibold mb-1">ACTIVE TASK</div>
                                                        <div className="text-sm text-indigo-100 mb-1">{agent.current_task}</div>
                                                        <div className="text-[10px] text-zinc-500 font-mono mb-3 truncate">{agent.active_session_id}</div>
                                                        <button
                                                            onClick={async () => {
                                                                if (agent.active_session_id) {
                                                                    try {
                                                                        const runId = agentStates[agent.id]?.currentRunId || undefined;
                                                                        await openclaw.abortSession(agent.active_session_id, runId);
                                                                        toast.success('Abort signal sent');
                                                                        await refreshFleet();
                                                                    } catch (e) {
                                                                        console.error("Failed to stop session", e);
                                                                        toast.error('Failed to stop task');
                                                                    }
                                                                }
                                                            }}
                                                            className="w-full py-1.5 bg-red-500/10 hover:bg-red-500/20 text-red-500 border border-red-500/20 rounded text-xs font-bold transition-colors flex items-center justify-center gap-2"
                                                        >
                                                            <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10" /><rect x="9" y="9" width="6" height="6" /></svg>
                                                            STOP TASK
                                                        </button>
                                                    </div>
                                                )}

                                                <form onSubmit={handleSpawnTask} className="space-y-2">
                                                    <input
                                                        name="task"
                                                        type="text"
                                                        placeholder="Assign new task..."
                                                        className="w-full bg-black/40 border border-white/10 rounded px-3 py-2 text-sm text-zinc-200 placeholder:text-zinc-600 focus:border-indigo-500 outline-none transition-colors"
                                                    />
                                                    <button
                                                        type="submit"
                                                        disabled={isSpawning}
                                                        className="w-full py-2 bg-indigo-500 hover:bg-indigo-600 disabled:opacity-50 text-white font-bold rounded-lg transition-colors text-sm"
                                                    >
                                                        {isSpawning ? 'Deploying...' : 'Spawn Task'}
                                                    </button>
                                                </form>
                                            </div>
                                        </div>
                                    );
                                })()
                            ) : (
                                <div className="text-zinc-500 italic">Agent not found</div>
                            )}
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>

            {/* Terminal Drawer */}
            <motion.div
                initial={{ height: 0 }}
                animate={{ height: showTerminal ? 300 : 0 }}
                transition={{ type: "spring", stiffness: 300, damping: 30 }}
                className="overflow-hidden bg-black z-30 flex flex-col border-t border-white/10"
            >
                <FleetTerminal agentIds={activeIds} logs={logs} className="flex-1" />

                {/* Command Input */}
                <form
                    onSubmit={(e) => {
                        e.preventDefault();
                        const input = e.currentTarget.elements.namedItem('cmd') as HTMLInputElement;
                        if (input.value.trim()) {
                            handleBroadcast(input.value.trim());
                            input.value = '';
                        }
                    }}
                    className="p-2 bg-zinc-900 border-t border-white/5 flex gap-2 items-center"
                >
                    <div className="text-indigo-500 font-mono">❯</div>
                    <input
                        name="cmd"
                        type="text"
                        placeholder="Broadcast to all sessions..."
                        className="flex-1 bg-transparent border-none outline-none font-mono text-sm text-zinc-300 placeholder:text-zinc-600"
                        autoComplete="off"
                    />
                </form>
            </motion.div>
        </div>
    );
}
