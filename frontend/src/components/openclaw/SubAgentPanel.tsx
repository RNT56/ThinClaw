/**
 * SubAgentPanel — shows child sessions spawned by the current parent session.
 *
 * Renders a collapsible panel listing all sub-agent tasks with their status,
 * progress bars, and result previews. Listens for SubAgentUpdate events
 * from the backend to update in real-time.
 */
import { useState, useEffect, useCallback, useRef } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import {
    Bot,
    CheckCircle,
    XCircle,
    Loader2,
    ChevronDown,
    ChevronUp,
    Plus,
} from 'lucide-react';
import type { ChildSessionInfo } from '../../lib/openclaw';
import { listChildSessions, spawnSession } from '../../lib/openclaw';

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

// ── Status Badge ─────────────────────────────────────────────────────────

function StatusBadge({ status }: { status: string }) {
    switch (status) {
        case 'running':
            return (
                <span className="inline-flex items-center gap-1 text-xs font-medium text-blue-400">
                    <Loader2 className="w-3 h-3 animate-spin" />
                    Running
                </span>
            );
        case 'completed':
            return (
                <span className="inline-flex items-center gap-1 text-xs font-medium text-emerald-400">
                    <CheckCircle className="w-3 h-3" />
                    Done
                </span>
            );
        case 'failed':
            return (
                <span className="inline-flex items-center gap-1 text-xs font-medium text-red-400">
                    <XCircle className="w-3 h-3" />
                    Failed
                </span>
            );
        default:
            return (
                <span className="text-xs text-zinc-500">{status}</span>
            );
    }
}

// ── Progress Bar ─────────────────────────────────────────────────────────

function ProgressBar({ value }: { value: number }) {
    const pct = Math.min(100, Math.max(0, value * 100));
    return (
        <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden mt-1">
            <div
                className="h-full rounded-full transition-all duration-500 ease-out"
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

// ── Child Session Row ────────────────────────────────────────────────────

function ChildRow({ child }: { child: ChildSessionInfo & { progress?: number | null } }) {
    const [expanded, setExpanded] = useState(false);
    const timeAgo = formatTimeAgo(child.spawned_at);

    return (
        <div
            className="group rounded-lg border border-zinc-700/50 bg-zinc-800/30 hover:bg-zinc-800/50
                        transition-colors duration-200 overflow-hidden"
        >
            {/* Header row */}
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full flex items-center gap-3 px-3 py-2.5 text-left"
            >
                <Bot className="w-4 h-4 text-zinc-400 flex-shrink-0" />
                <div className="flex-1 min-w-0">
                    <p className="text-sm text-zinc-200 truncate">{child.task}</p>
                    <div className="flex items-center gap-2 mt-0.5">
                        <StatusBadge status={child.status} />
                        <span className="text-[10px] text-zinc-500">{timeAgo}</span>
                    </div>
                    {child.status === 'running' && child.progress != null && (
                        <ProgressBar value={child.progress} />
                    )}
                </div>
                {child.result_summary && (
                    expanded
                        ? <ChevronUp className="w-3.5 h-3.5 text-zinc-500" />
                        : <ChevronDown className="w-3.5 h-3.5 text-zinc-500" />
                )}
            </button>

            {/* Expanded detail */}
            {expanded && child.result_summary && (
                <div className="px-3 pb-2.5 border-t border-zinc-700/30">
                    <p className="text-xs text-zinc-400 mt-2 whitespace-pre-wrap leading-relaxed">
                        {child.result_summary}
                    </p>
                </div>
            )}
        </div>
    );
}

// ── Main Panel ───────────────────────────────────────────────────────────

interface SubAgentPanelProps {
    /** The parent session key to track children for */
    sessionKey: string;
    /** Callback to spawn a new sub-agent — uses built-in dialog if not provided */
    onSpawnSubAgent?: () => void;
}

export default function SubAgentPanel({ sessionKey, onSpawnSubAgent }: SubAgentPanelProps) {
    const [children, setChildren] = useState<(ChildSessionInfo & { progress?: number | null })[]>([]);
    const [collapsed, setCollapsed] = useState(false);
    const [spawning, setSpawning] = useState(false);
    const [spawnTask, setSpawnTask] = useState('');
    const inputRef = useRef<HTMLInputElement>(null);

    // Load initial children
    useEffect(() => {
        listChildSessions(sessionKey)
            .then(setChildren)
            .catch(() => { }); // Silently fail if engine is not running
    }, [sessionKey]);

    // Listen for SubAgentUpdate events
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
                    if (idx >= 0) {
                        // Update existing
                        const updated = [...prev];
                        updated[idx] = {
                            ...updated[idx],
                            status: data.status as ChildSessionInfo['status'],
                            progress: data.progress,
                            result_summary: data.result_preview ?? updated[idx].result_summary,
                        };
                        return updated;
                    } else {
                        // New child we haven't seen
                        return [
                            ...prev,
                            {
                                session_key: data.child_session,
                                task: data.task,
                                status: data.status as ChildSessionInfo['status'],
                                spawned_at: Date.now(),
                                result_summary: data.result_preview,
                                progress: data.progress,
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

    // Inline spawn handler
    const handleSpawn = useCallback(async () => {
        if (!spawnTask.trim()) return;
        try {
            await spawnSession('main', spawnTask.trim(), sessionKey);
            setSpawnTask('');
            setSpawning(false);
        } catch (e) {
            console.error('Failed to spawn sub-agent:', e);
        }
    }, [spawnTask, sessionKey]);

    // Don't render if no children and no spawn prompt
    if (children.length === 0 && !spawning) {
        return null;
    }

    return (
        <div className="rounded-xl border border-zinc-700/40 bg-zinc-900/60 backdrop-blur-sm
                        overflow-hidden transition-all duration-300">
            {/* Panel header */}
            <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-700/30">
                <button
                    onClick={() => setCollapsed(!collapsed)}
                    className="flex items-center gap-2 text-sm font-medium text-zinc-300 hover:text-white
                               transition-colors"
                >
                    <Bot className="w-4 h-4 text-blue-400" />
                    Sub-Agents
                    <span className="text-xs text-zinc-500 font-normal">
                        ({children.length})
                    </span>
                    {collapsed
                        ? <ChevronDown className="w-3 h-3 text-zinc-500" />
                        : <ChevronUp className="w-3 h-3 text-zinc-500" />
                    }
                </button>

                <button
                    onClick={() => {
                        if (onSpawnSubAgent) {
                            onSpawnSubAgent();
                        } else {
                            setSpawning(true);
                            setTimeout(() => inputRef.current?.focus(), 50);
                        }
                    }}
                    className="p-1 rounded-md hover:bg-zinc-700/50 text-zinc-400 hover:text-white
                               transition-colors"
                    title="Spawn sub-agent"
                >
                    <Plus className="w-3.5 h-3.5" />
                </button>
            </div>

            {/* Content */}
            {!collapsed && (
                <div className="p-2 space-y-1.5 max-h-64 overflow-y-auto">
                    {/* Inline spawn input */}
                    {spawning && (
                        <div className="flex items-center gap-2 p-2 rounded-lg border border-blue-500/30
                                        bg-blue-500/5">
                            <input
                                ref={inputRef}
                                value={spawnTask}
                                onChange={(e) => setSpawnTask(e.target.value)}
                                onKeyDown={(e) => {
                                    if (e.key === 'Enter') handleSpawn();
                                    if (e.key === 'Escape') { setSpawning(false); setSpawnTask(''); }
                                }}
                                placeholder="Describe the task..."
                                className="flex-1 bg-transparent text-sm text-zinc-200 placeholder-zinc-500
                                           border-none outline-none"
                            />
                            <button
                                onClick={handleSpawn}
                                disabled={!spawnTask.trim()}
                                className="text-xs font-medium text-blue-400 hover:text-blue-300
                                           disabled:text-zinc-600 transition-colors"
                            >
                                Spawn
                            </button>
                        </div>
                    )}

                    {/* Child rows */}
                    {children.map((child) => (
                        <ChildRow key={child.session_key} child={child} />
                    ))}
                </div>
            )}
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
