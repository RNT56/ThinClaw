import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import {
    GitBranch, RefreshCw, Zap, Info, Cpu, Layers,
    ArrowRight, CheckCircle2
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

export function OpenClawRouting() {
    const [smartRoutingEnabled, setSmartRoutingEnabled] = useState(false);
    const [isLoading, setIsLoading] = useState(true);
    const [toggling, setToggling] = useState(false);

    useEffect(() => {
        openclaw.getRoutingConfig()
            .then(cfg => setSmartRoutingEnabled(cfg.smart_routing_enabled))
            .catch(() => { })
            .finally(() => setIsLoading(false));
    }, []);

    const handleToggle = async () => {
        const next = !smartRoutingEnabled;
        setToggling(true);
        setSmartRoutingEnabled(next); // Optimistic
        try {
            await openclaw.setRoutingConfig(next);
            toast.success(next ? '🧠 Smart Routing enabled' : 'Smart Routing disabled');
        } catch (e) {
            setSmartRoutingEnabled(!next); // Rollback
            toast.error(`Failed to toggle: ${e}`);
        } finally {
            setToggling(false);
        }
    };

    return (
        <motion.div
            className="flex-1 overflow-y-auto p-8 space-y-8"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-violet-500/10 border border-violet-500/20">
                        <GitBranch className="w-5 h-5 text-violet-400" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">LLM Routing</h1>
                        <p className="text-xs text-muted-foreground">
                            Configure how requests are routed across models and providers
                        </p>
                    </div>
                </div>
            </div>

            {/* Smart Routing Toggle Card */}
            <motion.div
                initial={{ opacity: 0, y: 5 }}
                animate={{ opacity: 1, y: 0 }}
                className="rounded-2xl border border-white/10 bg-card/30 backdrop-blur-md overflow-hidden"
            >
                <div className="p-6 space-y-5">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-4">
                            <div className={cn(
                                "p-3 rounded-xl border transition-all duration-300",
                                smartRoutingEnabled
                                    ? "bg-violet-500/10 border-violet-500/20 text-violet-400"
                                    : "bg-white/5 border-white/10 text-muted-foreground"
                            )}>
                                <Zap className="w-5 h-5" />
                            </div>
                            <div>
                                <h2 className="font-semibold text-base">Smart Routing</h2>
                                <p className="text-xs text-muted-foreground mt-0.5">
                                    Automatically route requests to the optimal model based on task complexity
                                </p>
                            </div>
                        </div>

                        {/* Toggle Switch */}
                        <button
                            onClick={handleToggle}
                            disabled={isLoading || toggling}
                            className={cn(
                                "relative w-12 h-6 rounded-full transition-all duration-300 shrink-0",
                                smartRoutingEnabled
                                    ? "bg-violet-500"
                                    : "bg-zinc-700",
                                (isLoading || toggling) && "opacity-50 cursor-wait"
                            )}
                        >
                            <motion.div
                                className="absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow-md"
                                animate={{ x: smartRoutingEnabled ? 24 : 0 }}
                                transition={{ type: "spring", stiffness: 500, damping: 30 }}
                            />
                        </button>
                    </div>

                    {/* Status indicator */}
                    <div className={cn(
                        "flex items-center gap-2 px-3 py-2 rounded-lg text-xs font-medium transition-all",
                        smartRoutingEnabled
                            ? "bg-violet-500/10 text-violet-400 border border-violet-500/20"
                            : "bg-zinc-500/10 text-zinc-400 border border-zinc-500/20"
                    )}>
                        {isLoading ? (
                            <><RefreshCw className="w-3.5 h-3.5 animate-spin" /> Loading configuration…</>
                        ) : smartRoutingEnabled ? (
                            <><CheckCircle2 className="w-3.5 h-3.5" /> Smart routing is active — requests will be intelligently distributed</>
                        ) : (
                            <><Info className="w-3.5 h-3.5" /> Smart routing is disabled — all requests go to the default model</>
                        )}
                    </div>
                </div>
            </motion.div>

            {/* How It Works */}
            <motion.div
                initial={{ opacity: 0, y: 5 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.1 }}
                className="rounded-2xl border border-white/10 bg-card/30 backdrop-blur-md p-6 space-y-4"
            >
                <h3 className="text-sm font-bold uppercase tracking-widest text-muted-foreground/60 flex items-center gap-2">
                    <Cpu className="w-3.5 h-3.5" />
                    How Smart Routing Works
                </h3>

                <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    {[
                        {
                            title: 'Analyze',
                            description: 'Each request is analyzed for complexity, context length, and required capabilities.',
                            icon: Layers,
                            color: 'text-blue-400 bg-blue-500/10 border-blue-500/20',
                        },
                        {
                            title: 'Route',
                            description: 'The routing engine selects the optimal model based on cost, speed, and quality tradeoffs.',
                            icon: GitBranch,
                            color: 'text-violet-400 bg-violet-500/10 border-violet-500/20',
                        },
                        {
                            title: 'Execute',
                            description: 'The request is sent to the selected model with automatic fallback if the primary provider fails.',
                            icon: Zap,
                            color: 'text-emerald-400 bg-emerald-500/10 border-emerald-500/20',
                        },
                    ].map((step, i) => (
                        <div key={step.title} className="relative">
                            <div className="p-4 rounded-xl border border-white/5 bg-white/[0.02] space-y-3">
                                <div className={cn("p-2 rounded-lg border w-fit", step.color)}>
                                    <step.icon className="w-4 h-4" />
                                </div>
                                <div>
                                    <h4 className="text-sm font-semibold">{step.title}</h4>
                                    <p className="text-xs text-muted-foreground mt-1 leading-relaxed">{step.description}</p>
                                </div>
                            </div>
                            {i < 2 && (
                                <ArrowRight className="hidden md:block absolute -right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground/30 z-10" />
                            )}
                        </div>
                    ))}
                </div>
            </motion.div>

            {/* Sprint 14 Teaser */}
            <motion.div
                initial={{ opacity: 0, y: 5 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.2 }}
                className="rounded-2xl border border-dashed border-white/10 bg-white/[0.01] p-6"
            >
                <div className="flex items-start gap-3">
                    <div className="p-2 rounded-lg bg-amber-500/10 border border-amber-500/20">
                        <Info className="w-4 h-4 text-amber-400" />
                    </div>
                    <div>
                        <h3 className="text-sm font-semibold text-amber-400">Coming in Sprint 14</h3>
                        <p className="text-xs text-muted-foreground mt-1 leading-relaxed">
                            Full routing rule builder — define custom rules like "Use GPT-4 for code, Claude for writing".
                            Priority-based model fallback chains, cost caps per model, and request labeling for audit.
                        </p>
                    </div>
                </div>
            </motion.div>
        </motion.div>
    );
}
