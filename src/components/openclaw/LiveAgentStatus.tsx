
import { motion, AnimatePresence } from 'framer-motion';
import { Loader2, Terminal, CheckCircle2, ChevronRight, ChevronDown, Brain, Zap, Globe, MousePointer2, FileCode, Search, Image as ImageIcon } from 'lucide-react';
import { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cn } from '../../lib/utils';
import { StreamRun } from '../../hooks/use-openclaw-stream';
import { ApprovalCard } from './ApprovalCard';

interface LiveAgentStatusProps {
    run: StreamRun;
    persistent?: boolean;
}

export function LiveAgentStatus({ run, persistent = false }: LiveAgentStatusProps) {
    const [expanded, setExpanded] = useState(true);
    const [progress, setProgress] = useState<{ message: string, value: number } | null>(null);

    // Auto-collapse when done, unless persistent
    useEffect(() => {
        if (!persistent && (run.status === 'completed' || run.status === 'failed')) {
            const t = setTimeout(() => setExpanded(false), 2000); // 2s delay then collapse/fade
            return () => clearTimeout(t);
        }
    }, [run.status, persistent]);

    // Listen for Sidecar Progress (Reading Context, etc)
    useEffect(() => {
        if (run.status !== 'running') return;

        const unlisten = listen<any>("sidecar_event", (event) => {
            if (event.payload.type === 'Progress' && event.payload.service === 'chat') {
                setProgress({ message: event.payload.message, value: event.payload.progress });
                if (event.payload.progress >= 0.99) {
                    setTimeout(() => setProgress(null), 800);
                }
            }
        });

        return () => { unlisten.then(f => f()); }
    }, [run.status]);

    // Clear progress when text starts streaming
    useEffect(() => {
        if (run.text && run.text.length > 0) {
            setProgress(null);
        }
    }, [run.text]);

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={persistent ? undefined : { opacity: 0, scale: 0.95 }}
            className={cn(
                "w-full max-w-3xl mx-auto my-4",
                persistent ? "mb-2" : ""
            )}
        >
            <div className={cn(
                "backdrop-blur-md rounded-xl border shadow-2xl overflow-hidden",
                persistent ? "bg-black/40 border-white/5" : "bg-gradient-to-br from-black/80 to-zinc-900/80 border-white/10"
            )}>
                {/* Header / Status Bar */}
                <div
                    className="flex items-center justify-between px-4 py-3 bg-white/5 border-b border-white/5 cursor-pointer hover:bg-white/10 transition-colors"
                    onClick={() => setExpanded(!expanded)}
                >
                    <div className="flex items-center gap-3">
                        <div className={cn(
                            "w-8 h-8 rounded-full flex items-center justify-center shadow-inner",
                            run.status === 'running' ? "bg-blue-500/20 text-blue-400" :
                                run.status === 'completed' ? "bg-green-500/20 text-green-400" :
                                    run.status === 'failed' ? "bg-red-500/20 text-red-400" : "bg-gray-500/20"
                        )}>
                            {run.status === 'running' ? <Loader2 className="w-4 h-4 animate-spin" /> :
                                run.status === 'completed' ? <CheckCircle2 className="w-4 h-4" /> :
                                    <Zap className="w-4 h-4" />}
                        </div>
                        <div>
                            <h3 className="text-xs font-bold uppercase tracking-widest text-white/90">
                                {run.status === 'running' ? 'Agent Working' : run.status === 'failed' && run.error ? 'Agent Error' : 'Agent Finished'}
                            </h3>
                            <p className="text-[10px] text-white/50 font-mono mt-0.5">
                                Run ID: {run.id.split('-').pop()} • {run.tools.length} Actions
                            </p>
                        </div>
                    </div>
                    {expanded ? <ChevronDown className="w-4 h-4 text-white/30" /> : <ChevronRight className="w-4 h-4 text-white/30" />}
                </div>

                {/* Progress Bar Override */}
                <AnimatePresence>
                    {progress && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: 'auto' }}
                            exit={{ opacity: 0, height: 0 }}
                            className="px-4 py-2 bg-blue-500/10 border-b border-blue-500/10 flex items-center gap-3 overflow-hidden"
                        >
                            <div className="w-4 h-4 flex items-center justify-center shrink-0">
                                <Loader2 className="w-3 h-3 text-blue-400 animate-spin" />
                            </div>
                            <div className="flex-1 w-full">
                                <div className="flex justify-between text-[10px] text-blue-200 uppercase font-bold mb-1">
                                    <span>{progress.message}</span>
                                    <span>{Math.round(progress.value * 100)}%</span>
                                </div>
                                <div className="h-1 bg-blue-900/50 rounded-full overflow-hidden w-full">
                                    <motion.div
                                        className="h-full bg-blue-500"
                                        initial={{ width: 0 }}
                                        animate={{ width: `${progress.value * 100}%` }}
                                        transition={{ duration: 0.2 }}
                                    />
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* Body Content */}
                <AnimatePresence>
                    {expanded && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: 'auto', opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            className="bg-black/20"
                        >
                            <div className="p-4 space-y-3">
                                {/* Tool Log */}
                                <div className="space-y-2">
                                    {run.tools.map((tool, idx) => (
                                        <div key={idx} className="group relative pl-4 border-l-2 border-white/10 hover:border-blue-500/50 transition-colors">
                                            <div className="flex items-center gap-2 mb-1">
                                                <Terminal className="w-3 h-3 text-blue-400" />
                                                <span className="text-[11px] font-bold text-blue-100 uppercase tracking-wider">{tool.tool}</span>
                                                <span className={cn(
                                                    "text-[9px] px-1.5 py-0.5 rounded uppercase font-bold",
                                                    tool.status === 'running' ? "bg-blue-500/20 text-blue-300 animate-pulse" :
                                                        tool.status === 'completed' ? "bg-green-500/10 text-green-500" :
                                                            "bg-red-500/10 text-red-400"
                                                )}>
                                                    {tool.status}
                                                </span>
                                            </div>
                                            {/* Tool Details */}
                                            <ToolDetail tool={tool.tool} input={tool.input} output={tool.output} />
                                        </div>
                                    ))}
                                </div>

                                {/* Live Text Stream Preview */}
                                {run.text && (
                                    <div className="mt-4 pt-4 border-t border-white/5">
                                        <div className="flex items-center gap-2 mb-2 text-white/40">
                                            <Brain className="w-3 h-3" />
                                            <span className="text-[10px] uppercase font-bold tracking-wider">Live Response</span>
                                        </div>
                                        <div className="text-sm text-white/80 leading-relaxed pl-5 border-l-2 border-purple-500/30">
                                            {run.text.slice(-500)}
                                            {run.status === 'running' && <span className="inline-block w-1.5 h-3 bg-purple-400 ml-1 animate-pulse" />}
                                        </div>
                                    </div>
                                )}

                                {/* Approval Requests */}
                                <AnimatePresence>
                                    {run.approvals.filter(a => a.status === 'pending').map(approval => (
                                        <ApprovalCard
                                            key={approval.id}
                                            approvalId={approval.id}
                                            tool={approval.tool}
                                            input={approval.input}
                                        />
                                    ))}
                                </AnimatePresence>

                                {/* Error Banner */}
                                {run.status === 'failed' && run.error && (
                                    <div className="mt-3 p-3 rounded-lg bg-red-500/10 border border-red-500/20">
                                        <div className="flex items-center gap-2 mb-1">
                                            <Zap className="w-3.5 h-3.5 text-red-400" />
                                            <span className="text-[10px] font-bold uppercase tracking-wider text-red-400">Error Details</span>
                                        </div>
                                        <p className="text-xs text-red-300/90 font-mono leading-relaxed pl-5 border-l-2 border-red-500/30 whitespace-pre-wrap">
                                            {run.error}
                                        </p>
                                    </div>
                                )}
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>
        </motion.div>
    );
}

function ToolDetail({ tool, input, output }: { tool: string, input: any, output: any }) {
    if (!input && !output) return null;

    const [expanded, setExpanded] = useState(false);

    // Browser Tools
    if (tool.startsWith('browser_')) {
        const url = input?.url || input;
        return (
            <div className="space-y-2">
                <div className="flex items-center gap-2 text-[10px] text-blue-400/80 bg-blue-500/5 px-2 py-1 rounded border border-blue-500/10">
                    <Globe className="w-3 h-3" />
                    <span className="truncate flex-1">{typeof url === 'string' ? url : 'Browser Action'}</span>
                    {tool === 'browser_navigate' && <span className="text-white/40">Navigate</span>}
                    {tool === 'browser_click' && <span className="text-white/40">Click</span>}
                </div>
                {output?.screenshot && (
                    <div className="relative group rounded-lg overflow-hidden border border-white/10 bg-black/40 aspect-video flex items-center justify-center">
                        <img src={`data:image/png;base64,${output.screenshot}`} alt="Browser Screenshot" className="max-h-full object-contain" />
                        <div className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
                            <MousePointer2 className="w-6 h-6 text-white animate-bounce" />
                        </div>
                    </div>
                )}
            </div>
        );
    }

    // Canvas / Display Image
    if (tool === 'display_canvas' || tool === 'canvas') {
        const imageData = output?.image || input?.image;
        if (imageData) {
            return (
                <div className="space-y-2">
                    <div className="flex items-center gap-2 text-[10px] text-pink-400/80 bg-pink-500/5 px-2 py-1 rounded border border-pink-500/10">
                        <ImageIcon className="w-3 h-3" />
                        <span>Canvas View</span>
                    </div>
                    <div className="rounded-lg overflow-hidden border border-white/10 bg-black/40 p-2">
                        <img src={`data:image/png;base64,${imageData}`} alt="Canvas Display" className="w-full h-auto rounded" />
                    </div>
                </div>
            );
        }
    }

    // Apply Patch / Write File
    if (tool === 'apply_patch' || tool === 'write_file') {
        const path = input?.path || 'File';
        const patch = input?.patch || input;
        return (
            <div className="space-y-2">
                <div className="flex items-center gap-2 text-[10px] text-emerald-400/80 bg-emerald-500/5 px-2 py-1 rounded border border-emerald-500/10">
                    <FileCode className="w-3 h-3" />
                    <span className="truncate">{path}</span>
                </div>
                <button
                    onClick={() => setExpanded(!expanded)}
                    className="w-full text-left text-[10px] font-mono text-white/40 bg-black/40 p-2 rounded overflow-x-auto whitespace-pre-wrap hover:text-white/80 transition-colors"
                >
                    {expanded ? 'Hide code change' : 'View code change...'}
                </button>
                {expanded && (
                    <pre className="text-[10px] font-mono text-emerald-300/60 bg-black/60 p-3 rounded-lg border border-white/5 overflow-x-auto whitespace-pre-wrap animate-in slide-in-from-top-1 duration-200">
                        {typeof patch === 'string' ? patch : JSON.stringify(patch, null, 2)}
                    </pre>
                )}
            </div>
        );
    }

    // Search Tools
    if (tool.includes('search')) {
        return (
            <div className="space-y-2">
                <div className="flex items-center gap-2 text-[10px] text-amber-400/80 bg-amber-500/5 px-2 py-1 rounded border border-amber-500/10">
                    <Search className="w-3 h-3" />
                    <span className="truncate">{input?.query || JSON.stringify(input)}</span>
                </div>
            </div>
        );
    }

    // Default Fallback
    return (
        <pre className="text-[10px] font-mono text-white/40 bg-black/40 p-2 rounded overflow-x-auto whitespace-pre-wrap max-h-20 hover:max-h-60 transition-all opacity-80 hover:opacity-100">
            {typeof input === 'string' ? input : JSON.stringify(input, null, 2)}
        </pre>
    );
}
