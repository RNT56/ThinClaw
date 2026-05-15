import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    RotateCcw,
    CheckCircle2,
    XCircle,
    Zap,
    ShieldAlert,
    Box
} from 'lucide-react';
import * as thinclaw from '../../lib/thinclaw';
import { commands, GGUFMetadata, Result } from '../../lib/bindings';
import { directCommands } from '../../lib/generated/direct-commands';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import { useModelContext } from '../model-context';
import * as Switch from '@radix-ui/react-switch';
import { CustomSelect } from './CustomSelect';
import { analyzeMemoryConstraints, GB } from './memory-analysis';

export function ServerSettings() {
    const {
        currentModelPath: modelPath,
        maxContext,
        setMaxContext,
        localModels,
        systemSpecs,
        currentModelTemplate,
        engineInfo,
        runtimeSnapshot,
        refreshRuntimeSnapshot,
    } = useModelContext();
    const [loading, setLoading] = useState(false);
    const [metadata, setMetadata] = useState<GGUFMetadata | undefined>();
    const [config, setConfig] = useState<any>(null);

    const runtimeKind = runtimeSnapshot?.kind;
    const isLlamaCpp = runtimeKind ? runtimeKind === 'llama_cpp' : (!engineInfo || engineInfo.id === 'llamacpp');
    const isCloudOnly = runtimeKind ? runtimeKind === 'none' : engineInfo?.id === 'none';

    useEffect(() => {
        commands.getUserConfig().then(setConfig);
    }, []);

    useEffect(() => {
        if (modelPath && modelPath !== "auto") {
            commands.getModelMetadata(modelPath).then((res: Result<GGUFMetadata, string>) => {
                if (res.status === "ok") setMetadata(res.data);
                else setMetadata(undefined);
            }).catch(() => setMetadata(undefined));
        } else {
            setMetadata(undefined);
        }
    }, [modelPath]);

    const checkStatus = async () => {
        try {
            await refreshRuntimeSnapshot();
        } catch (e) {
            console.error("Failed to get status", e);
        }
    };

    useEffect(() => {
        checkStatus();
        const interval = setInterval(checkStatus, 2000);
        return () => clearInterval(interval);
    }, [isLlamaCpp, isCloudOnly]);

    // Unified "is the inference server running?" across all engines
    const isServerRunning = runtimeSnapshot?.readiness === 'ready' && !!runtimeSnapshot.endpoint;

    const engineDisplayName = engineInfo?.display_name ?? 'Local AI';

    const manualRestart = async () => {
        setLoading(true);
        const toastId = toast.loading(`Restarting ${engineDisplayName}...`);
        try {
            if (isLlamaCpp) {
                // llama.cpp — use the existing sidecar restart command
                await directCommands.directRuntimeStartChatServer(modelPath, maxContext, currentModelTemplate, null, false, config?.mlock ?? false, config?.quantize_kv ?? false);
            } else if (!isCloudOnly) {
                // MLX / vLLM / Ollama — stop then start via EngineManager
                try { await directCommands.directRuntimeStopEngine(); } catch { /* may already be stopped */ }
                await new Promise(r => setTimeout(r, 500));
                const startRes = await directCommands.directRuntimeStartEngine(modelPath, maxContext);
                if (startRes.status === 'error') throw new Error(startRes.error);
            }
            const snapshot = await refreshRuntimeSnapshot();

            // Attempt dynamic config update for ThinClaw
            try {
                const gatewayStatus = await commands.thinclawGetStatus();
                if (gatewayStatus.status === "ok" && gatewayStatus.data.engine_running) {
                    toast.loading("Syncing Agent Configuration...", { id: toastId });

                    const endpoint = snapshot?.endpoint;
                    const localBaseUrl = endpoint?.baseUrl?.replace(/\/v1\/?$/, "") ?? "http://127.0.0.1:53755";
                    const usedContext = endpoint?.contextSize ?? maxContext;


                    const configPatch = {
                        models: {
                            providers: {
                                local: {
                                    baseUrl: localBaseUrl,
                                    api: "openai-completions",
                                    models: [
                                        {
                                            id: "model",
                                            name: "Local Model",
                                            contextWindow: usedContext,
                                            maxTokens: Math.max(4096, Math.min(8192, Math.floor(usedContext / 4)))
                                        }
                                    ]
                                }
                            }
                        }
                    };

                    await thinclaw.patchThinClawConfig({
                        raw: JSON.stringify(configPatch)
                    });

                    toast.success("Server restarted & Agent Synced", { id: toastId });
                    return;
                }
            } catch (err) {
                console.warn("Dynamic config update failed, falling back to restart:", err);

                // Fallback: Restart ThinClaw Gateway if running
                try {
                    const gatewayStatus = await commands.thinclawGetStatus();
                    if (gatewayStatus.status === "ok" && gatewayStatus.data.engine_running) {
                        toast.loading("Restarting Agent Engine...", { id: toastId });
                        await commands.thinclawStopGateway();
                        await new Promise(r => setTimeout(r, 1000));
                        await commands.thinclawStartGateway();
                    }
                } catch (ignore) {
                    console.warn("Failed to restart gateway:", ignore);
                }
            }

            toast.success("Server restarted", { id: toastId });
        } catch (e) {
            toast.error("Restart failed", { id: toastId, description: String(e) });
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="space-y-6">
            <div className="flex items-center justify-between p-6 border border-border/50 rounded-xl bg-card shadow-sm">
                <div className="space-y-1">
                    <div className="flex items-center font-semibold text-lg">
                        {engineDisplayName} Inference
                        {isServerRunning ? (
                            <span className="ml-3 flex items-center text-emerald-600 dark:text-emerald-400 text-xs bg-emerald-500/10 px-3 py-1 rounded-full border border-emerald-500/20">
                                <CheckCircle2 className="w-3.5 h-3.5 mr-1.5" /> Running
                            </span>
                        ) : (
                            <span className="ml-3 flex items-center text-rose-600 dark:text-rose-400 text-xs bg-rose-500/10 px-3 py-1 rounded-full border border-rose-500/20">
                                <XCircle className="w-3.5 h-3.5 mr-1.5" /> Stopped
                            </span>
                        )}
                    </div>
                    <p className="text-sm text-muted-foreground">
                        {isServerRunning
                            ? `The ${engineDisplayName} server is active and ready.`
                            : isCloudOnly
                                ? "No local inference engine — using cloud providers only."
                                : "The server is currently stopped or initializing."}
                    </p>
                </div>
                {!isCloudOnly && (
                    <button
                        onClick={manualRestart}
                        disabled={loading}
                        className="inline-flex items-center justify-center rounded-xl text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 h-10 px-6 border border-input bg-background hover:bg-accent hover:text-accent-foreground shadow-sm"
                    >
                        <RotateCcw className={cn("w-4 h-4 mr-2", loading && "animate-spin")} />
                        Restart Server
                    </button>
                )}
            </div>

            <div className="p-6 border border-border/50 rounded-xl bg-card space-y-6 shadow-sm">
                <div className="flex items-center justify-between">
                    <div className="space-y-1">
                        <label className="text-base font-semibold">
                            Max Context Window
                        </label>
                        <p className="text-sm text-muted-foreground mr-4">
                            Sets the maximum number of tokens the model can process.
                            Higher values require more RAM/VRAM.
                        </p>
                    </div>
                    <CustomSelect
                        value={maxContext}
                        onChange={setMaxContext}
                        disabled={loading}
                        options={[2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144, 524288, 1048576].map(size => {
                            const label = size >= 1048576 ? '1.0M' : (size / 1024) + 'k';
                            if (!systemSpecs) return { value: size, label };

                            const currentModelFile = localModels.find(m => m.path === modelPath);
                            const modelSize = currentModelFile ? currentModelFile.size : 5 * GB;
                            const reservation = config?.memory_reservation_gb ?? 4;
                            const enableRes = config?.enable_memory_reservation ?? true;
                            const analysis = analyzeMemoryConstraints(
                                size,
                                systemSpecs.total_memory,
                                modelSize,
                                reservation,
                                enableRes,
                                systemSpecs.used_memory,
                                systemSpecs.app_memory,
                                config?.quantize_kv ?? false,
                                systemSpecs.memory_bandwidth_gbps,
                                metadata
                            );

                            return {
                                value: size,
                                label: config?.quantize_kv ? `${label} (Optimized)` : label,
                                disabled: !analysis.canRun,
                                risk: analysis.risk
                            };
                        })}
                    />
                </div>

                {systemSpecs && (() => {
                    const currentModelFile = localModels.find(m => m.path === modelPath);
                    const modelSize = currentModelFile ? currentModelFile.size : 5 * GB;
                    const reservation = config?.memory_reservation_gb ?? 4;
                    const enableRes = config?.enable_memory_reservation ?? true;
                    const analysis = analyzeMemoryConstraints(
                        maxContext,
                        systemSpecs.total_memory,
                        modelSize,
                        reservation,
                        enableRes,
                        systemSpecs.used_memory,
                        systemSpecs.app_memory,
                        config?.quantize_kv ?? false,
                        systemSpecs.memory_bandwidth_gbps,
                        metadata
                    );

                    return (
                        <div className="bg-muted/30 p-4 rounded-xl text-sm space-y-3 border border-border/50">
                            <div className="flex justify-between items-center font-semibold">
                                <span>Estimated RAM Impact</span>
                                <span className={cn(
                                    "px-3 py-1 rounded-full text-xs border bg-background",
                                    analysis.risk === "Critical" ? "text-rose-600 dark:text-rose-400 border-rose-500/20" :
                                        analysis.risk === "Moderate" ? "text-amber-600 dark:text-amber-400 border-amber-500/20" :
                                            "text-emerald-600 dark:text-emerald-400 border-emerald-500/20"
                                )}>
                                    Risk: {analysis.risk}
                                </span>
                            </div>
                            <p className="text-muted-foreground leading-relaxed">
                                {analysis.details}
                            </p>
                            {!analysis.canRun && (
                                <p className="text-rose-600 dark:text-rose-400 font-bold flex items-center gap-2">
                                    <ShieldAlert className="w-4 h-4" /> This setting exceeds your system's safety limits.
                                </p>
                            )}

                            <div className="flex items-center gap-2 pt-2 border-t border-border/10">
                                <Zap className="w-3 h-3 text-amber-500" />
                                <span className="text-[11px] font-medium opacity-80">
                                    Predicted Performance:
                                    <span className="text-primary ml-1 font-bold">
                                        {analysis.predictedTokensPerSec.toFixed(1)} tokens/sec
                                    </span>
                                    <span className="ml-2 opacity-50 font-normal">
                                        (via {systemSpecs.memory_bandwidth_gbps}GB/s hardware bus)
                                    </span>
                                </span>
                            </div>
                        </div>
                    );
                })()}
            </div>

            <div className="p-6 border border-border/50 rounded-xl bg-card space-y-6 shadow-sm">
                <div className="flex items-center justify-between">
                    <div className="space-y-1">
                        <label className="text-base font-semibold">
                            Dedicated AI Memory Quota
                        </label>
                        <p className="text-sm text-muted-foreground mr-4">
                            Commit a specific portion of your hardware to the AI models.
                        </p>
                    </div>
                    <Switch.Root
                        checked={config?.enable_memory_reservation ?? true}
                        onCheckedChange={async (val) => {
                            if (!config) return;
                            const newConfig = { ...config, enable_memory_reservation: val };
                            setConfig(newConfig);
                            await commands.updateUserConfig(newConfig);
                        }}
                        className="w-[42px] h-[25px] bg-muted rounded-full relative shadow-[inner_0_2px_4px_rgba(0,0,0,0.2)] data-[state=checked]:bg-primary transition-colors cursor-pointer outline-none"
                    >
                        <Switch.Thumb className="block w-[21px] h-[21px] bg-white rounded-full shadow-[0_2px_2px_rgba(0,0,0,0.2)] transition-transform duration-100 translate-x-0.5 will-change-transform data-[state=checked]:translate-x-[19px]" />
                    </Switch.Root>
                </div>

                <AnimatePresence>
                    {(config?.enable_memory_reservation ?? true) && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="space-y-6 pt-2 border-t border-border/10"
                        >
                            <div className="flex items-center justify-between">
                                <div className="space-y-1">
                                    <label className="text-sm font-medium opacity-80">
                                        Allocation Limit (Quota)
                                    </label>
                                    <p className="text-xs text-muted-foreground">
                                        Current: <span className="text-primary font-bold">{(systemSpecs?.total_memory ? systemSpecs.total_memory / GB : 0).toFixed(0)}GB Total</span>
                                    </p>
                                </div>
                                <div className="flex items-center gap-4">
                                    <span className="text-lg font-bold w-12 text-right">{config?.memory_reservation_gb ?? 4}GB</span>
                                    <input
                                        type="range"
                                        min="1"
                                        max={(() => {
                                            if (!systemSpecs) return 16;
                                            const appUsedGb = systemSpecs.app_memory / GB;
                                            const freeGb = (systemSpecs.total_memory - systemSpecs.used_memory) / GB;
                                            // Max is app usage + 90% of currently free
                                            return Math.max(1, Math.floor(appUsedGb + (freeGb * 0.9)));
                                        })()}
                                        step="1"
                                        value={config?.memory_reservation_gb ?? 4}
                                        onChange={async (e) => {
                                            if (!config) return;
                                            const val = parseInt(e.target.value);
                                            const newConfig = { ...config, memory_reservation_gb: val };
                                            setConfig(newConfig);
                                            await commands.updateUserConfig(newConfig);
                                        }}
                                        className="w-[200px] h-2 bg-muted rounded-lg appearance-none cursor-pointer accent-primary"
                                    />
                                </div>
                            </div>
                            <p className="text-[11px] opacity-70 italic text-primary bg-primary/5 p-3 rounded-lg border border-primary/10 leading-relaxed">
                                <b>Note on "Reservation":</b> This creates a <b>Virtual Safety Limit</b> for models. The app prevents models from starting if they would encroach on this buffer.
                            </p>

                            <div className="flex items-center justify-between pt-2">
                                <div className="space-y-0.5">
                                    <label className="text-sm font-medium opacity-90">
                                        Hard Memory Locking (mlock)
                                    </label>
                                    <p className="text-[10px] text-muted-foreground max-w-xs">
                                        Forces the OS to keep the AI model pinned in physical RAM.
                                        Prevents swapping/stuttering but may impact system responsiveness.
                                    </p>
                                </div>
                                <Switch.Root
                                    checked={config?.mlock ?? false}
                                    onCheckedChange={async (val) => {
                                        if (!config) return;
                                        const newConfig = { ...config, mlock: val };
                                        setConfig(newConfig);
                                        await commands.updateUserConfig(newConfig);
                                        toast.info("Memory locking strategy updated. Restart server to apply.", { icon: <RotateCcw className="w-4 h-4" /> });
                                    }}
                                    className="w-[36px] h-[20px] bg-muted rounded-full relative shadow-[inner_0_1px_2px_rgba(0,0,0,0.2)] data-[state=checked]:bg-emerald-500 transition-colors cursor-pointer outline-none"
                                >
                                    <Switch.Thumb className="block w-[16px] h-[16px] bg-white rounded-full shadow-[0_1px_2px_rgba(0,0,0,0.2)] transition-transform duration-100 translate-x-0.5 will-change-transform data-[state=checked]:translate-x-[19px]" />
                                </Switch.Root>
                            </div>

                            <div className="flex items-center justify-between pt-4 border-t border-border/10">
                                <div className="space-y-0.5">
                                    <div className="flex items-center gap-2">
                                        <label className="text-sm font-medium opacity-90">
                                            High Capacity Context (4-bit KV)
                                        </label>
                                        <span className="text-[10px] bg-primary/10 text-primary px-1.5 py-0.5 rounded font-bold uppercase tracking-tighter">Reduced RAM</span>
                                    </div>
                                    <p className="text-[10px] text-muted-foreground max-w-xs">
                                        Compresses the model's memory (KV cache). Reduces RAM usage by ~70%,
                                        enabling 4x larger context windows with minimal intelligence loss.
                                    </p>
                                </div>
                                <Switch.Root
                                    checked={config?.quantize_kv ?? false}
                                    onCheckedChange={async (val) => {
                                        if (!config) return;
                                        const newConfig = { ...config, quantize_kv: val };
                                        setConfig(newConfig);
                                        await commands.updateUserConfig(newConfig);
                                        toast.info("Context optimization updated. Restart server to apply.", { icon: <Box className="w-4 h-4" /> });
                                    }}
                                    className="w-[36px] h-[20px] bg-muted rounded-full relative shadow-[inner_0_1px_2px_rgba(0,0,0,0.2)] data-[state=checked]:bg-blue-500 transition-colors cursor-pointer outline-none"
                                >
                                    <Switch.Thumb className="block w-[16px] h-[16px] bg-white rounded-full shadow-[0_1px_2px_rgba(0,0,0,0.2)] transition-transform duration-100 translate-x-0.5 will-change-transform data-[state=checked]:translate-x-[19px]" />
                                </Switch.Root>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>

                {systemSpecs && (() => {
                    const appUsagePercent = (systemSpecs.app_memory / systemSpecs.total_memory) * 100;
                    const systemUsagePercent = ((systemSpecs.used_memory - systemSpecs.app_memory) / systemSpecs.total_memory) * 100;
                    const quotaGb = config?.enable_memory_reservation ? (config.memory_reservation_gb ?? 4) : 0;
                    const quotaPercent = (quotaGb * GB / systemSpecs.total_memory) * 100;

                    // Physical Reality: How much can AI actually use?
                    const totalUsedPercent = (systemSpecs.used_memory / systemSpecs.total_memory) * 100;
                    const physicalFreePercent = 100 - totalUsedPercent;

                    // Widths for the bar:
                    // 1. AI ACTIVE
                    const aiActiveWidth = appUsagePercent;

                    // 2. AI QUOTA (AVAILABLE) - Part of quota that is physically free
                    const aiQuotaAvailableWidth = Math.min(Math.max(0, quotaPercent - appUsagePercent), physicalFreePercent);

                    // 3. AI QUOTA (CONTENDED) - Part of quota the OS is sitting on
                    const aiQuotaContendedWidth = Math.max(0, quotaPercent - appUsagePercent - aiQuotaAvailableWidth);

                    // 4. SYSTEM (OTHER) - System usage outside of AI quota
                    const systemRemainingWidth = Math.max(0, systemUsagePercent - aiQuotaContendedWidth);

                    // 5. UNRESERVED FREE - Total breathing room

                    return (
                        <div className="space-y-4">
                            <div className="flex justify-between text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                                <span>AI Resource Allocation</span>
                                <span className={`${quotaGb > 0 ? "text-primary" : "text-muted-foreground"}`}>
                                    {quotaGb > 0
                                        ? `${((systemSpecs.app_memory / (quotaGb * GB)) * 100).toFixed(0)}% Quota Used`
                                        : `${((systemSpecs.app_memory / systemSpecs.total_memory) * 100).toFixed(1)}% Total RAM Load`
                                    }
                                </span>
                            </div>

                            <div className="w-full h-5 bg-muted/20 rounded-lg overflow-hidden flex border border-border/10 p-0.5">
                                {/* AI ACTIVE */}
                                <div
                                    className="h-full bg-primary rounded-sm transition-all duration-700 ease-out relative group"
                                    style={{ width: `${aiActiveWidth}%` }}
                                >
                                    <div className="absolute hidden group-hover:block bottom-full mb-2 left-1/2 -translate-x-1/2 bg-popover text-popover-foreground text-[10px] px-2 py-1 rounded shadow-xl whitespace-nowrap z-20">
                                        Active AI (App + Sidecars): {(systemSpecs.app_memory / GB).toFixed(1)}GB
                                    </div>
                                </div>

                                {/* AI QUOTA AVAILABLE (FREE) */}
                                <div
                                    className="h-full bg-primary/20 rounded-sm mx-0.5 transition-all duration-700 ease-out relative group border border-primary/20"
                                    style={{ width: `${aiQuotaAvailableWidth}%`, backgroundImage: 'repeating-linear-gradient(45deg, transparent, transparent 2px, rgba(var(--primary), 0.05) 2px, rgba(var(--primary), 0.05) 4px)' }}
                                >
                                    <div className="absolute hidden group-hover:block bottom-full mb-2 left-1/2 -translate-x-1/2 bg-popover text-popover-foreground text-[10px] px-2 py-1 rounded shadow-xl whitespace-nowrap z-20 border border-primary/20">
                                        Free Quota Space: {(aiQuotaAvailableWidth * systemSpecs.total_memory / (100 * GB)).toFixed(1)}GB
                                    </div>
                                </div>

                                {/* AI QUOTA CONTENDED (OS TAKEN) */}
                                {aiQuotaContendedWidth > 0 && (
                                    <div
                                        className="h-full bg-rose-500/20 rounded-sm mx-0.5 transition-all duration-700 ease-out relative group border border-rose-500/30"
                                        style={{ width: `${aiQuotaContendedWidth}%`, backgroundImage: 'repeating-linear-gradient(45deg, transparent, transparent 2px, rgba(239, 68, 68, 0.1) 2px, rgba(239, 68, 68, 0.1) 4px)' }}
                                    >
                                        <div className="absolute hidden group-hover:block bottom-full mb-2 left-1/2 -translate-x-1/2 bg-popover text-popover-foreground text-[10px] px-2 py-1 rounded shadow-xl whitespace-nowrap z-20 border border-rose-500/30">
                                            Quota Taken by OS: {(aiQuotaContendedWidth * systemSpecs.total_memory / (100 * GB)).toFixed(1)}GB
                                        </div>
                                    </div>
                                )}

                                {/* SYSTEM & OTHERS */}
                                <div
                                    className="h-full bg-muted-foreground/20 rounded-sm transition-all duration-700 ease-out relative group"
                                    style={{ width: `${systemRemainingWidth}%` }}
                                >
                                    <div className="absolute hidden group-hover:block bottom-full mb-2 left-1/2 -translate-x-1/2 bg-popover text-popover-foreground text-[10px] px-2 py-1 rounded shadow-xl whitespace-nowrap z-20">
                                        System (Outside Quota): {(systemRemainingWidth * systemSpecs.total_memory / (100 * GB)).toFixed(1)}GB
                                    </div>
                                </div>
                            </div>

                            <div className="grid grid-cols-3 gap-2 text-[10px] text-muted-foreground/80">
                                <div className="flex items-center gap-1.5">
                                    <div className="w-1.5 h-1.5 rounded-full bg-primary" />
                                    <span>AI Active: {(systemSpecs.app_memory / GB).toFixed(1)}GB</span>
                                </div>
                                <div className="flex items-center gap-1.5">
                                    <div className="w-1.5 h-1.5 rounded-full bg-primary/20 border border-primary/30" />
                                    <span>AI Quota: {quotaGb}GB</span>
                                </div>
                                <div className="flex items-center gap-1.5">
                                    <div className="w-1.5 h-1.5 rounded-full bg-muted-foreground/30" />
                                    <span>System: {((systemSpecs.used_memory - systemSpecs.app_memory) / GB).toFixed(1)}GB</span>
                                </div>
                            </div>
                        </div>
                    );
                })()}
            </div>
        </div >
    );
}
