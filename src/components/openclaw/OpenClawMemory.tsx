import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import {
    History,
    Calendar,
    Search,
    RefreshCw,
    ChevronRight,
    BookOpen,
    Clock,
    Zap,
    Filter
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

export function OpenClawMemory() {
    const [logs, setLogs] = useState<string[]>([]);
    const [activeLog, setActiveLog] = useState<string | null>(null);
    const [content, setContent] = useState('');
    const [isLoading, setIsLoading] = useState(true);
    const [search, setSearch] = useState('');

    const fetchLogs = async () => {
        try {
            const list = await openclaw.listWorkspaceFiles();
            const memoryLogs = list.filter(f => f.startsWith('memory/')).sort().reverse();
            setLogs(memoryLogs);
            if (memoryLogs.length > 0 && !activeLog) {
                handleSelectLog(memoryLogs[0]);
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

    useEffect(() => {
        fetchLogs();
    }, []);

    const filteredLogs = logs.filter(f => f.toLowerCase().includes(search.toLowerCase()));

    return (
        <motion.div
            initial={{ opacity: 0, x: 20 }}
            animate={{ opacity: 1, x: 0 }}
            className="flex-1 flex overflow-hidden h-[calc(100vh-100px)]"
        >
            {/* Sidebar: Log List */}
            <div className="w-72 border-r border-white/5 flex flex-col bg-black/10">
                <div className="p-6 border-b border-white/5 space-y-4">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                            <History className="w-5 h-5 text-primary" />
                            <h2 className="text-sm font-bold uppercase tracking-wider">Temporal Logs</h2>
                        </div>
                        <span className="text-[10px] px-2 py-0.5 rounded-full bg-primary/10 text-primary font-mono border border-primary/20">
                            {logs.length}
                        </span>
                    </div>
                    <div className="relative">
                        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
                        <input
                            type="text"
                            placeholder="Search by date..."
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                            className="w-full pl-9 pr-3 py-2 bg-white/5 border border-white/10 rounded-xl text-xs focus:ring-1 focus:ring-primary/40 outline-none transition-all"
                        />
                    </div>
                </div>

                <div className="flex-1 overflow-y-auto p-3 space-y-1">
                    {filteredLogs.map(log => {
                        const dateStr = log.replace('memory/', '').replace('.md', '');
                        return (
                            <button
                                key={log}
                                onClick={() => handleSelectLog(log)}
                                className={cn(
                                    "w-full flex items-center gap-3 px-4 py-3 rounded-xl text-xs font-medium transition-all group",
                                    activeLog === log
                                        ? "bg-primary text-primary-foreground shadow-lg shadow-primary/20 ring-1 ring-primary/50"
                                        : "text-muted-foreground hover:bg-white/5 hover:text-foreground border border-transparent"
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
                </div>

                <div className="p-4 border-t border-white/5">
                    <button
                        onClick={fetchLogs}
                        className="w-full flex items-center justify-center gap-2 py-2 rounded-xl bg-white/5 hover:bg-white/10 text-[10px] font-bold uppercase tracking-widest transition-all"
                    >
                        <RefreshCw className={cn("w-3.5 h-3.5", isLoading && "animate-spin")} />
                        Refresh Memory
                    </button>
                </div>
            </div>

            {/* Main Content: Viewer */}
            <div className="flex-1 flex flex-col bg-[#0D0D0E]">
                {activeLog ? (
                    <>
                        <div className="p-6 flex items-center justify-between border-b border-white/5 bg-black/20">
                            <div className="flex items-center gap-4">
                                <div className="p-2.5 bg-primary/10 rounded-xl">
                                    <BookOpen className="w-5 h-5 text-primary" />
                                </div>
                                <div>
                                    <h1 className="text-lg font-bold tracking-tight">{activeLog.replace('memory/', '')}</h1>
                                    <div className="flex items-center gap-3 mt-0.5">
                                        <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground uppercase font-bold tracking-tighter">
                                            <Clock className="w-3 h-3" />
                                            Daily Summary
                                        </div>
                                        <div className="w-1 h-1 rounded-full bg-white/10" />
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
                                    <div className="prose prose-invert prose-zinc max-w-none">
                                        <div className="whitespace-pre-wrap font-sans text-sm leading-relaxed text-zinc-300 selection:bg-primary/30">
                                            {content || "This memory log is intentionally left blank."}
                                        </div>
                                    </div>
                                </div>
                            )}
                        </div>
                    </>
                ) : (
                    <div className="flex-1 flex flex-col items-center justify-center p-12 text-center space-y-8">
                        <div className="relative">
                            <div className="absolute inset-0 animate-ping bg-primary/20 rounded-full blur-xl" />
                            <div className="relative p-8 rounded-full bg-primary/5 border border-primary/10 shadow-2xl">
                                <History className="w-16 h-16 text-primary/40" />
                            </div>
                        </div>
                        <div className="space-y-3 max-w-md">
                            <h2 className="text-2xl font-bold tracking-tight">Temporal Reflective Ledger</h2>
                            <p className="text-muted-foreground text-sm leading-relaxed">
                                Experience the agent's growth through daily append-only logs.
                                These files capture every interaction, tool result, and long-term reflection.
                            </p>
                        </div>
                        <div className="p-6 rounded-2xl border border-white/5 bg-white/[0.02] flex items-start gap-4 text-left max-w-lg">
                            <div className="p-2 bg-blue-500/10 rounded-lg shrink-0">
                                <Search className="w-5 h-5 text-blue-400" />
                            </div>
                            <div>
                                <h4 className="text-xs font-bold text-blue-400 uppercase tracking-widest mb-1">Semantic Search Note</h4>
                                <p className="text-xs text-muted-foreground leading-relaxed">
                                    While you browse these as raw text, the agent uses vector embeddings to semantically search these files during chat turn reasoning.
                                </p>
                            </div>
                        </div>
                    </div>
                )}
            </div>
        </motion.div>
    );
}
