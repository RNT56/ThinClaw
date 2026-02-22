
import { memo } from 'react';
import { Handle, Position, NodeProps } from '@xyflow/react';
import { cn } from '../../../lib/utils';
import { Activity, Brain, AlertTriangle } from 'lucide-react';
import { motion } from 'framer-motion';

const AgentNode = ({ data, selected }: NodeProps) => {
    const isOnline = data.online as boolean;
    const isActive = data.active as boolean;
    const label = data.label as string;
    const task = data.task as string;
    const progress = data.progress as number;
    const status = (data.status as string) || 'idle';
    const model = data.model as string | null;

    const statusDotColor = (() => {
        switch (status) {
            case 'processing': return 'bg-indigo-500 shadow-[0_0_8px_rgba(99,102,241,0.8)]';
            case 'waiting_approval': return 'bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.8)] animate-pulse';
            case 'error': return 'bg-red-500';
            case 'offline': return 'bg-red-500';
            case 'idle': return 'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.8)]';
            default: return 'bg-zinc-500';
        }
    })();

    const borderGlow = (() => {
        if (!isOnline) return 'border-zinc-800';
        if (selected) return 'border-indigo-500 shadow-[0_0_30px_-5px_rgba(99,102,241,0.6)]';
        if (status === 'processing') return 'border-indigo-500/50';
        if (status === 'waiting_approval') return 'border-amber-500/50';
        return 'border-zinc-800';
    })();

    return (
        <div className={cn(
            "relative w-64 rounded-xl border-2 transition-all duration-300 bg-black/80 backdrop-blur-md overflow-hidden",
            borderGlow,
            isOnline ? "opacity-100" : "opacity-50 grayscale"
        )}>
            {/* Header */}
            <div className={cn(
                "p-3 flex items-center justify-between border-b",
                isOnline ? "border-white/10" : "border-white/5",
                status === 'processing' && "bg-indigo-500/10",
                status === 'waiting_approval' && "bg-amber-500/10"
            )}>
                <div className="flex items-center gap-2">
                    <div className={cn("w-2 h-2 rounded-full", statusDotColor)} />
                    <span className="font-bold text-xs uppercase tracking-wider text-zinc-100">{label}</span>
                </div>
                {status === 'processing' && (
                    <motion.div
                        animate={{ rotate: 360 }}
                        transition={{ duration: 2, repeat: Infinity, ease: "linear" }}
                    >
                        <Activity className="w-3.5 h-3.5 text-indigo-400" />
                    </motion.div>
                )}
                {status === 'waiting_approval' && (
                    <AlertTriangle className="w-3.5 h-3.5 text-amber-400 animate-pulse" />
                )}
            </div>

            {/* Body */}
            <div className="p-4 space-y-3">
                <div className="flex items-center gap-3">
                    <div className="p-2 bg-zinc-900 rounded-lg border border-white/5">
                        <Brain className="w-5 h-5 text-zinc-400" />
                    </div>
                    <div className="flex-1 min-w-0">
                        <div className="text-[10px] text-zinc-500 uppercase">
                            {status === 'waiting_approval' ? 'Awaiting Approval' : 'Current Operation'}
                        </div>
                        <div className="text-xs text-zinc-300 truncate font-mono">
                            {task || "Idle / Awaiting"}
                        </div>
                    </div>
                </div>

                {/* Model */}
                {model && (
                    <div className="text-[10px] text-zinc-500 font-mono truncate px-1">
                        ⚡ {model}
                    </div>
                )}

                {/* Progress Bar */}
                {(isActive || progress > 0) && (
                    <div className="space-y-1">
                        <div className="flex justify-between text-[10px] text-zinc-500">
                            <span>{status === 'processing' ? 'Running' : 'Progress'}</span>
                            {progress > 0 && <span>{Math.round(progress * 100)}%</span>}
                        </div>
                        <div className="h-1 w-full bg-zinc-800 rounded-full overflow-hidden">
                            {progress > 0 ? (
                                <motion.div
                                    className={cn(
                                        "h-full",
                                        status === 'waiting_approval'
                                            ? "bg-amber-500"
                                            : "bg-indigo-500"
                                    )}
                                    initial={{ width: 0 }}
                                    animate={{ width: `${progress * 100}%` }}
                                    transition={{ type: "spring", stiffness: 50 }}
                                />
                            ) : (
                                // Indeterminate progress bar
                                <motion.div
                                    className="h-full bg-indigo-500/60 w-1/3"
                                    animate={{ x: ['0%', '200%', '0%'] }}
                                    transition={{ duration: 2, repeat: Infinity, ease: "easeInOut" }}
                                />
                            )}
                        </div>
                    </div>
                )}
            </div>

            {/* Ports */}
            <Handle type="target" position={Position.Top} className="!bg-zinc-500 !w-3 !h-1 !rounded-sm !-top-1.5" />
            <Handle type="source" position={Position.Bottom} className="!bg-indigo-500 !w-3 !h-1 !rounded-sm !-bottom-1.5" />
        </div>
    );
};

export default memo(AgentNode);
