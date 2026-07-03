import { useState, useCallback } from 'react';
import { motion } from 'framer-motion';
import { Search, RefreshCw, MessagesSquare, Sparkles, AlertTriangle } from 'lucide-react';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';

/** Defensive accessor — rendered hits are raw JSON whose shape can evolve. */
function field(record: thinclaw.ThinClawJson, ...keys: string[]): string {
    const obj = (record ?? {}) as Record<string, unknown>;
    for (const k of keys) {
        const v = obj[k];
        if (typeof v === 'string' && v.trim()) return v;
        if (typeof v === 'number') return String(v);
    }
    return '';
}

export function ThinClawSessionSearch() {
    const [query, setQuery] = useState('');
    const [summarize, setSummarize] = useState(false);
    const [results, setResults] = useState<thinclaw.ThinClawJson[]>([]);
    const [meta, setMeta] = useState<{ summarized: boolean; fallback: boolean } | null>(null);
    const [isLoading, setIsLoading] = useState(false);
    const [searched, setSearched] = useState(false);
    // Typed gate (BridgeError::Unavailable) or a plain error message.
    const [notice, setNotice] = useState<{ reason: string; remediation?: string } | null>(null);

    const search = useCallback(async () => {
        if (!query.trim()) return;
        setIsLoading(true);
        setNotice(null);
        setSearched(true);
        try {
            const r = await thinclaw.searchSessions(query, 30, summarize);
            setResults(Array.isArray(r.results) ? r.results : []);
            setMeta({ summarized: r.summarized, fallback: r.fallback });
        } catch (e: unknown) {
            const err = e as { kind?: string; capability?: string; reason?: string; remediation?: string; message?: string };
            // Typed gate: render the reason + remediation as a CTA, not an error.
            if (err?.kind === 'unavailable' && err.reason) {
                setNotice({ reason: err.reason, remediation: err.remediation });
            } else {
                setNotice({ reason: String(err?.message ?? e) });
            }
            setResults([]);
            setMeta(null);
        } finally {
            setIsLoading(false);
        }
    }, [query, summarize]);

    return (
        <motion.div className="flex-1 overflow-y-auto p-8 space-y-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
            {/* Header */}
            <div className="flex items-center gap-3">
                <div className="p-2.5 rounded-xl bg-cyan-500/10 border border-cyan-500/20">
                    <MessagesSquare className="w-5 h-5 text-primary" />
                </div>
                <div>
                    <h1 className="text-xl font-bold">Session Search</h1>
                    <p className="text-xs text-muted-foreground">
                        Full-text search across stored conversation transcripts (local runtime)
                    </p>
                </div>
            </div>

            {/* Search bar */}
            <div className="flex items-center gap-2">
                <div className="flex-1 flex items-center gap-2 rounded-xl border border-border/40 bg-card/30 backdrop-blur-md px-3 py-2">
                    <Search className="w-4 h-4 text-muted-foreground shrink-0" />
                    <input
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                        onKeyDown={(e) => { if (e.key === 'Enter') search(); }}
                        placeholder="Search transcripts…"
                        className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/60"
                    />
                </div>
                <button
                    onClick={() => setSummarize((s) => !s)}
                    title="Summarize matching sessions with the primary model"
                    className={cn(
                        'inline-flex items-center gap-1.5 px-3 py-2 rounded-xl text-xs font-medium border transition-all',
                        summarize ? 'bg-primary/10 text-primary border-primary/20' : 'bg-white/[0.03] text-muted-foreground border-white/5',
                    )}
                >
                    <Sparkles className="w-3.5 h-3.5" /> Summarize
                </button>
                <button
                    onClick={search}
                    disabled={isLoading || !query.trim()}
                    className="inline-flex items-center gap-1.5 px-4 py-2 rounded-xl text-xs font-medium bg-primary/15 text-primary border border-primary/20 hover:bg-primary/20 disabled:opacity-50 transition-all"
                >
                    {isLoading ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Search className="w-3.5 h-3.5" />}
                    Search
                </button>
            </div>

            {/* Typed gate / error */}
            {notice && (
                <div className="rounded-xl border border-amber-500/20 bg-amber-500/5 px-4 py-3 flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-400 mt-0.5 shrink-0" />
                    <div>
                        <p className="text-xs text-amber-200/90">{notice.reason}</p>
                        {notice.remediation && <p className="text-[11px] text-amber-200/60 mt-0.5">{notice.remediation}</p>}
                    </div>
                </div>
            )}

            {/* Results */}
            {!notice && searched && (
                <div className="space-y-2">
                    {meta && (
                        <p className="text-[10px] uppercase tracking-widest text-muted-foreground">
                            {results.length} result{results.length === 1 ? '' : 's'}
                            {meta.summarized ? ' · summarized' : meta.fallback ? ' · raw excerpts' : ''}
                        </p>
                    )}
                    {results.length === 0 && !isLoading ? (
                        <p className="text-xs text-muted-foreground">No matching transcripts.</p>
                    ) : (
                        results.map((r, i) => {
                            const title = field(r, 'session_key', 'thread_id', 'session_id', 'channel') || 'session';
                            const excerpt = field(r, 'excerpt', 'snippet', 'preview', 'summary', 'text');
                            const score = field(r, 'score', 'rank');
                            return (
                                <div key={i} className="rounded-xl border border-white/5 bg-white/[0.02] px-3 py-2.5">
                                    <div className="flex items-center justify-between gap-2">
                                        <span className="text-xs font-mono text-foreground/80 truncate">{title}</span>
                                        {score && <span className="text-[10px] tabular-nums text-muted-foreground shrink-0">{score}</span>}
                                    </div>
                                    {excerpt && <p className="text-xs text-foreground/70 mt-1 line-clamp-3 whitespace-pre-wrap">{excerpt}</p>}
                                </div>
                            );
                        })
                    )}
                </div>
            )}
        </motion.div>
    );
}
