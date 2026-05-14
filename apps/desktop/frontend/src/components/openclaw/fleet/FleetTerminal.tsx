
import { useRef, useEffect } from 'react';
import { Terminal } from 'lucide-react';
import { cn } from '../../../lib/utils';

interface FleetTerminalProps {
    agentIds: string[];
    logs: { id: string, lines: string[] }[]; // Map agent ID to log lines
    className?: string;
}

export function FleetTerminal({ agentIds, logs, className }: FleetTerminalProps) {
    const scrollRef = useRef<HTMLDivElement>(null);

    // Auto-scroll to bottom
    useEffect(() => {
        if (scrollRef.current) {
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
        }
    }, [logs]);

    const activeLogs = logs.filter(l => agentIds.includes(l.id)).flatMap(l => l.lines.map(line => ({ id: l.id, text: line })));

    return (
        <div className={cn("flex flex-col bg-black border-t border-white/10 font-mono text-xs", className)}>
            <div className="flex items-center justify-between px-4 py-2 bg-zinc-900 border-b border-white/5 select-none">
                <div className="flex items-center gap-3">
                    <Terminal className="w-4 h-4 text-emerald-500" />
                    <span className="font-bold text-zinc-400">FLEET UPLINK</span>
                    <div className="flex gap-1.5 is-nav">
                        {agentIds.map(id => (
                            <span key={id} className="px-1.5 py-0.5 rounded bg-zinc-800 text-[10px] text-zinc-500 border border-white/5">
                                {id.substring(0, 8)}
                            </span>
                        ))}
                    </div>
                </div>
                <div className="flex items-center gap-2 text-zinc-600">
                    <div className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse" />
                    <span>LIVE</span>
                </div>
            </div>

            <div
                ref={scrollRef}
                className="flex-1 overflow-y-auto p-4 space-y-1 scrollbar-thin scrollbar-thumb-zinc-800 scrollbar-track-transparent"
            >
                {activeLogs.length === 0 ? (
                    <div className="text-zinc-700 italic text-center mt-10">Waiting for agent telemetry...</div>
                ) : (
                    activeLogs.map((log, i) => (
                        <div key={i} className="flex gap-3 hover:bg-white/5 px-2 py-0.5 rounded">
                            <span className="text-zinc-600 w-24 shrink-0 truncate text-right select-none">{log.id.substring(0, 8)}</span>
                            <span className="text-zinc-300 break-all">{log.text}</span>
                        </div>
                    ))
                )}
            </div>
        </div>
    );
}
