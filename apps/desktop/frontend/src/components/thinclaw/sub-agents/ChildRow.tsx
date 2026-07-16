import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
    Archive,
    CheckCircle,
    ChevronDown,
    ChevronUp,
    ExternalLink,
    GitBranch,
    Loader2,
    MessageSquare,
    X,
    XCircle
} from 'lucide-react';
import type { ChildSessionInfo } from '../../../lib/thinclaw';

export interface FeedMessage {
    id: string;
    timestamp: number;
    content: string;
    category: string;
}

export type ChildEntry = ChildSessionInfo & {
    progress?: number | null;
    feed: FeedMessage[];
    archived: boolean;
    completedAt?: number;
};

interface ChildRowProps {
    child: ChildEntry;
    onViewSession?: (sessionKey: string) => void;
    onCancel: (sessionKey: string) => void;
    onArchive: (sessionKey: string) => void;
}

function StatusBadge({ status }: { status: string }) {
    switch (status.split(':')[0]) {
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
            return <span className="text-[10px] text-zinc-500">{status}</span>;
    }
}

function ProgressBar({ value }: { value: number }) {
    const percentage = Math.min(100, Math.max(0, value * 100));
    return (
        <div className="w-full h-1 bg-zinc-800 rounded-full overflow-hidden mt-1">
            <div
                className="h-full rounded-full transition-all duration-700 ease-out"
                style={{
                    width: `${percentage}%`,
                    background:
                        percentage >= 100
                            ? 'linear-gradient(90deg, #10b981, #34d399)'
                            : 'linear-gradient(90deg, #3b82f6, #60a5fa)'
                }}
            />
        </div>
    );
}

function FeedMessageRow({ message }: { message: FeedMessage }) {
    const categoryIcon = useMemo(() => {
        switch (message.category) {
            case 'tool':
                return '🔧';
            case 'thinking':
                return '💭';
            case 'result':
                return '📋';
            case 'error':
                return '❌';
            default:
                return '↳';
        }
    }, [message.category]);
    const time = useMemo(
        () =>
            new Date(message.timestamp).toLocaleTimeString([], {
                hour: '2-digit',
                minute: '2-digit',
                second: '2-digit'
            }),
        [message.timestamp]
    );

    return (
        <div className="flex items-start gap-1.5 py-0.5 group/feed animate-in fade-in slide-in-from-left-2 duration-300">
            <div className="flex flex-col items-center shrink-0 pt-0.5">
                <div className="w-px h-0.5 bg-zinc-700/50" />
                <span className="text-[9px] leading-none">{categoryIcon}</span>
                <div className="w-px flex-1 bg-zinc-700/50" />
            </div>
            <div className="flex-1 min-w-0">
                <p className="text-[10px] text-zinc-400 leading-relaxed wrap-break-word">{message.content}</p>
            </div>
            <span className="text-[8px] text-zinc-600 shrink-0 pt-0.5 opacity-0 group-hover/feed:opacity-100 transition-opacity">
                {time}
            </span>
        </div>
    );
}

export function ChildRow({ child, onViewSession, onCancel, onArchive }: ChildRowProps) {
    const [expanded, setExpanded] = useState(() => child.status === 'running');
    const [cancelling, setCancelling] = useState(false);
    const feedEndRef = useRef<HTMLDivElement>(null);
    const isActive = child.status === 'running' || child.status.startsWith('running:');
    const isDone = child.status === 'completed' || child.status === 'failed';

    useEffect(() => {
        if (isActive) {
            setExpanded(true);
        } else if (isDone && child.completedAt) {
            const timer = setTimeout(() => setExpanded(false), 3000);
            return () => clearTimeout(timer);
        }
    }, [isActive, isDone, child.completedAt]);

    useEffect(() => {
        if (expanded)
            feedEndRef.current?.scrollIntoView({
                behavior: 'smooth',
                block: 'nearest'
            });
    }, [child.feed.length, expanded]);

    const handleCancel = useCallback(
        async (event: React.MouseEvent) => {
            event.stopPropagation();
            setCancelling(true);
            try {
                await onCancel(child.session_key);
            } finally {
                setCancelling(false);
            }
        },
        [child.session_key, onCancel]
    );
    const handleView = useCallback(
        (event: React.MouseEvent) => {
            event.stopPropagation();
            onViewSession?.(child.session_key);
        },
        [child.session_key, onViewSession]
    );
    const handleArchive = useCallback(
        (event: React.MouseEvent) => {
            event.stopPropagation();
            onArchive(child.session_key);
        },
        [child.session_key, onArchive]
    );

    const [displayName, displayTask] = useMemo(() => {
        const match = child.task.match(/^\[([^\]]+)\]\s*(.*)/);
        return match ? [match[1], match[2]] : ['sub-agent', child.task];
    }, [child.task]);
    const doneStatusColor =
        child.status === 'completed'
            ? 'border-emerald-500/20 bg-emerald-500/5'
            : child.status === 'failed'
              ? 'border-red-500/20 bg-red-500/5'
              : '';

    return (
        <div
            className={`group rounded-lg border transition-all duration-300 overflow-hidden ${
                isActive
                    ? 'border-blue-500/30 bg-blue-500/5 shadow-[0_0_12px_rgba(59,130,246,0.08)]'
                    : isDone
                      ? doneStatusColor
                      : 'border-zinc-700/50 bg-zinc-800/30'
            } hover:bg-zinc-800/40`}
        >
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full flex items-center gap-2 px-2.5 py-1.5 text-left"
            >
                <GitBranch
                    className={`w-3 h-3 shrink-0 ${isActive ? 'text-blue-400 animate-pulse' : 'text-zinc-500'}`}
                />
                <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5">
                        <span className="text-[11px] font-medium text-zinc-300 truncate">{displayName}</span>
                        {displayName.startsWith('Automation') && (
                            <span className="text-[8px] font-semibold uppercase tracking-wider px-1 py-0 rounded bg-indigo-500/15 text-indigo-400 border border-indigo-500/20">
                                Auto
                            </span>
                        )}
                        <StatusBadge status={child.status} />
                        <span className="text-[9px] text-zinc-600">{formatTimeAgo(child.spawned_at)}</span>
                    </div>
                    <p className="text-[10px] text-zinc-500 truncate mt-0.5">{displayTask}</p>
                    {isActive && child.progress != null && <ProgressBar value={child.progress} />}
                </div>
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
                            {cancelling ? (
                                <Loader2 className="w-2.5 h-2.5 animate-spin" />
                            ) : (
                                <X className="w-2.5 h-2.5" />
                            )}
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
                {(child.feed.length > 0 || child.result_summary) &&
                    (expanded ? (
                        <ChevronUp className="w-2.5 h-2.5 text-zinc-500 shrink-0" />
                    ) : (
                        <ChevronDown className="w-2.5 h-2.5 text-zinc-500 shrink-0" />
                    ))}
            </button>

            {expanded && (
                <div className="border-t border-zinc-700/20">
                    {child.feed.length > 0 && (
                        <div className="px-2.5 py-1 max-h-40 overflow-y-auto scrollbar-thin scrollbar-thumb-zinc-700 scrollbar-track-transparent">
                            <div className="flex items-center gap-1 mb-0.5">
                                <MessageSquare className="w-2.5 h-2.5 text-zinc-600" />
                                <span className="text-[9px] text-zinc-600 font-medium uppercase tracking-wider">
                                    Feed
                                </span>
                                <span className="text-[8px] text-zinc-700">({child.feed.length})</span>
                            </div>
                            {child.feed.map((message) => (
                                <FeedMessageRow key={message.id} message={message} />
                            ))}
                            <div ref={feedEndRef} />
                        </div>
                    )}
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

function formatTimeAgo(timestamp: number): string {
    const seconds = Math.floor((Date.now() - timestamp) / 1000);
    if (seconds < 60) return 'just now';
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
}
