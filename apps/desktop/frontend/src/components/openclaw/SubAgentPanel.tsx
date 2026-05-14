/**
 * SubAgentPanel — right-side split pane for parallel sub-agent execution.
 *
 * Slides in from the right when sub-agents are spawned; slides out when all are
 * archived or dismissed. Each sub-agent gets its own collapsible card with a
 * live activity feed (branched timeline).
 *
 * Features:
 * - Real-time progress message feed per sub-agent
 * - Status badges, progress bars, result previews
 * - Auto-expand running agents, auto-collapse 3s after completion
 * - Cancel / archive / view actions
 * - Manual spawn from panel header
 *
 * Listens for SubAgentUpdate events from the backend to update in real-time.
 */
import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { motion, AnimatePresence } from 'framer-motion';
import {
    CheckCircle,
    XCircle,
    Loader2,
    ChevronDown,
    ChevronUp,
    Plus,
    ExternalLink,
    X,
    AlertCircle,
    GitBranch,
    Archive,
    MessageSquare,
    PanelRightClose,
} from 'lucide-react';
import type { ChildSessionInfo } from '../../lib/openclaw';
import { listChildSessions, spawnSession, abortOpenClawChat, updateSubAgentStatus } from '../../lib/openclaw';

// ── Types ────────────────────────────────────────────────────────────────

interface SubAgentUpdateEvent {
    kind: 'SubAgentUpdate';
    parent_session: string;
    child_session: string;
    task: string;
    status: string;
    progress: number | null;
    result_preview: string | null;
}

/** A single progress message in the sub-agent's message feed. */
interface FeedMessage {
    id: string;
    timestamp: number;
    content: string;
    category: string; // 'tool' | 'thinking' | 'progress' | 'result' | 'error'
}

type ChildEntry = ChildSessionInfo & {
    progress?: number | null;
    /** Accumulated progress messages — the "branched feed" */
    feed: FeedMessage[];
    /** Whether the user has manually dismissed/archived this entry */
    archived: boolean;
    /** When the sub-agent finished (ms timestamp) */
    completedAt?: number;
};

// ── Status Badge ─────────────────────────────────────────────────────────

function StatusBadge({ status }: { status: string }) {
    const baseStatus = status.split(':')[0];
    switch (baseStatus) {
        case 'running':
            return (
                <span className="inline-flex items-center gap-1 text-[10px] font-medium text-blue-400">
                    <Loader2 className="w-2.5 h-2.5 animate-spin" />
                    Running
                </span>
            );
        case 'completed':
            return (
                <span className="inline-flex items-center gap-1 text-[10px] font-medium text-emerald-400">
                    <CheckCircle className="w-2.5 h-2.5" />
                    Done
                </span>
            );
        case 'failed':
            return (
                <span className="inline-flex items-center gap-1 text-[10px] font-medium text-red-400">
                    <XCircle className="w-2.5 h-2.5" />
                    Failed
                </span>
            );
        default:
            return (
                <span className="text-[10px] text-zinc-500">{status}</span>
            );
    }
}

// ── Progress Bar ─────────────────────────────────────────────────────────

function ProgressBar({ value }: { value: number }) {
    const pct = Math.min(100, Math.max(0, value * 100));
    return (
        <div className="w-full h-1 bg-zinc-800 rounded-full overflow-hidden mt-1">
            <div
                className="h-full rounded-full transition-all duration-700 ease-out"
                style={{
                    width: `${pct}%`,
                    background: pct >= 100
                        ? 'linear-gradient(90deg, #10b981, #34d399)'
                        : 'linear-gradient(90deg, #3b82f6, #60a5fa)',
                }}
            />
        </div>
    );
}

// ── Feed Message Row ─────────────────────────────────────────────────────

function FeedMessageRow({ msg }: { msg: FeedMessage }) {
    const categoryIcon = useMemo(() => {
        switch (msg.category) {
            case 'tool': return '🔧';
            case 'thinking': return '💭';
            case 'result': return '📋';
            case 'error': return '❌';
            default: return '↳';
        }
    }, [msg.category]);

    const timeStr = useMemo(() => {
        const d = new Date(msg.timestamp);
        return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    }, [msg.timestamp]);

    return (
        <div className="flex items-start gap-1.5 py-0.5 group/feed animate-in fade-in slide-in-from-left-2 duration-300">
            {/* Branch line connector */}
            <div className="flex flex-col items-center flex-shrink-0 pt-0.5">
                <div className="w-px h-0.5 bg-zinc-700/50" />
                <span className="text-[9px] leading-none">{categoryIcon}</span>
                <div className="w-px flex-1 bg-zinc-700/50" />
            </div>
            <div className="flex-1 min-w-0">
                <p className="text-[10px] text-zinc-400 leading-relaxed break-words">
                    {msg.content}
                </p>
            </div>
            <span className="text-[8px] text-zinc-600 flex-shrink-0 pt-0.5 opacity-0 group-hover/feed:opacity-100 transition-opacity">
                {timeStr}
            </span>
        </div>
    );
}

// ── Child Session Row (with branched feed) ───────────────────────────────

interface ChildRowProps {
    child: ChildEntry;
    onViewSession?: (sessionKey: string) => void;
    onCancel: (sessionKey: string) => void;
    onArchive: (sessionKey: string) => void;
}

function ChildRow({ child, onViewSession, onCancel, onArchive }: ChildRowProps) {
    const [expanded, setExpanded] = useState(() => child.status === 'running');
    const [cancelling, setCancelling] = useState(false);
    const feedEndRef = useRef<HTMLDivElement>(null);
    const timeAgo = formatTimeAgo(child.spawned_at);
    const isActive = child.status === 'running' || child.status.startsWith('running:');
    const isDone = child.status === 'completed' || child.status === 'failed';

    // Auto-expand when running, auto-collapse shortly after completion
    useEffect(() => {
        if (isActive) {
            setExpanded(true);
        } else if (isDone && child.completedAt) {
            const timer = setTimeout(() => setExpanded(false), 3000);
            return () => clearTimeout(timer);
        }
    }, [isActive, isDone, child.completedAt]);

    // Auto-scroll feed to bottom on new messages
    useEffect(() => {
        if (expanded && feedEndRef.current) {
            feedEndRef.current.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
        }
    }, [child.feed.length, expanded]);

    const handleCancel = useCallback(async (e: React.MouseEvent) => {
        e.stopPropagation();
        setCancelling(true);
        try {
            await onCancel(child.session_key);
        } finally {
            setCancelling(false);
        }
    }, [child.session_key, onCancel]);

    const handleView = useCallback((e: React.MouseEvent) => {
        e.stopPropagation();
        onViewSession?.(child.session_key);
    }, [child.session_key, onViewSession]);

    const handleArchive = useCallback((e: React.MouseEvent) => {
        e.stopPropagation();
        onArchive(child.session_key);
    }, [child.session_key, onArchive]);

    // Extract the display name from task (format: "[name] actual task")
    const [displayName, displayTask] = useMemo(() => {
        const m = child.task.match(/^\[([^\]]+)\]\s*(.*)/);
        return m ? [m[1], m[2]] : ['sub-agent', child.task];
    }, [child.task]);

    const doneStatusColor = child.status === 'completed'
        ? 'border-emerald-500/20 bg-emerald-500/5'
        : child.status === 'failed'
            ? 'border-red-500/20 bg-red-500/5'
            : '';

    return (
        <div
            className={`group rounded-lg border transition-all duration-300 overflow-hidden ${isActive
                ? 'border-blue-500/30 bg-blue-500/5 shadow-[0_0_12px_rgba(59,130,246,0.08)]'
                : isDone
                    ? doneStatusColor
                    : 'border-zinc-700/50 bg-zinc-800/30'
                } hover:bg-zinc-800/40`}
        >
            {/* Header row */}
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full flex items-center gap-2 px-2.5 py-1.5 text-left"
            >
                {/* Branch icon */}
                <GitBranch className={`w-3 h-3 flex-shrink-0 ${isActive ? 'text-blue-400 animate-pulse' : 'text-zinc-500'
                    }`} />

                <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5">
                        <span className="text-[11px] font-medium text-zinc-300 truncate">{displayName}</span>
                        {displayName.startsWith('Automation') && (
                            <span className="text-[8px] font-semibold uppercase tracking-wider px-1 py-0 rounded bg-indigo-500/15 text-indigo-400 border border-indigo-500/20">
                                Auto
                            </span>
                        )}
                        <StatusBadge status={child.status} />
                        <span className="text-[9px] text-zinc-600">{timeAgo}</span>
                    </div>
                    <p className="text-[10px] text-zinc-500 truncate mt-0.5">{displayTask}</p>
                    {isActive && child.progress != null && (
                        <ProgressBar value={child.progress} />
                    )}
                </div>

                {/* Action buttons  */}
                <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                    {onViewSession && (
                        <button
                            onClick={handleView}
                            title="Open session"
                            className="p-0.5 rounded hover:bg-zinc-700 text-zinc-400 hover:text-blue-400 transition-colors"
                        >
                            <ExternalLink className="w-2.5 h-2.5" />
                        </button>
                    )}
                    {isActive && (
                        <button
                            onClick={handleCancel}
                            disabled={cancelling}
                            title="Cancel task"
                            className="p-0.5 rounded hover:bg-zinc-700 text-zinc-400 hover:text-red-400 transition-colors disabled:opacity-50"
                        >
                            {cancelling
                                ? <Loader2 className="w-2.5 h-2.5 animate-spin" />
                                : <X className="w-2.5 h-2.5" />
                            }
                        </button>
                    )}
                    {isDone && (
                        <button
                            onClick={handleArchive}
                            title="Archive / dismiss"
                            className="p-0.5 rounded hover:bg-zinc-700 text-zinc-400 hover:text-zinc-300 transition-colors"
                        >
                            <Archive className="w-2.5 h-2.5" />
                        </button>
                    )}
                </div>

                {(child.feed.length > 0 || child.result_summary) && (
                    expanded
                        ? <ChevronUp className="w-2.5 h-2.5 text-zinc-500 flex-shrink-0" />
                        : <ChevronDown className="w-2.5 h-2.5 text-zinc-500 flex-shrink-0" />
                )}
            </button>

            {/* Expanded branched feed */}
            {expanded && (
                <div className="border-t border-zinc-700/20">
                    {/* Message feed — the "branch" */}
                    {child.feed.length > 0 && (
                        <div className="px-2.5 py-1 max-h-40 overflow-y-auto scrollbar-thin scrollbar-thumb-zinc-700 scrollbar-track-transparent">
                            <div className="flex items-center gap-1 mb-0.5">
                                <MessageSquare className="w-2.5 h-2.5 text-zinc-600" />
                                <span className="text-[9px] text-zinc-600 font-medium uppercase tracking-wider">
                                    Feed
                                </span>
                                <span className="text-[8px] text-zinc-700">
                                    ({child.feed.length})
                                </span>
                            </div>
                            {child.feed.map((msg) => (
                                <FeedMessageRow key={msg.id} msg={msg} />
                            ))}
                            <div ref={feedEndRef} />
                        </div>
                    )}

                    {/* Result summary — shown only when done */}
                    {isDone && child.result_summary && (
                        <div className="px-2.5 pb-2 border-t border-zinc-700/20">
                            <p className="text-[10px] text-zinc-400 mt-1 whitespace-pre-wrap leading-relaxed line-clamp-6">
                                {child.result_summary}
                            </p>
                            {onViewSession && (
                                <button
                                    onClick={handleView}
                                    className="mt-1 text-[10px] text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-1"
                                >
                                    <ExternalLink className="w-2.5 h-2.5" />
                                    Full session
                                </button>
                            )}
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}

// ── Main Panel (Right-Side Pane) ─────────────────────────────────────────

interface SubAgentPanelProps {
    /** The parent session key to track children for */
    sessionKey: string;
    /** Called to open a child session as a new chat view */
    onViewSession?: (sessionKey: string) => void;
    /** Called to close/hide the panel */
    onClose?: () => void;
}

let feedIdCounter = 0;

/**
 * Returns `true` when the panel has children to display (controls parent visibility).
 * The panel itself also exposes this via the `hasChildren` export for the parent to read.
 */
export function useSubAgentCount(sessionKey: string): number {
    const [count, setCount] = useState(0);

    useEffect(() => {
        let unlisten: UnlistenFn | null = null;
        let cancelled = false;
        const tracked = new Set<string>();

        // Load initial
        listChildSessions(sessionKey)
            .then((loaded) => {
                if (cancelled) return;
                loaded.forEach((c) => tracked.add(c.session_key));
                setCount(tracked.size);
            })
            .catch(() => { });

        // Listen for new sub-agents
        (async () => {
            unlisten = await listen<SubAgentUpdateEvent>('openclaw-event', (event) => {
                if (cancelled) return;
                const data = event.payload;
                if (data.kind !== 'SubAgentUpdate') return;
                if (data.parent_session !== sessionKey) return;

                tracked.add(data.child_session);
                setCount(tracked.size);
            });
        })();

        return () => {
            cancelled = true;
            unlisten?.();
        };
    }, [sessionKey]);

    return count;
}

export default function SubAgentPanel({ sessionKey, onViewSession, onClose }: SubAgentPanelProps) {
    const [children, setChildren] = useState<ChildEntry[]>([]);
    const [showArchived, setShowArchived] = useState(false);
    const [spawning, setSpawning] = useState(false);
    const [spawnTask, setSpawnTask] = useState('');
    const [spawnError, setSpawnError] = useState<string | null>(null);
    const inputRef = useRef<HTMLInputElement>(null);

    // Load initial children
    useEffect(() => {
        listChildSessions(sessionKey)
            .then((loaded) => {
                setChildren(loaded.map((c) => ({
                    ...c,
                    feed: [],
                    archived: c.status === 'completed' || c.status === 'failed',
                })));
            })
            .catch(() => { });
    }, [sessionKey]);

    // Listen for SubAgentUpdate events from the backend
    useEffect(() => {
        let unlisten: UnlistenFn | null = null;
        let cancelled = false;

        (async () => {
            unlisten = await listen<SubAgentUpdateEvent>('openclaw-event', (event) => {
                if (cancelled) return;
                const data = event.payload;
                if (data.kind !== 'SubAgentUpdate') return;
                if (data.parent_session !== sessionKey) return;

                setChildren((prev) => {
                    const idx = prev.findIndex((c) => c.session_key === data.child_session);
                    const statusParts = data.status.split(':');
                    const baseStatus = statusParts[0];
                    const category = statusParts[1] || 'progress';

                    if (idx >= 0) {
                        const updated = [...prev];
                        const existing = updated[idx];

                        const newFeed = [...existing.feed];
                        if (data.task && baseStatus === 'running' && data.task !== existing.task) {
                            newFeed.push({
                                id: `feed-${++feedIdCounter}`,
                                timestamp: Date.now(),
                                content: data.task,
                                category,
                            });
                        }

                        if ((baseStatus === 'completed' || baseStatus === 'failed') && data.result_preview) {
                            newFeed.push({
                                id: `feed-${++feedIdCounter}`,
                                timestamp: Date.now(),
                                content: data.result_preview,
                                category: baseStatus === 'completed' ? 'result' : 'error',
                            });
                        }

                        updated[idx] = {
                            ...existing,
                            status: data.status as ChildSessionInfo['status'],
                            progress: data.progress ?? existing.progress,
                            result_summary: data.result_preview ?? existing.result_summary,
                            feed: newFeed,
                            completedAt: (baseStatus === 'completed' || baseStatus === 'failed')
                                ? Date.now()
                                : existing.completedAt,
                        };
                        return updated;
                    } else {
                        const initialFeed: FeedMessage[] = [];
                        if (data.task) {
                            initialFeed.push({
                                id: `feed-${++feedIdCounter}`,
                                timestamp: Date.now(),
                                content: `Task started: ${data.task}`,
                                category: 'progress',
                            });
                        }

                        return [
                            ...prev,
                            {
                                session_key: data.child_session,
                                task: data.task,
                                status: data.status as ChildSessionInfo['status'],
                                spawned_at: Date.now(),
                                result_summary: data.result_preview,
                                progress: data.progress,
                                feed: initialFeed,
                                archived: false,
                            },
                        ];
                    }
                });
            });
        })();

        return () => {
            cancelled = true;
            unlisten?.();
        };
    }, [sessionKey]);

    // Spawn handler
    const handleSpawn = useCallback(async () => {
        if (!spawnTask.trim()) return;
        setSpawnError(null);
        try {
            await spawnSession('main', spawnTask.trim(), sessionKey);
            setSpawnTask('');
            setSpawning(false);
        } catch (e: any) {
            setSpawnError(e?.message ?? 'Failed to spawn sub-agent');
        }
    }, [spawnTask, sessionKey]);

    // Cancel handler
    const handleCancel = useCallback(async (childSessionKey: string) => {
        try {
            await abortOpenClawChat(childSessionKey);
            await updateSubAgentStatus(childSessionKey, 'failed', 'Cancelled by user');
            setChildren((prev) =>
                prev.map((c) =>
                    c.session_key === childSessionKey
                        ? {
                            ...c,
                            status: 'failed',
                            result_summary: 'Cancelled by user',
                            completedAt: Date.now(),
                            feed: [
                                ...c.feed,
                                {
                                    id: `feed-${++feedIdCounter}`,
                                    timestamp: Date.now(),
                                    content: 'Cancelled by user',
                                    category: 'error',
                                },
                            ],
                        }
                        : c
                )
            );
        } catch (e) {
            console.error('Failed to cancel sub-agent:', e);
        }
    }, []);

    // Archive handler
    const handleArchive = useCallback((childSessionKey: string) => {
        setChildren((prev) =>
            prev.map((c) =>
                c.session_key === childSessionKey
                    ? { ...c, archived: true }
                    : c
            )
        );
    }, []);

    // Archive all completed
    const handleArchiveAll = useCallback(() => {
        setChildren((prev) =>
            prev.map((c) =>
                (c.status === 'completed' || c.status === 'failed')
                    ? { ...c, archived: true }
                    : c
            )
        );
    }, []);

    // Separate active and archived
    const activeChildren = useMemo(() =>
        children.filter((c) => !c.archived),
        [children]
    );
    const archivedChildren = useMemo(() =>
        children.filter((c) => c.archived),
        [children]
    );

    const runningCount = children.filter((c) =>
        c.status === 'running' || c.status.startsWith('running:')
    ).length;

    const completedCount = activeChildren.filter((c) =>
        c.status === 'completed' || c.status === 'failed'
    ).length;

    return (
        <div className="flex flex-col h-full bg-zinc-950/50 backdrop-blur-sm">
            {/* Panel Header */}
            <div className="flex items-center justify-between px-3 py-2.5 border-b border-zinc-700/40 shrink-0">
                <div className="flex items-center gap-2">
                    <GitBranch className="w-4 h-4 text-blue-400" />
                    <span className="text-xs font-semibold text-zinc-200 tracking-wide">
                        Sub-Agents
                    </span>
                    {runningCount > 0 && (
                        <span className="text-[9px] bg-blue-500/20 text-blue-400 px-1.5 py-0.5 rounded-full font-mono animate-pulse">
                            {runningCount} active
                        </span>
                    )}
                </div>
                <div className="flex items-center gap-1">
                    {completedCount > 0 && (
                        <button
                            onClick={handleArchiveAll}
                            className="p-1 rounded-md text-zinc-500 hover:text-zinc-300 hover:bg-zinc-700/50 transition-colors"
                            title="Archive all completed"
                        >
                            <Archive className="w-3.5 h-3.5" />
                        </button>
                    )}
                    <button
                        onClick={() => {
                            setSpawning(true);
                            setSpawnError(null);
                            setTimeout(() => inputRef.current?.focus(), 50);
                        }}
                        className="p-1 rounded-md hover:bg-zinc-700/50 text-zinc-400 hover:text-white transition-colors"
                        title="Spawn sub-agent"
                    >
                        <Plus className="w-3.5 h-3.5" />
                    </button>
                    {onClose && (
                        <button
                            onClick={onClose}
                            className="p-1 rounded-md hover:bg-zinc-700/50 text-zinc-500 hover:text-zinc-300 transition-colors"
                            title="Close panel"
                        >
                            <PanelRightClose className="w-3.5 h-3.5" />
                        </button>
                    )}
                </div>
            </div>

            {/* Scrollable content */}
            <div className="flex-1 overflow-y-auto scrollbar-thin scrollbar-thumb-zinc-700 scrollbar-track-transparent p-2 space-y-1.5">
                {/* Inline spawn input */}
                <AnimatePresence>
                    {spawning && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: 'auto', opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            transition={{ duration: 0.2 }}
                            className="overflow-hidden"
                        >
                            <div className="flex flex-col gap-1 p-2 rounded-lg border border-blue-500/30 bg-blue-500/5 mb-1">
                                <div className="flex items-center gap-1.5">
                                    <input
                                        ref={inputRef}
                                        value={spawnTask}
                                        onChange={(e) => setSpawnTask(e.target.value)}
                                        onKeyDown={(e) => {
                                            if (e.key === 'Enter') handleSpawn();
                                            if (e.key === 'Escape') { setSpawning(false); setSpawnTask(''); setSpawnError(null); }
                                        }}
                                        placeholder="Describe the task..."
                                        className="flex-1 bg-transparent text-[11px] text-zinc-200 placeholder-zinc-500 border-none outline-none"
                                    />
                                    <button
                                        onClick={handleSpawn}
                                        disabled={!spawnTask.trim()}
                                        className="text-[10px] font-medium text-blue-400 hover:text-blue-300 disabled:text-zinc-600 transition-colors"
                                    >
                                        Go
                                    </button>
                                    <button
                                        onClick={() => { setSpawning(false); setSpawnTask(''); setSpawnError(null); }}
                                        className="text-zinc-500 hover:text-zinc-300 transition-colors"
                                    >
                                        <X className="w-3 h-3" />
                                    </button>
                                </div>
                                {spawnError && (
                                    <div className="flex items-center gap-1 text-[10px] text-red-400">
                                        <AlertCircle className="w-2.5 h-2.5" />
                                        {spawnError}
                                    </div>
                                )}
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* Empty state */}
                {activeChildren.length === 0 && !spawning && (
                    <div className="flex flex-col items-center justify-center py-8 text-center">
                        <GitBranch className="w-8 h-8 text-zinc-700 mb-2" />
                        <p className="text-[11px] text-zinc-500 font-medium">No active sub-agents</p>
                        <p className="text-[10px] text-zinc-600 mt-0.5">
                            The agent will spawn workers here when needed
                        </p>
                    </div>
                )}

                {/* Active child rows */}
                <AnimatePresence mode="popLayout">
                    {activeChildren.map((child) => (
                        <motion.div
                            key={child.session_key}
                            layout
                            initial={{ opacity: 0, x: 20 }}
                            animate={{ opacity: 1, x: 0 }}
                            exit={{ opacity: 0, x: 20, height: 0 }}
                            transition={{ duration: 0.25 }}
                        >
                            <ChildRow
                                child={child}
                                onViewSession={onViewSession}
                                onCancel={handleCancel}
                                onArchive={handleArchive}
                            />
                        </motion.div>
                    ))}
                </AnimatePresence>

                {/* Archived section */}
                {archivedChildren.length > 0 && (
                    <div className="pt-1">
                        <button
                            onClick={() => setShowArchived(!showArchived)}
                            className="flex items-center gap-2 w-full py-1 group"
                        >
                            <div className="flex-1 h-px bg-zinc-700/30" />
                            <span className="text-[9px] text-zinc-600 font-medium uppercase tracking-wider group-hover:text-zinc-400 transition-colors">
                                {archivedChildren.length} Archived
                            </span>
                            <ChevronDown className={`w-2.5 h-2.5 text-zinc-600 transition-transform ${showArchived ? 'rotate-180' : ''}`} />
                            <div className="flex-1 h-px bg-zinc-700/30" />
                        </button>

                        <AnimatePresence>
                            {showArchived && archivedChildren.map((child) => (
                                <motion.div
                                    key={child.session_key}
                                    initial={{ height: 0, opacity: 0 }}
                                    animate={{ height: 'auto', opacity: 0.6 }}
                                    exit={{ height: 0, opacity: 0 }}
                                    transition={{ duration: 0.2 }}
                                    className="overflow-hidden"
                                >
                                    <ChildRow
                                        child={child}
                                        onViewSession={onViewSession}
                                        onCancel={handleCancel}
                                        onArchive={handleArchive}
                                    />
                                </motion.div>
                            ))}
                        </AnimatePresence>
                    </div>
                )}
            </div>

            {/* Panel footer — summary stats */}
            <div className="shrink-0 border-t border-zinc-700/30 px-3 py-1.5 flex items-center justify-between">
                <span className="text-[9px] text-zinc-600">
                    {children.length} total · {runningCount} running · {archivedChildren.length} archived
                </span>
                {archivedChildren.length > 0 && (
                    <button
                        onClick={() => setChildren((prev) => prev.filter((c) => !c.archived))}
                        className="text-[9px] text-zinc-600 hover:text-zinc-400 transition-colors"
                    >
                        Clear archived
                    </button>
                )}
            </div>
        </div>
    );
}

// ── Helpers ──────────────────────────────────────────────────────────────

function formatTimeAgo(tsMs: number): string {
    const diff = Date.now() - tsMs;
    const seconds = Math.floor(diff / 1000);
    if (seconds < 60) return 'just now';
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
}
