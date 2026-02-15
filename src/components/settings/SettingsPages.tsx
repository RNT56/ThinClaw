import { motion, AnimatePresence } from 'framer-motion';
import * as openclaw from "../../lib/openclaw";
import { ModelBrowser } from './ModelBrowser';
import { PersonaTab } from './PersonaTab';
import { PersonalizationTab } from './PersonalizationTab';
import { SlackTab } from './SlackTab';
import { TelegramTab } from './TelegramTab';
import { GatewayTab } from './GatewayTab';
import { SecretsTab } from './SecretsTab';
import { ChatProviderTab } from './ChatProviderTab';
import { SettingsPage } from './SettingsSidebar';
import {
    Cpu,
    Server,
    RotateCcw,
    CheckCircle2,
    XCircle,
    FolderOpen,
    ImageIcon,
    Settings,
    KeyRound,
    Radio,
    ChevronDown,
    Zap,
    AlertTriangle,
    ShieldAlert,
    Box,
    Command,
    Sparkles,
    FlaskConical
} from 'lucide-react';
import { useModelContext } from '../model-context';
import { useState, useEffect, useCallback } from 'react';
import { commands, SidecarStatus, GGUFMetadata, Result } from '../../lib/bindings';
import { toast } from 'sonner';
import { cn, unwrap } from '../../lib/utils';
import { ThemeToggle, useTheme } from '../theme-provider';
import { DARK_SYNTAX_THEMES, LIGHT_SYNTAX_THEMES, SyntaxTheme } from '../../lib/syntax-themes';
import { APP_THEMES, AppTheme } from '../../lib/app-themes';
import * as Switch from '@radix-ui/react-switch';

interface SettingsContentProps {
    activePage: SettingsPage;
}

// 1 GB = 1024^3 bytes
const GB = 1073741824;

interface MemoryAnalysis {
    canRun: boolean;
    risk: "Safe" | "Moderate" | "Critical";
    totalNeededRef: number; // in GB
    details: string;
    predictedTokensPerSec: number;
}

function analyzeMemoryConstraints(
    ctx: number,
    totalRamBytes: number,
    modelSizeBytes: number,
    reservationGb: number,
    enableReservation: boolean,
    usedMemoryBytes: number,
    appMemoryBytes: number,
    quantizeKv: boolean,
    bandwidthGbps: number,
    metadata?: GGUFMetadata
): MemoryAnalysis {
    let availableForAI = 0;
    let limitLabel = "";

    // Calculate physical headroom: total RAM minus what the OS and other apps are using (excluding our app's AI usage)
    const physicalHeadroomForAI = totalRamBytes - (usedMemoryBytes - appMemoryBytes);

    if (enableReservation) {
        const quotaBytes = reservationGb * GB;
        // The effective limit is the MIN of user quota and actual physical headroom
        availableForAI = Math.min(quotaBytes, physicalHeadroomForAI);

        limitLabel = availableForAI < quotaBytes ? "Physical Limit (System Full)" : `${reservationGb}GB AI Quota`;
    } else {
        availableForAI = Math.max(0, physicalHeadroomForAI);
        limitLabel = "Physical Headroom";
    }

    // GGUF models are already quantized, so weightLoad is basically the file size
    const weightLoad = modelSizeBytes * 1.05;

    let kvLoad = 0;
    let breakdown = "";

    if (metadata && metadata.block_count > 0) {
        // KV size per token = 2 * layers * heads_kv * head_dim * precision
        const n_layers = metadata.block_count;
        const n_heads = metadata.head_count;
        const n_heads_kv = metadata.head_count_kv;
        const n_embd = metadata.embedding_length;
        const head_dim = n_heads > 0 ? n_embd / n_heads : 128;

        // llama.cpp default KV is F16 (2 bytes)
        // If quantizeKv is true, we use Q4_0 (0.5 bytes approx / 4-bit)
        const bytes_per_element = quantizeKv ? 0.5 : 2.0;
        const bytes_per_token = 2 * n_layers * n_heads_kv * head_dim * bytes_per_element;
        kvLoad = ctx * bytes_per_token;

        breakdown = `${metadata.architecture.toUpperCase()} ${n_layers}L. `;
    } else {
        const baseKv = 204800;
        kvLoad = ctx * (quantizeKv ? baseKv * 0.25 : baseKv);
        breakdown = "Estimate: ";
    }

    const totalNeeded = weightLoad + kvLoad;
    const totalNeededGB = totalNeeded / GB;

    let risk: "Safe" | "Moderate" | "Critical" = "Safe";
    if (totalNeeded > availableForAI * 0.9) risk = "Critical";
    else if (totalNeeded > availableForAI * 0.7) risk = "Moderate";

    // Speed (tok/s) = Bandwidth / (Model weights + KV Cache)
    // We add a 20% penalty for system overhead and 4-bit KV boost if active
    const kvBoost = quantizeKv ? 1.05 : 1.0; // Minimal scaling for decoding
    const totalDataToRead = weightLoad + kvLoad;
    const predictedTokensPerSec = (bandwidthGbps / (totalDataToRead / GB)) * 0.85 * kvBoost;

    return {
        canRun: totalNeeded <= availableForAI,
        risk,
        totalNeededRef: totalNeededGB,
        predictedTokensPerSec,
        details: `${breakdown}Weights: ${(weightLoad / GB).toFixed(1)}GB + Cache: ${(kvLoad / GB).toFixed(2)}GB ≈ ${totalNeededGB.toFixed(1)}GB. Limit: ${(availableForAI / GB).toFixed(1)}GB (${limitLabel}).`
    };
}

const OptimizedIcon = () => (
    <div className="relative flex items-center justify-center w-4 h-4">
        <div className="absolute inset-0 bg-emerald-500/20 rounded-full animate-pulse" />
        <Zap className="w-3 h-3 text-emerald-500 fill-emerald-500/20" />
    </div>
);

const RiskIcon = () => (
    <div className="relative flex items-center justify-center w-4 h-4">
        <div className="absolute inset-0 bg-amber-500/20 rounded-full animate-ping opacity-20" />
        <AlertTriangle className="w-3 h-3 text-amber-500" />
    </div>
);

export function SettingsContent({ activePage }: SettingsContentProps) {
    return (
        <div className="flex-1 h-full overflow-hidden flex flex-col bg-background/50 backdrop-blur-sm">
            <AnimatePresence mode="wait">
                <motion.div
                    key={activePage}
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -10 }}
                    transition={{ duration: 0.2 }}
                    className="flex-1 overflow-y-auto p-8 max-w-5xl mx-auto w-full"
                >
                    <PageHeader page={activePage} />

                    <div className="mt-8">
                        {activePage === 'models' && <ModelBrowser />}
                        {activePage === 'persona' && <PersonaTab />}
                        {activePage === 'personalization' && <PersonalizationTab />}
                        {activePage === 'server' && <ServerSettings />}
                        {activePage === 'troubleshooting' && <TroubleshootingSettings />}
                        {activePage === 'appearance' && <AppearanceSettings />}
                        {activePage === 'openclaw-slack' && <SlackTab />}
                        {activePage === 'openclaw-telegram' && <TelegramTab />}
                        {activePage === 'openclaw-gateway' && <GatewayTab />}
                        {activePage === 'secrets' && <SecretsTab />}
                        {activePage === 'inference' && <ChatProviderTab />}
                    </div>
                </motion.div>
            </AnimatePresence>
        </div>
    );
}

function PageHeader({ page }: { page: SettingsPage }) {
    const titles: Record<SettingsPage, { title: string, description: string, icon: any }> = {
        models: {
            title: "Model Management",
            description: "Download and configure your local LLMs, Vision, and Image models.",
            icon: Cpu
        },
        inference: {
            title: "Chat Provider",
            description: "Select the primary intelligence engine for your workspace.",
            icon: Radio
        },
        persona: {
            title: "My Persona",
            description: "Define how the AI perceives itself and interacts with you.",
            icon: Settings
        },
        personalization: {
            title: "Global Instructions",
            description: "Custom system instructions and memory preferences.",
            icon: Settings
        },
        server: {
            title: "Server & Memory",
            description: "Monitor system performance and adjust inference parameters.",
            icon: Server
        },
        troubleshooting: {
            title: "Troubleshooting",
            description: "Diagnostic tools and access to configuration files.",
            icon: ShieldAlert
        },
        appearance: {
            title: "Appearance",
            description: "Customize the look and feel of your workspace.",
            icon: Settings
        },
        'openclaw-slack': {
            title: "Slack Integration",
            description: "Connect OpenClaw to your Slack workspace.",
            icon: Settings
        },
        'openclaw-telegram': {
            title: "Telegram Integration",
            description: "Connect OpenClaw to Telegram.",
            icon: Settings
        },
        'openclaw-gateway': {
            title: "OpenClaw Gateway",
            description: "Manage autonomy, connectivity and agent runtime.",
            icon: Radio
        },
        'secrets': {
            title: "API Secrets",
            description: "Manage API keys for cloud providers.",
            icon: KeyRound
        }
    };

    const { title, description, icon: Icon } = titles[page];

    return (
        <div className="border-b border-border/50 pb-6">
            <div className="flex items-center gap-3 mb-2">
                <div className="p-2 bg-primary/10 rounded-lg">
                    <Icon className="w-6 h-6 text-primary" />
                </div>
                <h1 className="text-3xl font-bold tracking-tight">{title}</h1>
            </div>
            <p className="text-muted-foreground">{description}</p>
        </div>
    );
}

function CustomSelect({
    value,
    onChange,
    options,
    disabled,
    placeholder = "Select option..."
}: {
    value: number,
    onChange: (val: number) => void,
    options: { value: number, label: string, disabled?: boolean, risk?: "Safe" | "Moderate" | "Critical" }[],
    disabled?: boolean,
    placeholder?: string
}) {
    const [isOpen, setIsOpen] = useState(false);
    const selectedOption = options.find(o => o.value === value);

    // Close on click outside
    useEffect(() => {
        if (!isOpen) return;
        const handleClick = () => setIsOpen(false);
        window.addEventListener('click', handleClick);
        return () => window.removeEventListener('click', handleClick);
    }, [isOpen]);

    return (
        <div className="relative w-[220px]" onClick={e => e.stopPropagation()}>
            <button
                type="button"
                onClick={() => !disabled && setIsOpen(!isOpen)}
                disabled={disabled}
                className={cn(
                    "flex h-11 w-full items-center justify-between rounded-xl border bg-background/50 px-4 py-2 text-sm shadow-sm transition-all duration-200 backdrop-blur-md",
                    isOpen ? "border-primary ring-2 ring-primary/20 shadow-lg" : "border-border/50 hover:border-border",
                    disabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer"
                )}
            >
                <span className="truncate font-medium">
                    {selectedOption ? selectedOption.label : placeholder}
                </span>
                <ChevronDown className={cn("h-4 w-4 text-muted-foreground transition-transform duration-300", isOpen && "rotate-180")} />
            </button>

            <AnimatePresence>
                {isOpen && (
                    <motion.div
                        initial={{ opacity: 0, scale: 0.95, y: -10 }}
                        animate={{ opacity: 1, scale: 1, y: 0 }}
                        exit={{ opacity: 0, scale: 0.95, y: -10 }}
                        transition={{ duration: 0.15, ease: "easeOut" }}
                        className="absolute right-0 top-[calc(100%+8px)] z-50 w-full min-w-[200px] overflow-hidden rounded-xl border border-border/50 bg-card/90 p-1.5 shadow-2xl backdrop-blur-xl"
                    >
                        <div className="max-h-[300px] overflow-y-auto custom-scrollbar">
                            {options.map((option) => (
                                <button
                                    key={option.value}
                                    type="button"
                                    disabled={option.disabled}
                                    onClick={() => {
                                        onChange(option.value);
                                        setIsOpen(false);
                                    }}
                                    className={cn(
                                        "flex w-full items-center justify-between rounded-lg px-3 py-2.5 text-left text-sm transition-all duration-200 mb-0.5 last:mb-0",
                                        option.value === value ? "bg-primary/10 text-primary font-bold" : "hover:bg-muted/50 text-foreground",
                                        option.disabled ? "opacity-40 cursor-not-allowed grayscale-[50%]" : "cursor-pointer"
                                    )}
                                >
                                    <span className="flex items-center gap-2">
                                        {option.label}
                                        {option.disabled && <RiskIcon />}
                                    </span>
                                    {option.risk && !option.disabled && (
                                        <div className="flex items-center gap-2">
                                            {option.value < 32768 && <OptimizedIcon />}
                                            <div className={cn(
                                                "w-1.5 h-1.5 rounded-full",
                                                option.risk === "Critical" ? "bg-rose-500 shadow-[0_0_8px_rgba(244,63,94,0.5)]" :
                                                    option.risk === "Moderate" ? "bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.5)]" :
                                                        "bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.5)]"
                                            )} />
                                        </div>
                                    )}
                                </button>
                            ))}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}

function ServerSettings() {
    const [status, setStatus] = useState<SidecarStatus | null>(null);
    const {
        currentModelPath: modelPath,
        maxContext,
        setMaxContext,
        localModels,
        systemSpecs,
        currentModelTemplate
    } = useModelContext();
    const [loading, setLoading] = useState(false);
    const [metadata, setMetadata] = useState<GGUFMetadata | undefined>();
    const [config, setConfig] = useState<any>(null);

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
            const s = await commands.getSidecarStatus();
            setStatus(s);
        } catch (e) {
            console.error("Failed to get status", e);
        }
    };

    useEffect(() => {
        checkStatus();
        const interval = setInterval(checkStatus, 2000);
        return () => clearInterval(interval);
    }, []);

    const manualRestart = async () => {
        setLoading(true);
        const toastId = toast.loading("Restarting server manually...");
        try {
            await commands.startChatServer(modelPath, maxContext, currentModelTemplate, null, false, config?.mlock ?? false, config?.quantize_kv ?? false);
            await checkStatus();

            // Attempt dynamic config update for OpenClaw
            try {
                const gatewayStatus = await commands.openclawGetStatus();
                if (gatewayStatus.status === "ok" && gatewayStatus.data.gateway_running) {
                    toast.loading("Syncing Agent Configuration...", { id: toastId });

                    // Fetch the actual running config of the chat server
                    const chatConfig = await commands.getChatServerConfig();

                    const localPort = chatConfig ? chatConfig.port : 53755;
                    const usedContext = chatConfig ? chatConfig.context_size : maxContext;


                    const configPatch = {
                        models: {
                            providers: {
                                local: {
                                    baseUrl: `http://127.0.0.1:${localPort}`,
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

                    await openclaw.patchOpenClawConfig({
                        raw: JSON.stringify(configPatch)
                    });

                    toast.success("Server restarted & Agent Synced", { id: toastId });
                    return;
                }
            } catch (err) {
                console.warn("Dynamic config update failed, falling back to restart:", err);

                // Fallback: Restart OpenClaw Gateway if running
                try {
                    const gatewayStatus = await commands.openclawGetStatus();
                    if (gatewayStatus.status === "ok" && gatewayStatus.data.gateway_running) {
                        toast.loading("Restarting Agent Gateway...", { id: toastId });
                        await commands.openclawStopGateway();
                        await new Promise(r => setTimeout(r, 1000));
                        await commands.openclawStartGateway();
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
                        Local AI Inference
                        {status?.chat_running ? (
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
                        {status?.chat_running ? "The Local AI Server is active and ready." : "The server is currently stopped or initializing."}
                    </p>
                </div>
                <button
                    onClick={manualRestart}
                    disabled={loading}
                    className="inline-flex items-center justify-center rounded-xl text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 h-10 px-6 border border-input bg-background hover:bg-accent hover:text-accent-foreground shadow-sm"
                >
                    <RotateCcw className={cn("w-4 h-4 mr-2", loading && "animate-spin")} />
                    Restart Server
                </button>
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

function TroubleshootingSettings() {
    const { currentEmbeddingModelPath, currentModelPath: modelPath } = useModelContext();
    const [status, setStatus] = useState<SidecarStatus | null>(null);
    const [pathValid, setPathValid] = useState<boolean | null>(null);

    const [clawStatus, setClawStatus] = useState<openclaw.OpenClawStatus | null>(null);

    const checkStatus = async () => {
        try {
            const s = await commands.getSidecarStatus();
            setStatus(s);
            const cs = await openclaw.getOpenClawStatus();
            setClawStatus(cs);
        } catch (e) {
            console.error(e);
        }
    };

    const toggleDevMode = async (enabled: boolean) => {
        try {
            await openclaw.setDevModeWizard(enabled);
            const cs = await openclaw.getOpenClawStatus();
            setClawStatus(cs);
            toast.success(enabled ? "Dev mode onboarding enabled" : "Dev mode onboarding disabled");
        } catch (e) {
            toast.error("Failed to update dev mode");
        }
    };

    const validatePath = useCallback(async (path: string) => {
        if (path === "auto") { setPathValid(true); return; }
        if (!path.trim()) { setPathValid(false); return; }
        try {
            if (path.length > 3) {
                const isValid = await commands.checkModelPath(path);
                setPathValid(isValid);
            } else {
                setPathValid(false);
            }
        } catch (e) { setPathValid(false); }
    }, []);

    useEffect(() => {
        checkStatus();
        validatePath(modelPath);
    }, [modelPath, validatePath]);

    const openModelsFolder = async () => unwrap(await commands.openModelsFolder());



    return (
        <div className="space-y-6">
            <div className="grid gap-4 md:grid-cols-2">
                <div className="p-6 border border-border/50 rounded-xl bg-card space-y-4 font-mono text-sm shadow-sm">
                    <h4 className="font-semibold font-sans mb-4">System Details</h4>
                    <div className="flex justify-between border-b border-border/50 pb-2">
                        <span className="text-muted-foreground">Embedding Server:</span>
                        <span className={status?.embedding_running ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground"}>
                            {status?.embedding_running ? "Running" : "Stopped"}
                        </span>
                    </div>
                    <div className="flex flex-col gap-1">
                        <span className="text-muted-foreground">Embedding Model:</span>
                        <span className="truncate bg-muted/50 p-2 rounded text-xs" title={currentEmbeddingModelPath}>
                            {currentEmbeddingModelPath || "None"}
                        </span>
                    </div>
                </div>

                <div className="p-6 border border-border/50 rounded-xl bg-card space-y-4 shadow-sm">
                    <h4 className="font-semibold mb-4">Diagnostic Links</h4>
                    <div className="flex flex-col gap-2">
                        <button
                            onClick={openModelsFolder}
                            className="w-full bg-background border border-border/50 hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <FolderOpen className="w-4 h-4 mr-2 text-primary" /> Open Models Folder
                        </button>
                        <button
                            onClick={async () => unwrap(await commands.openImagesFolder())}
                            className="w-full bg-background border border-border/50 hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <ImageIcon className="w-4 h-4 mr-2 text-pink-500" /> Open Generated Images
                        </button>
                        <button
                            onClick={async () => unwrap(await commands.openConfigFile())}
                            className="w-full bg-background border border-border/50 hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <Settings className="w-4 h-4 mr-2 text-muted-foreground" /> Open User Config
                        </button>
                    </div>
                </div>
            </div>

            <div className="p-6 border border-border/50 rounded-xl bg-card space-y-4 shadow-sm">
                <h4 className="font-semibold">Model Path Validation</h4>
                <div className="space-y-2">
                    <label className="text-sm text-muted-foreground">Current Model Absolute Path</label>
                    <div className="relative">
                        <input
                            value={modelPath}
                            readOnly
                            className="flex h-12 w-full rounded-xl border bg-muted/30 px-4 py-2 text-xs font-mono text-muted-foreground"
                        />
                        <div className="absolute right-4 top-3.5">
                            {pathValid === true && <CheckCircle2 className="h-5 w-5 text-emerald-600 dark:text-emerald-400" />}
                            {pathValid === false && <XCircle className="h-5 w-5 text-rose-600 dark:text-rose-400" />}
                        </div>
                    </div>
                </div>
            </div>

            <div className="p-6 border border-rose-500/20 rounded-xl bg-card/50 space-y-4 shadow-sm">
                <div className="flex items-center gap-2 mb-2">
                    <FlaskConical className="w-5 h-5 text-rose-500" />
                    <h4 className="font-semibold text-rose-500 dark:text-rose-400">Developer Settings</h4>
                </div>

                <div className="flex items-center justify-between p-4 bg-muted/30 rounded-xl border border-border/50">
                    <div className="space-y-1">
                        <span className="text-sm font-medium">Always show Onboarding Wizard</span>
                        <p className="text-xs text-muted-foreground">Force the onboarding flow to run every time Scrappy starts.</p>
                    </div>
                    <button
                        onClick={() => toggleDevMode(!clawStatus?.dev_mode_wizard)}
                        className={cn(
                            "relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2",
                            clawStatus?.dev_mode_wizard ? "bg-primary" : "bg-muted"
                        )}
                    >
                        <span
                            className={cn(
                                "inline-block h-4 w-4 transform rounded-full bg-white transition-transform",
                                clawStatus?.dev_mode_wizard ? "translate-x-6" : "translate-x-1"
                            )}
                        />
                    </button>
                </div>
            </div>
        </div>
    );
}

function SyntaxThemeOption({ theme, isActive, onClick }: { theme: SyntaxTheme, isActive: boolean, onClick: () => void }) {
    return (
        <button
            onClick={onClick}
            className={cn(
                "group relative flex flex-col items-start p-4 rounded-xl border transition-all duration-200 text-left w-full",
                isActive
                    ? "bg-primary/5 border-primary shadow-[0_0_20px_rgba(var(--primary),0.1)] ring-1 ring-primary/20"
                    : "bg-card/50 hover:bg-muted/50 border-border/50 hover:border-border shadow-sm"
            )}
        >
            <div className="flex items-center justify-between w-full mb-3">
                <span className={cn(
                    "text-[10px] font-bold transition-colors uppercase tracking-[0.15em]",
                    isActive ? "text-primary" : "text-muted-foreground group-hover:text-foreground"
                )}>
                    {theme.label}
                </span>
                {isActive && (
                    <div className="w-1.5 h-1.5 rounded-full bg-primary animate-pulse shadow-[0_0_8px_rgba(var(--primary),0.5)]" />
                )}
            </div>

            <div className="flex gap-2 p-1.5 rounded-lg bg-black/5 dark:bg-white/5 w-full border border-border/10 justify-center">
                <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${theme.colors.keyword})` }} title="Keyword" />
                <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${theme.colors.string})` }} title="String" />
                <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${theme.colors.function})` }} title="Function" />
                <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${theme.colors.number})` }} title="Number" />
            </div>
        </button>
    );
}

function AppThemeOption({ theme, isActive, onClick, currentMode }: { theme: AppTheme, isActive: boolean, onClick: () => void, currentMode: 'light' | 'dark' }) {
    const colors = currentMode === 'dark' ? theme.dark : theme.light;

    return (
        <button
            onClick={onClick}
            className={cn(
                "group relative flex flex-col items-start p-4 rounded-xl border transition-all duration-200 text-left w-full",
                isActive
                    ? "bg-primary/5 border-primary shadow-[0_0_20px_rgba(var(--primary),0.1)] ring-1 ring-primary/20"
                    : "bg-card/50 hover:bg-muted/50 border-border/50 hover:border-border shadow-sm"
            )}
        >
            <div className="flex items-center justify-between w-full mb-3">
                <span className={cn(
                    "text-[10px] font-bold transition-colors uppercase tracking-[0.15em]",
                    isActive ? "text-primary" : "text-muted-foreground group-hover:text-foreground"
                )}>
                    {theme.label}
                </span>
                {isActive && (
                    <div className="w-1.5 h-1.5 rounded-full bg-primary animate-pulse shadow-[0_0_8px_rgba(var(--primary),0.5)]" />
                )}
            </div>

            <div className="flex gap-2 p-1.5 rounded-lg w-full border border-border/10 justify-center" style={{ backgroundColor: `hsl(${colors.background})` }}>
                <div className="w-4 h-4 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.primary})` }} title="Primary" />
                <div className="w-4 h-4 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.accent})` }} title="Accent" />
                <div className="w-4 h-4 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.secondary})` }} title="Secondary" />
            </div>
        </button>
    );
}

function AppearanceSettings() {
    const {
        theme,
        darkSyntaxTheme,
        lightSyntaxTheme,
        setSyntaxTheme,
        appThemeId,
        setAppThemeId
    } = useTheme();

    const [config, setConfig] = useState<any>(null);

    useEffect(() => {
        commands.getUserConfig().then(setConfig);
    }, []);

    const updateShortcut = async (val: string) => {
        if (!config) return;
        const newConfig = { ...config, spotlight_shortcut: val };
        setConfig(newConfig);
        await commands.updateUserConfig(newConfig);
    };

    const effectiveMode = theme === 'system'
        ? (window.matchMedia("(prefers-color-scheme: dark)").matches ? 'dark' : 'light')
        : theme as 'light' | 'dark';

    return (
        <div className="space-y-10">
            {/* UI Theme Group */}
            <div className="p-8 border rounded-2xl bg-gradient-to-br from-card to-background shadow-xl border-border/30 flex items-center justify-between">
                <div className="space-y-1">
                    <h4 className="font-bold text-xl tracking-tight">Workspace Aesthetic</h4>
                    <p className="text-sm text-muted-foreground max-w-sm leading-relaxed">
                        Customize your environment interface. Mode switches instantly update your syntax palette.
                    </p>
                </div>
                <div className="scale-110">
                    <ThemeToggle />
                </div>
            </div>

            {/* App Style Templates */}
            <div className="space-y-6">
                <div className="pt-6 border-t border-border/10 space-y-1">
                    <h3 className="text-xl font-bold tracking-tight">App Style Templates</h3>
                    <p className="text-sm text-muted-foreground">Adjust the entire application styling with these curated templates.</p>
                </div>

                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
                    {APP_THEMES.map(t => (
                        <AppThemeOption
                            key={t.id}
                            theme={t}
                            isActive={appThemeId === t.id}
                            onClick={() => setAppThemeId(t.id)}
                            currentMode={effectiveMode}
                        />
                    ))}
                </div>
            </div>

            <div className="pt-6 border-t border-border/10 space-y-1">
                <h3 className="text-xl font-bold tracking-tight">Syntax Highlight Palettes</h3>
                <p className="text-sm text-muted-foreground">Choose how code blocks and transcripts are rendered in your workspace.</p>
            </div>

            {/* Dark Mode Group */}
            <div className="space-y-4">
                <div className="flex items-center gap-3 px-1">
                    <div className="w-1 h-6 rounded-full bg-indigo-500 shadow-[0_0_10px_rgba(99,102,241,0.5)]" />
                    <h4 className="font-bold text-lg tracking-tight">Dark Mode Palette</h4>
                </div>
                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
                    {DARK_SYNTAX_THEMES.map(t => (
                        <SyntaxThemeOption
                            key={t.id}
                            theme={t}
                            isActive={darkSyntaxTheme === t.id}
                            onClick={() => setSyntaxTheme('dark', t.id)}
                        />
                    ))}
                </div>
            </div>

            {/* Light Mode Group */}
            <div className="space-y-4">
                <div className="flex items-center gap-3 px-1">
                    <div className="w-1 h-6 rounded-full bg-orange-400 shadow-[0_0_10px_rgba(251,146,60,0.5)]" />
                    <h4 className="font-bold text-lg tracking-tight">Light Mode Palette</h4>
                </div>
                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
                    {LIGHT_SYNTAX_THEMES.map(t => (
                        <SyntaxThemeOption
                            key={t.id}
                            theme={t}
                            isActive={lightSyntaxTheme === t.id}
                            onClick={() => setSyntaxTheme('light', t.id)}
                        />
                    ))}
                </div>
            </div>

            {/* Global Hotkeys */}
            <div className="pt-6 border-t border-border/10 space-y-4">
                <div className="space-y-1">
                    <h3 className="text-xl font-bold tracking-tight">Global Hotkeys</h3>
                    <p className="text-sm text-muted-foreground">Configure shortcuts to trigger Scrappy from anywhere on your system.</p>
                </div>

                <div className="p-6 border border-border/50 rounded-xl bg-card/50 flex items-center justify-between shadow-sm border-border/50">
                    <div className="space-y-1">
                        <label className="text-sm font-semibold flex items-center gap-2 text-foreground">
                            <Sparkles className="w-4 h-4 text-primary" />
                            Spotlight Chat Shortcut
                        </label>
                        <p className="text-xs text-muted-foreground">
                            Press this to instantly open the liquid glass chat bar.
                        </p>
                    </div>
                    <div className="relative">
                        <input
                            value={config?.spotlight_shortcut ?? "Command+Shift+K"}
                            onChange={(e) => updateShortcut(e.currentTarget.value)}
                            placeholder="e.g. Command+Shift+K"
                            className="bg-background border border-border/50 rounded-lg px-3 py-2 text-sm w-48 font-mono focus:ring-2 focus:ring-primary outline-none transition-all text-foreground"
                        />
                        <span className="absolute right-3 top-2.5 opacity-30 pointer-events-none">
                            <Command className="w-4 h-4 text-foreground" />
                        </span>
                    </div>
                </div>
                <p className="text-[10px] text-muted-foreground italic px-2 flex items-center gap-2">
                    <AlertTriangle className="w-3 h-3 text-amber-500" /> Note: Shortcut changes require application restart to register properly with the OS.
                </p>
            </div>

            {/* Hint Box */}
            <div className="bg-primary/5 border border-primary/10 rounded-2xl p-6 relative overflow-hidden group">
                <div className="absolute top-0 right-0 w-32 h-32 bg-primary/5 rounded-full blur-3xl -mr-16 -mt-16" />
                <div className="flex gap-4 relative z-10">
                    <div className="p-2.5 bg-primary/10 rounded-xl h-fit border border-primary/20">
                        <Settings className="w-5 h-5 text-primary" />
                    </div>
                    <div className="space-y-1">
                        <span className="font-bold text-[10px] text-primary uppercase tracking-[0.2em] block mb-1">Runtime Adaptive</span>
                        <p className="text-sm text-muted-foreground leading-relaxed">
                            Selections are applied globally and instantly. We persist your preferences to ensure a consistent experience across sessions.
                        </p>
                    </div>
                </div>
            </div>
        </div>
    );
}
