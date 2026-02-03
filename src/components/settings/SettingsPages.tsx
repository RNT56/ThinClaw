import { motion, AnimatePresence } from 'framer-motion';
import * as clawdbot from "../../lib/clawdbot";
import { ModelBrowser } from './ModelBrowser';
import { PersonaTab } from './PersonaTab';
import { PersonalizationTab } from './PersonalizationTab';
import { SlackTab } from './SlackTab';
import { TelegramTab } from './TelegramTab';
import { GatewayTab } from './GatewayTab';
import { SecretsTab } from './SecretsTab';
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
    ShieldAlert,
    KeyRound
} from 'lucide-react';
import { useModelContext } from '../model-context';
import { useState, useEffect, useCallback } from 'react';
import { commands, SidecarStatus, GGUFMetadata, Result } from '../../lib/bindings';
import { toast } from 'sonner';
import { cn, unwrap } from '../../lib/utils';
import { ThemeToggle, useTheme } from '../theme-provider';
import { DARK_SYNTAX_THEMES, LIGHT_SYNTAX_THEMES, SyntaxTheme } from '../../lib/syntax-themes';

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
}

function analyzeMemoryConstraints(
    ctx: number,
    totalRamBytes: number,
    modelSizeBytes: number,
    metadata?: GGUFMetadata
): MemoryAnalysis {
    const reserve = Math.max(4 * GB, totalRamBytes * 0.2);
    const availableForAI = Math.max(0, totalRamBytes - reserve);

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
        const bytes_per_token = 2 * n_layers * n_heads_kv * head_dim * 2;
        kvLoad = ctx * bytes_per_token;

        breakdown = `${metadata.architecture.toUpperCase()} ${n_layers}L. `;
    } else {
        kvLoad = ctx * 204800;
        breakdown = "Estimate: ";
    }

    const totalNeeded = weightLoad + kvLoad;
    const totalNeededGB = totalNeeded / GB;

    let risk: "Safe" | "Moderate" | "Critical" = "Safe";
    if (totalNeeded > availableForAI * 0.9) risk = "Critical";
    else if (totalNeeded > availableForAI * 0.7) risk = "Moderate";

    return {
        canRun: totalNeeded <= availableForAI,
        risk,
        totalNeededRef: totalNeededGB,
        details: `${breakdown}Weights: ${(weightLoad / GB).toFixed(1)}GB + Cache: ${(kvLoad / GB).toFixed(1)}GB ≈ ${totalNeededGB.toFixed(1)}GB. Limit: ${(availableForAI / GB).toFixed(1)}GB.`
    };
}

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
                        {activePage === 'clawdbot-slack' && <SlackTab />}
                        {activePage === 'clawdbot-telegram' && <TelegramTab />}
                        {activePage === 'clawdbot-gateway' && <GatewayTab />}
                        {activePage === 'secrets' && <SecretsTab />}
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
        'clawdbot-slack': {
            title: "Slack Integration",
            description: "Connect Clawdbot to your Slack workspace.",
            icon: Settings
        },
        'clawdbot-telegram': {
            title: "Telegram Integration",
            description: "Connect Clawdbot to Telegram.",
            icon: Settings
        },
        'clawdbot-gateway': {
            title: "Gateway Control",
            description: "Manage Clawdbot runtime services.",
            icon: Server
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
            await commands.startChatServer(modelPath, maxContext, currentModelTemplate, null, false);
            await checkStatus();

            // Attempt dynamic config update for Clawdbot
            try {
                const gatewayStatus = await commands.getClawdbotStatus();
                if (gatewayStatus.status === "ok" && gatewayStatus.data.gateway_running) {
                    toast.loading("Syncing Agent Configuration...", { id: toastId });

                    // Fetch the actual running config of the chat server
                    const chatConfig = await commands.getChatServerConfig();

                    const localPort = chatConfig ? chatConfig.port : 53755;
                    const usedContext = chatConfig ? chatConfig.context_size : maxContext;

                    console.log(`[DynamicPatch] Updating Moltbot with port=${localPort} ctx=${usedContext}`);

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

                    await clawdbot.patchClawdbotConfig({
                        raw: JSON.stringify(configPatch)
                    });

                    toast.success("Server restarted & Agent Synced", { id: toastId });
                    return;
                }
            } catch (err) {
                console.warn("Dynamic config update failed, falling back to restart:", err);

                // Fallback: Restart Clawdbot Gateway if running
                try {
                    const gatewayStatus = await commands.getClawdbotStatus();
                    if (gatewayStatus.status === "ok" && gatewayStatus.data.gateway_running) {
                        toast.loading("Restarting Agent Gateway...", { id: toastId });
                        await commands.stopClawdbotGateway();
                        await new Promise(r => setTimeout(r, 1000));
                        await commands.startClawdbotGateway();
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
            <div className="flex items-center justify-between p-6 border rounded-xl bg-card shadow-sm">
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

            <div className="p-6 border rounded-xl bg-card space-y-6 shadow-sm">
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
                    <select
                        value={maxContext}
                        onChange={(e) => setMaxContext(parseInt(e.target.value))}
                        className="h-10 w-[180px] rounded-xl border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        disabled={loading}
                    >
                        {[2048, 4096, 8192, 16384, 32768, 65536, 131072].map(size => {
                            if (!systemSpecs) return <option key={size} value={size}>{size / 1024}k</option>;
                            const currentModelFile = localModels.find(m => m.path === modelPath);
                            const modelSize = currentModelFile ? currentModelFile.size : 5 * GB;
                            const analysis = analyzeMemoryConstraints(size, systemSpecs.total_memory, modelSize, metadata);

                            return (
                                <option
                                    key={size}
                                    value={size}
                                    disabled={!analysis.canRun}
                                    className={analysis.risk === "Critical" ? "text-rose-600 dark:text-rose-400" : ""}
                                >
                                    {size / 1024}k {analysis.canRun ? (size < 32768 ? "(Min 32k for Agent)" : "") : "(Unsafe)"}
                                </option>
                            );
                        })}
                    </select>
                </div>

                {systemSpecs && (() => {
                    const currentModelFile = localModels.find(m => m.path === modelPath);
                    const modelSize = currentModelFile ? currentModelFile.size : 5 * GB;
                    const analysis = analyzeMemoryConstraints(maxContext, systemSpecs.total_memory, modelSize, metadata);

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
                        </div>
                    );
                })()}
            </div>
        </div>
    );
}

function TroubleshootingSettings() {
    const { currentEmbeddingModelPath, currentModelPath: modelPath } = useModelContext();
    const [status, setStatus] = useState<SidecarStatus | null>(null);
    const [pathValid, setPathValid] = useState<boolean | null>(null);

    const checkStatus = async () => {
        try {
            const s = await commands.getSidecarStatus();
            setStatus(s);
        } catch (e) {
            console.error(e);
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
                <div className="p-6 border rounded-xl bg-card space-y-4 font-mono text-sm shadow-sm">
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

                <div className="p-6 border rounded-xl bg-card space-y-4 shadow-sm">
                    <h4 className="font-semibold mb-4">Diagnostic Links</h4>
                    <div className="flex flex-col gap-2">
                        <button
                            onClick={openModelsFolder}
                            className="w-full bg-background border hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <FolderOpen className="w-4 h-4 mr-2 text-primary" /> Open Models Folder
                        </button>
                        <button
                            onClick={async () => unwrap(await commands.openImagesFolder())}
                            className="w-full bg-background border hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <ImageIcon className="w-4 h-4 mr-2 text-pink-500" /> Open Generated Images
                        </button>
                        <button
                            onClick={async () => unwrap(await commands.openConfigFile())}
                            className="w-full bg-background border hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <Settings className="w-4 h-4 mr-2 text-muted-foreground" /> Open User Config
                        </button>
                    </div>
                </div>
            </div>

            <div className="p-6 border rounded-xl bg-card space-y-4 shadow-sm">
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

function AppearanceSettings() {
    const {
        darkSyntaxTheme,
        lightSyntaxTheme,
        setSyntaxTheme
    } = useTheme();

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
