import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Activity, Trash2, Pause, Play, Filter, Search,
    Zap, MessageSquare, Wrench, AlertTriangle, Eye
} from 'lucide-react';
import { listen } from '@tauri-apps/api/event';

interface EventEntry {
    id: string;
    timestamp: Date;
    kind: string;
    session_key?: string;
    run_id?: string;
    raw: any;
}

const KIND_CONFIG: Record<string, { icon: typeof Activity; color: string }> = {
    'AssistantText': { icon: MessageSquare, color: 'text-blue-400' },
    'AssistantInternal': { icon: Eye, color: 'text-primary' },
    'ToolCall': { icon: Wrench, color: 'text-muted-foreground' },
    'ToolResult': { icon: Wrench, color: 'text-primary' },
    'StatusUpdate': { icon: Activity, color: 'text-primary' },
    'Error': { icon: AlertTriangle, color: 'text-red-400' },
    'CanvasUpdate': { icon: Zap, color: 'text-primary' },
};

let nextId = 0;

export function OpenClawEventInspector() {
    const [events, setEvents] = useState<EventEntry[]>([]);
    const [paused, setPaused] = useState(false);
    const [search, setSearch] = useState('');
    const [selectedKinds, setSelectedKinds] = useState<Set<string>>(new Set());
    const [expandedId, setExpandedId] = useState<string | null>(null);
    const [showFilters, setShowFilters] = useState(false);
    const scrollRef = useRef<HTMLDivElement>(null);
    const pausedRef = useRef(false);

    // Keep ref in sync
    useEffect(() => { pausedRef.current = paused; }, [paused]);

    // Listen for events
    useEffect(() => {
        const unlistenPromise = listen<any>('openclaw-event', (event) => {
            if (pausedRef.current) return;
            const payload = event.payload;
            const entry: EventEntry = {
                id: `evt-${nextId++}`,
                timestamp: new Date(),
                kind: payload?.kind || 'Unknown',
                session_key: payload?.session_key,
                run_id: payload?.run_id,
                raw: payload,
            };
            setEvents(prev => {
                const next = [...prev, entry];
                if (next.length > 500) return next.slice(-500);
                return next;
            });
        });

        return () => { unlistenPromise.then(u => u()); };
    }, []);

    // Auto-scroll
    useEffect(() => {
        if (!paused && scrollRef.current) {
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
        }
    }, [events, paused]);

    const allKinds = [...new Set(events.map(e => e.kind))];

    const filtered = events.filter(e => {
        if (selectedKinds.size > 0 && !selectedKinds.has(e.kind)) return false;
        if (search) {
            const s = search.toLowerCase();
            return e.kind.toLowerCase().includes(s) ||
                e.session_key?.toLowerCase().includes(s) ||
                JSON.stringify(e.raw).toLowerCase().includes(s);
        }
        return true;
    });

    const toggleKind = (kind: string) => {
        setSelectedKinds(prev => {
            const next = new Set(prev);
            if (next.has(kind)) next.delete(kind);
            else next.add(kind);
            return next;
        });
    };

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Header */}
            <div className="flex-shrink-0 px-5 pt-5 pb-3">
                <div className="flex items-center justify-between mb-3">
                    <div className="flex items-center gap-3">
                        <div className="w-9 h-9 rounded-xl bg-gradient-to-br from-cyan-500/20 to-blue-500/20 border border-cyan-500/30 flex items-center justify-center">
                            <Activity className="w-4.5 h-4.5 text-primary" />
                        </div>
                        <div>
                            <h2 className="text-base font-semibold text-zinc-100">Event Inspector</h2>
                            <p className="text-xs text-muted-foreground/60">{events.length} events captured</p>
                        </div>
                    </div>
                    <div className="flex items-center gap-2">
                        <button
                            onClick={() => setPaused(!paused)}
                            className={`p-2 rounded-lg border transition-all ${paused
                                ? 'bg-amber-500/10 border-amber-500/30 text-muted-foreground'
                                : 'bg-white/5 border-border/40 text-muted-foreground hover:text-white'
                                }`}
                            title={paused ? 'Resume' : 'Pause'}
                        >
                            {paused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
                        </button>
                        <button
                            onClick={() => setShowFilters(!showFilters)}
                            className={`p-2 rounded-lg border transition-all ${showFilters || selectedKinds.size > 0
                                ? 'bg-cyan-500/10 border-cyan-500/30 text-primary'
                                : 'bg-white/5 border-border/40 text-muted-foreground hover:text-white'
                                }`}
                        >
                            <Filter className="w-3.5 h-3.5" />
                        </button>
                        <button
                            onClick={() => setEvents([])}
                            className="p-2 rounded-lg bg-white/5 border border-border/40 text-muted-foreground hover:text-red-400 hover:bg-red-500/10 transition-all"
                            title="Clear"
                        >
                            <Trash2 className="w-3.5 h-3.5" />
                        </button>
                    </div>
                </div>

                {/* Search */}
                <div className="relative">
                    <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/60" />
                    <input
                        type="text"
                        value={search}
                        onChange={e => setSearch(e.target.value)}
                        placeholder="Filter events..."
                        className="w-full pl-9 pr-3 py-2 rounded-lg bg-white/5 border border-border/40 text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-cyan-500/50"
                    />
                </div>

                {/* Kind Filters */}
                <AnimatePresence>
                    {showFilters && allKinds.length > 0 && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: 'auto', opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            className="overflow-hidden mt-2"
                        >
                            <div className="flex flex-wrap gap-1.5">
                                {allKinds.map(kind => {
                                    const cfg = KIND_CONFIG[kind] || { icon: Zap, color: 'text-muted-foreground' };
                                    const active = selectedKinds.size === 0 || selectedKinds.has(kind);
                                    return (
                                        <button
                                            key={kind}
                                            onClick={() => toggleKind(kind)}
                                            className={`px-2 py-0.5 rounded text-[10px] font-mono border transition-all ${active ? `${cfg.color} bg-white/5 border-border/40` : 'text-zinc-600 bg-transparent border-transparent'
                                                }`}
                                        >
                                            {kind}
                                        </button>
                                    );
                                })}
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>

            {/* Paused Banner */}
            <AnimatePresence>
                {paused && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="mx-5 overflow-hidden"
                    >
                        <div className="py-1.5 px-3 rounded bg-amber-500/10 border border-amber-500/20 text-xs text-amber-300 text-center">
                            ⏸ Paused — new events are not captured
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Events List */}
            <div ref={scrollRef} className="flex-1 overflow-y-auto px-5 pb-5 space-y-1 mt-2 font-mono text-xs">
                {filtered.length === 0 ? (
                    <div className="flex flex-col items-center justify-center py-16 text-muted-foreground/60">
                        <Activity className="w-8 h-8 mb-3 opacity-30" />
                        <p className="text-sm font-sans">
                            {events.length === 0 ? 'Waiting for events...' : 'No matching events'}
                        </p>
                    </div>
                ) : (
                    filtered.map((evt) => {
                        const cfg = KIND_CONFIG[evt.kind] || { icon: Zap, color: 'text-muted-foreground' };
                        const EvtIcon = cfg.icon;
                        const expanded = expandedId === evt.id;
                        return (
                            <motion.div
                                key={evt.id}
                                layout
                                className="rounded border border-white/[0.04] hover:border-border/40 transition-all cursor-pointer"
                                onClick={() => setExpandedId(expanded ? null : evt.id)}
                            >
                                <div className="flex items-center gap-2 px-2 py-1.5">
                                    <span className="text-zinc-600 w-16 flex-shrink-0">
                                        {evt.timestamp.toLocaleTimeString('en', { hour12: false, fractionalSecondDigits: 3 } as any)}
                                    </span>
                                    <EvtIcon className={`w-3 h-3 ${cfg.color} flex-shrink-0`} />
                                    <span className={`${cfg.color} w-28 flex-shrink-0 truncate`}>{evt.kind}</span>
                                    <span className="text-zinc-600 truncate flex-1">
                                        {evt.session_key ? `session:${evt.session_key.substring(0, 8)}` : ''}
                                    </span>
                                </div>
                                <AnimatePresence>
                                    {expanded && (
                                        <motion.div
                                            initial={{ height: 0, opacity: 0 }}
                                            animate={{ height: 'auto', opacity: 1 }}
                                            exit={{ height: 0, opacity: 0 }}
                                            className="overflow-hidden"
                                        >
                                            <pre className="px-2 pb-2 text-muted-foreground whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
                                                {JSON.stringify(evt.raw, null, 2)}
                                            </pre>
                                        </motion.div>
                                    )}
                                </AnimatePresence>
                            </motion.div>
                        );
                    })
                )}
            </div>
        </div>
    );
}
