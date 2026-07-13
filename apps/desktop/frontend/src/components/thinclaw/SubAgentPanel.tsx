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
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronDown, Plus, X, AlertCircle, GitBranch, Archive, PanelRightClose } from 'lucide-react';
import type { ChildSessionInfo } from '../../lib/thinclaw';
import { listChildSessions, spawnSession, abortThinClawChat, updateSubAgentStatus } from '../../lib/thinclaw';
import { useThinClawEvents } from '../../hooks/use-thinclaw-stream';
import { ChildRow, type ChildEntry, type FeedMessage } from './sub-agents/ChildRow';

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
    const trackedRef = useRef(new Set<string>());

    useEffect(() => {
        let cancelled = false;
        const tracked = new Set<string>();
        trackedRef.current = tracked;
        setCount(0);

        // Load initial
        listChildSessions(sessionKey)
            .then((loaded) => {
                if (cancelled) return;
                loaded.forEach((c) => tracked.add(c.session_key));
                setCount(tracked.size);
            })
            .catch(() => {});

        return () => {
            cancelled = true;
        };
    }, [sessionKey]);

    useThinClawEvents((data) => {
        if (data.kind !== 'SubAgentUpdate' || data.parent_session !== sessionKey) return;
        trackedRef.current.add(data.child_session);
        setCount(trackedRef.current.size);
    });

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
                setChildren(
                    loaded.map((c) => ({
                        ...c,
                        feed: [],
                        archived: c.status === 'completed' || c.status === 'failed'
                    }))
                );
            })
            .catch(() => {});
    }, [sessionKey]);

    useThinClawEvents((data) => {
        if (data.kind !== 'SubAgentUpdate' || data.parent_session !== sessionKey) return;

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
                        category
                    });
                }

                if ((baseStatus === 'completed' || baseStatus === 'failed') && data.result_preview) {
                    newFeed.push({
                        id: `feed-${++feedIdCounter}`,
                        timestamp: Date.now(),
                        content: data.result_preview,
                        category: baseStatus === 'completed' ? 'result' : 'error'
                    });
                }

                updated[idx] = {
                    ...existing,
                    status: data.status as ChildSessionInfo['status'],
                    progress: data.progress ?? existing.progress,
                    result_summary: data.result_preview ?? existing.result_summary,
                    feed: newFeed,
                    completedAt:
                        baseStatus === 'completed' || baseStatus === 'failed' ? Date.now() : existing.completedAt
                };
                return updated;
            } else {
                const initialFeed: FeedMessage[] = [];
                if (data.task) {
                    initialFeed.push({
                        id: `feed-${++feedIdCounter}`,
                        timestamp: Date.now(),
                        content: `Task started: ${data.task}`,
                        category: 'progress'
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
                        archived: false
                    }
                ];
            }
        });
    });

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
            await abortThinClawChat(childSessionKey);
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
                                      category: 'error'
                                  }
                              ]
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
        setChildren((prev) => prev.map((c) => (c.session_key === childSessionKey ? { ...c, archived: true } : c)));
    }, []);

    // Archive all completed
    const handleArchiveAll = useCallback(() => {
        setChildren((prev) =>
            prev.map((c) => (c.status === 'completed' || c.status === 'failed' ? { ...c, archived: true } : c))
        );
    }, []);

    // Separate active and archived
    const activeChildren = useMemo(() => children.filter((c) => !c.archived), [children]);
    const archivedChildren = useMemo(() => children.filter((c) => c.archived), [children]);

    const runningCount = children.filter((c) => c.status === 'running' || c.status.startsWith('running:')).length;

    const completedCount = activeChildren.filter((c) => c.status === 'completed' || c.status === 'failed').length;

    return (
        <div className="flex flex-col h-full bg-zinc-950/50 backdrop-blur-xs">
            {/* Panel Header */}
            <div className="flex items-center justify-between px-3 py-2.5 border-b border-zinc-700/40 shrink-0">
                <div className="flex items-center gap-2">
                    <GitBranch className="w-4 h-4 text-blue-400" />
                    <span className="text-xs font-semibold text-zinc-200 tracking-wide">Sub-Agents</span>
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
                                            if (e.key === 'Escape') {
                                                setSpawning(false);
                                                setSpawnTask('');
                                                setSpawnError(null);
                                            }
                                        }}
                                        placeholder="Describe the task..."
                                        className="flex-1 bg-transparent text-[11px] text-zinc-200 placeholder-zinc-500 border-none outline-hidden"
                                    />
                                    <button
                                        onClick={handleSpawn}
                                        disabled={!spawnTask.trim()}
                                        className="text-[10px] font-medium text-blue-400 hover:text-blue-300 disabled:text-zinc-600 transition-colors"
                                    >
                                        Go
                                    </button>
                                    <button
                                        onClick={() => {
                                            setSpawning(false);
                                            setSpawnTask('');
                                            setSpawnError(null);
                                        }}
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
                            <ChevronDown
                                className={`w-2.5 h-2.5 text-zinc-600 transition-transform ${showArchived ? 'rotate-180' : ''}`}
                            />
                            <div className="flex-1 h-px bg-zinc-700/30" />
                        </button>

                        <AnimatePresence>
                            {showArchived &&
                                archivedChildren.map((child) => (
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
