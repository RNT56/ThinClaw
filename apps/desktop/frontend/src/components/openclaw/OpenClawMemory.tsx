import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    History,
    Calendar,
    Search,
    RefreshCw,
    ChevronRight,
    BookOpen,
    Clock,
    Zap,
    Filter,
    Sparkles,
    FileText,
    HardDrive,
    Brain,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';
import { OpenClawModeBadge, useOpenClawStatusSnapshot } from './OpenClawModeBadge';

export function OpenClawMemory() {
    const [logs, setLogs] = useState<string[]>([]);
    const [activeLog, setActiveLog] = useState<string | null>(null);
    const [content, setContent] = useState('');
    const [isLoading, setIsLoading] = useState(true);
    const [search, setSearch] = useState('');
    const [searchMode, setSearchMode] = useState<'files' | 'semantic'>('files');
    const [searchResults, setSearchResults] = useState<openclaw.MemorySearchResult[]>([]);
    const [isSearching, setIsSearching] = useState(false);
    const [searchError, setSearchError] = useState<string | null>(null);
    const { status: runtimeStatus } = useOpenClawStatusSnapshot(15000);

    const fetchLogs = async () => {
        try {
            const list = await openclaw.listWorkspaceFiles();
            const dailyLogs = list.filter(f => f.startsWith('daily/')).sort().reverse();
            setLogs(dailyLogs);
            if (dailyLogs.length > 0 && !activeLog) {
                handleSelectLog(dailyLogs[0]);
            }
        } catch (e) {
            console.error('Failed to fetch memory logs:', e);
            toast.error('Failed to load memory folder');
        } finally {
            setIsLoading(false);
        }
    };

    const handleSelectLog = async (path: string) => {
        try {
            setActiveLog(path);
            setIsLoading(true);
            const data = await openclaw.getOpenClawFile(path);
            setContent(data);
        } catch (e) {
            toast.error(`Failed to read log ${path}`);
        } finally {
            setIsLoading(false);
        }
    };

    const handleSemanticSearch = useCallback(async () => {
        if (!search.trim() || search.trim().length < 2) return;
        setIsSearching(true);
        try {
            const resp = await openclaw.searchMemory(search.trim(), 20);
            setSearchResults(resp.results);
            setSearchError(null);
            if (resp.results.length === 0) {
                toast.info('No matching memories found');
            }
        } catch (e) {
            console.error('Memory search failed:', e);
            setSearchError(String(e));
            toast.error('Memory search failed');
        } finally {
            setIsSearching(false);
        }
    }, [search]);

    useEffect(() => {
        fetchLogs();
    }, []);

    // Debounced semantic search
    useEffect(() => {
        if (searchMode !== 'semantic' || !search.trim() || search.trim().length < 3) {
            setSearchResults([]);
            return;
        }
        const timeout = setTimeout(handleSemanticSearch, 500);
        return () => clearTimeout(timeout);
    }, [search, searchMode, handleSemanticSearch]);

    const filteredLogs = logs.filter(f => f.toLowerCase().includes(search.toLowerCase()));

    return (
        <motion.div
            initial={{ opacity: 0, x: 20 }}
            animate={{ opacity: 1, x: 0 }}
            className="flex-1 flex overflow-hidden h-[calc(100vh-100px)]"
        >
            {/* Sidebar: Log List + Search */}
            <div className="w-72 border-r border-border/30 flex flex-col bg-muted/10">
                <div className="p-6 border-b border-border/30 space-y-4">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                            <History className="w-5 h-5 text-primary" />
                            <h2 className="text-sm font-bold uppercase tracking-wider">Temporal Logs</h2>
                        </div>
                        <div className="flex items-center gap-2">
                            <OpenClawModeBadge status={runtimeStatus} compact />
                            <span className="text-[10px] px-2 py-0.5 rounded-full bg-primary/10 text-primary font-mono border border-primary/20">
                                {logs.length}
                            </span>
                        </div>
                    </div>

                    {/* Search Mode Toggle */}
                    <div className="flex p-0.5 bg-muted/30 rounded-lg border border-border/40">
                        <button
                            onClick={() => setSearchMode('files')}
                            className={cn(
                                "flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded-md text-[10px] font-bold uppercase tracking-wider transition-all",
                                searchMode === 'files'
                                    ? "bg-primary/15 text-primary shadow-sm"
                                    : "text-muted-foreground hover:text-foreground"
                            )}
                        >
                            <FileText className="w-3 h-3" />
                            Files
                        </button>
                        <button
                            onClick={() => setSearchMode('semantic')}
                            className={cn(
                                "flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded-md text-[10px] font-bold uppercase tracking-wider transition-all",
                                searchMode === 'semantic'
                                    ? "bg-violet-500/15 text-primary shadow-sm"
                                    : "text-muted-foreground hover:text-foreground"
                            )}
                        >
                            <Sparkles className="w-3 h-3" />
                            Search
                        </button>
                    </div>

                    <div className="relative">
                        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
                        <input
                            type="text"
                            placeholder={searchMode === 'semantic' ? "Semantic search memories..." : "Search by date..."}
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                            onKeyDown={(e) => { if (e.key === 'Enter' && searchMode === 'semantic') handleSemanticSearch(); }}
                            className={cn(
                                "w-full pl-9 pr-3 py-2 bg-white/5 border rounded-xl text-xs focus:ring-1 outline-none transition-all",
                                searchMode === 'semantic'
                                    ? "border-violet-500/20 focus:ring-violet-500/40"
                                    : "border-border/40 focus:ring-primary/40"
                            )}
                        />
                        {isSearching && (
                            <RefreshCw className="absolute right-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-primary animate-spin" />
                        )}
                    </div>
                    {searchError && (
                        <div className="rounded-lg border border-amber-500/20 bg-amber-500/10 px-3 py-2 text-[10px] text-amber-300">
                            {searchError}
                        </div>
                    )}
                </div>

                <div className="flex-1 overflow-y-auto p-3 space-y-1">
                    <AnimatePresence mode="wait">
                        {searchMode === 'semantic' && searchResults.length > 0 ? (
                            <motion.div
                                key="search-results"
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                exit={{ opacity: 0 }}
                                className="space-y-1.5"
                            >
                                <div className="px-2 py-1.5 text-[10px] font-bold text-primary uppercase tracking-widest">
                                    {searchResults.length} memory result{searchResults.length !== 1 ? 's' : ''}
                                </div>
                                {searchResults.map((result, i) => (
                                    <button
                                        key={`${result.path}-${i}`}
                                        onClick={() => handleSelectLog(result.path)}
                                        className={cn(
                                            "w-full flex flex-col gap-1.5 px-3 py-3 rounded-xl text-xs transition-all border",
                                            activeLog === result.path
                                                ? "bg-violet-500/10 border-violet-500/30 shadow-sm"
                                                : "border-transparent hover:bg-muted/30 hover:border-border/40"
                                        )}
                                    >
                                        <div className="flex items-center justify-between w-full">
                                            <span className="font-mono text-[10px] text-primary truncate flex-1">
                                                {result.path.replace('daily/', '').replace('.md', '')}
                                            </span>
                                            <span className="text-[9px] font-mono text-muted-foreground ml-2 shrink-0">
                                                {(result.score * 100).toFixed(0)}%
                                            </span>
                                        </div>
                                        <p className="text-[10px] text-muted-foreground line-clamp-2 text-left leading-relaxed">
                                            {result.snippet}
                                        </p>
                                    </button>
                                ))}
                            </motion.div>
                        ) : (
                            <motion.div
                                key="file-list"
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                exit={{ opacity: 0 }}
                            >
                                {filteredLogs.map(log => {
                                    const dateStr = log.replace('daily/', '').replace('.md', '');
                                    return (
                                        <button
                                            key={log}
                                            onClick={() => handleSelectLog(log)}
                                            className={cn(
                                                "w-full flex items-center gap-3 px-4 py-3 rounded-xl text-xs font-medium transition-all group",
                                                activeLog === log
                                                    ? "bg-primary text-primary-foreground shadow-lg shadow-primary/20 ring-1 ring-primary/50"
                                                    : "text-muted-foreground hover:bg-muted/30 hover:text-foreground border border-transparent"
                                            )}
                                        >
                                            <Calendar className={cn("w-4 h-4", activeLog === log ? "text-primary-foreground" : "text-primary/60")} />
                                            <span className="flex-1 text-left font-mono">{dateStr}</span>
                                            {activeLog === log && <ChevronRight className="w-3.5 h-3.5" />}
                                        </button>
                                    );
                                })}
                                {filteredLogs.length === 0 && !isLoading && (
                                    <div className="py-12 flex flex-col items-center justify-center text-center opacity-40">
                                        <Filter className="w-8 h-8 mb-2" />
                                        <p className="text-[10px] uppercase font-bold tracking-widest">No logs match criteria</p>
                                    </div>
                                )}
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>

                <div className="p-4 border-t border-border/30">
                    <button
                        onClick={fetchLogs}
                        className="w-full flex items-center justify-center gap-2 py-2 rounded-xl bg-muted/30 hover:bg-muted/50 text-[10px] font-bold uppercase tracking-widest transition-all"
                    >
                        <RefreshCw className={cn("w-3.5 h-3.5", isLoading && "animate-spin")} />
                        Refresh Memory
                    </button>
                </div>
            </div>

            {/* Main Content: Viewer */}
            <div className="flex-1 flex flex-col bg-card">
                {activeLog ? (
                    <>
                        <div className="p-6 flex items-center justify-between border-b border-border/30 bg-muted/10">
                            <div className="flex items-center gap-4">
                                <div className="p-2.5 bg-primary/10 rounded-xl">
                                    <BookOpen className="w-5 h-5 text-primary" />
                                </div>
                                <div>
                                    <h1 className="text-lg font-bold tracking-tight">{activeLog.replace('daily/', '')}</h1>
                                    <div className="flex items-center gap-3 mt-0.5">
                                        <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground uppercase font-bold tracking-tighter">
                                            <Clock className="w-3 h-3" />
                                            Daily Summary
                                        </div>
                                        <div className="w-1 h-1 rounded-full bg-border" />
                                        <div className="flex items-center gap-1.5 text-[10px] text-green-500 uppercase font-bold tracking-tighter">
                                            <Zap className="w-3 h-3" />
                                            Persistent Ledger
                                        </div>
                                    </div>
                                </div>
                            </div>
                        </div>
                        <div className="flex-1 overflow-y-auto relative">
                            {isLoading ? (
                                <div className="absolute inset-0 flex items-center justify-center bg-black/40 backdrop-blur-sm z-10">
                                    <RefreshCw className="w-10 h-10 text-primary animate-spin" />
                                </div>
                            ) : (
                                <div className="max-w-4xl mx-auto p-12">
                                    <div className="prose prose-sm dark:prose-invert prose-zinc max-w-none">
                                        <div className="whitespace-pre-wrap font-sans text-sm leading-relaxed text-foreground/70 selection:bg-primary/30">
                                            {content || "This memory log is intentionally left blank."}
                                        </div>
                                    </div>
                                </div>
                            )}
                        </div>
                    </>
                ) : (
                    <div className="flex-1 flex flex-col items-center justify-center p-10 text-center space-y-6 max-w-xl mx-auto">
                        <div className="relative">
                            <div className="absolute inset-0 animate-ping bg-primary/10 rounded-full blur-xl" />
                            <div className="relative p-7 rounded-full bg-primary/5 border border-primary/10 shadow-2xl">
                                <History className="w-14 h-14 text-primary/40" />
                            </div>
                        </div>
                        <div className="space-y-2">
                            <h2 className="text-2xl font-bold tracking-tight">Temporal Reflective Ledger</h2>
                            <p className="text-muted-foreground text-sm leading-relaxed">
                                Daily memory logs will appear here as the agent accumulates experience over time.
                            </p>
                        </div>
                        {/* Explain the two storage systems */}
                        <div className="grid gap-3 w-full text-left">
                            <div className="p-4 rounded-xl border border-border/30 bg-muted/10 flex items-start gap-3">
                                <div className="p-2 bg-primary/10 rounded-lg shrink-0"><Brain className="w-4 h-4 text-primary" /></div>
                                <div>
                                    <h4 className="text-[10px] font-bold text-primary uppercase tracking-widest mb-1">Temporal Memory (this panel)</h4>
                                    <p className="text-[11px] text-muted-foreground leading-relaxed">
                                        Shows <code className="bg-muted/30 px-1 rounded text-[10px]">daily/YYYY-MM-DD.md</code> daily session logs created automatically during conversations.
                                        These are searchable by both text and semantic (vector) similarity.
                                        They'll appear once the agent has had its first conversation.
                                    </p>
                                </div>
                            </div>
                            <div className="p-4 rounded-xl border border-border/30 bg-muted/10 flex items-start gap-3">
                                <div className="p-2 bg-emerald-500/10 rounded-lg shrink-0"><HardDrive className="w-4 h-4 text-primary" /></div>
                                <div>
                                    <h4 className="text-[10px] font-bold text-primary uppercase tracking-widest mb-1">File-Created Content → The Brain</h4>
                                    <p className="text-[11px] text-muted-foreground leading-relaxed">
                                        When you ask the agent to <em>create a file</em>, it uses <code className="bg-muted/30 px-1 rounded text-[10px]">write_file</code> and it lands in the <strong>Local Files</strong> tab inside The Brain. You can reveal these directly in Finder.
                                    </p>
                                </div>
                            </div>
                            <div className="p-4 rounded-xl border border-border/30 bg-muted/10 flex items-start gap-3">
                                <div className="p-2 bg-blue-500/10 rounded-lg shrink-0"><Search className="w-4 h-4 text-blue-400" /></div>
                                <div>
                                    <h4 className="text-[10px] font-bold text-blue-400 uppercase tracking-widest mb-1">Semantic Search</h4>
                                    <p className="text-[11px] text-muted-foreground leading-relaxed">
                                        Once memory logs exist, the agent uses vector embeddings to retrieve the most relevant context from past sessions, enriching every conversation.
                                    </p>
                                </div>
                            </div>
                        </div>
                    </div>
                )}
            </div>
        </motion.div>
    );
}
