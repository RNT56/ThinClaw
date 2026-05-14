import { Trash2, RefreshCw, Download, Search, CheckCircle2, FolderOpen, Globe, Loader2 } from "lucide-react";
import * as Progress from '@radix-ui/react-progress';
import { cn } from "../../lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { useModelContext } from "../model-context";
import { useEffect, useMemo, useState } from "react";
import { commands } from "../../lib/bindings";
import { toast } from "sonner";
import { useConfig } from "../../hooks/use-config";
import { useCloudModels, type CloudModelEntry } from "../../hooks/use-cloud-models";
import { HFDiscovery } from "./HFDiscovery";
import { ActiveEngineChip } from "./ActiveEngineChip";
import { EngineSetupBanner } from "./EngineSetupBanner";

/** Format a short description for a cloud-discovered model. */
function formatCloudDescription(cm: CloudModelEntry): string {
    const parts: string[] = [cm.providerName];
    if (cm.contextWindow) parts.push(`${(cm.contextWindow / 1000).toFixed(0)}K context`);
    if (cm.supportsVision) parts.push('Vision');
    if (cm.supportsTools) parts.push('Tools');
    if (cm.pricing?.inputPerMillion != null) {
        parts.push(`$${cm.pricing.inputPerMillion.toFixed(2)}/1M input`);
    }
    return parts.join(' · ');
}

export function ModelBrowser() {
    const {
        models,
        localModels,
        downloading,
        isRefreshing,
        refreshModels,
        startDownload,
        cancelDownload,
        deleteModel,
        currentModelPath,
        currentEmbeddingModelPath,
        setModelPath,
        setEmbeddingModelPath,
        currentVisionModelPath,
        setVisionModelPath,
        currentSttModelPath,
        setSttModelPath,
        currentImageGenModelPath,
        setImageGenModelPath,
        currentSummarizerModelPath,
        setSummarizerModelPath,
        standardAssets,
        checkStandardAssets,
        downloadStandardAsset,
        engineInfo,
    } = useModelContext();

    // The curated model library is GGUF-only — only relevant for llama.cpp engine
    const isLlamaCpp = engineInfo?.id === 'llamacpp' || engineInfo?.id === 'ollama';

    // Trigger standard asset check on mount
    useEffect(() => {
        checkStandardAssets();
    }, [checkStandardAssets]);

    const [searchQuery, setSearchQuery] = useState("");
    const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
    const [activeCategory, setActiveCategory] = useState("All");
    const [selectedModelVariants, setSelectedModelVariants] = useState<{ model: any, isOpen: boolean } | null>(null);
    const [status, setStatus] = useState<any>(null);
    const { config, updateConfig } = useConfig();

    // Top-level tab: Discover (HF Hub, default) vs My Models (downloaded)
    const [topTab, setTopTab] = useState<"discover" | "library">("discover");

    // Cloud model discovery
    const { models: cloudDiscovered, loading: cloudLoading, refreshAll: refreshCloudModels, totalModels: cloudTotal, providers: cloudProviders, error: cloudError } = useCloudModels();
    // Suppress unused-var warnings for values used in JSX below
    void cloudTotal;

    useEffect(() => {
        const load = async () => {
            try {
                const s = await commands.openclawGetStatus();
                if (s.status === 'ok') setStatus(s.data);
            } catch (e) {
                console.error(e);
            }
        };
        load();
    }, []);

    const isActiveCloud = (model: any) => {
        if (!config || !status || !model?.id) return false;
        const parts = model.id.split('-');
        const provider = parts[0].toLowerCase();
        const modelId = parts.slice(1).join('-');

        const configProvider = config.selected_chat_provider?.toLowerCase();
        const effectiveProvider = (provider === "google" || provider === "gemini") ? "gemini" : provider;

        return configProvider === effectiveProvider && status.selected_cloud_model === modelId;
    };

    const isCloudConfigured = (model: any) => {
        if (model?.category !== "Cloud") return true;
        if (!status || !config) return false;

        const id = model.id.toLowerCase();

        // Detect provider slug from model ID prefix
        const providerMap: [string, string][] = [
            ["anthropic", "anthropic"],
            ["openai", "openai"],
            ["gemini", "gemini"], ["google", "gemini"],
            ["groq", "groq"],
            ["openrouter", "openrouter"],
            ["mistral", "mistral"], ["codestral", "mistral"],
            ["xai", "xai"],
            ["together", "together"],
            ["venice", "venice"],
            ["cohere", "cohere"],
            ["moonshot", "moonshot"],
            ["minimax", "minimax"],
            ["nvidia", "nvidia"],
            ["xiaomi", "xiaomi"],
        ];
        const matched = providerMap.find(([prefix]) => id.startsWith(prefix));
        const provider = matched ? matched[1] : "";

        // Check if disabled in config
        if (provider && config.disabled_providers?.includes(provider)) return false;

        // Original 5 providers use dedicated status keys
        if (provider === "anthropic") return !!(status?.has_anthropic_key || (status as any)?.hasAnthropicKey);
        if (provider === "openai") return !!(status?.has_openai_key || (status as any)?.hasOpenaiKey);
        if (provider === "gemini") return !!(status?.has_gemini_key || (status as any)?.hasGeminiKey);
        if (provider === "groq") return !!(status?.has_groq_key || (status as any)?.hasGroqKey);
        if (provider === "openrouter") return !!(status?.has_openrouter_key || (status as any)?.hasOpenrouterKey);

        // Additional providers use implicit provider key pattern
        const implicitProviders = ["mistral", "xai", "together", "venice", "cohere", "moonshot", "minimax", "nvidia", "xiaomi"];
        if (implicitProviders.includes(provider)) {
            const camel = provider.charAt(0).toUpperCase() + provider.slice(1);
            return !!((status as any)?.[`has_${provider}_key`] || (status as any)?.[`has${camel}Key`]);
        }

        return false;
    };

    const hasAnyCloud = !!(
        status?.has_anthropic_key || (status as any)?.hasAnthropicKey ||
        status?.has_openai_key || (status as any)?.hasOpenaiKey ||
        status?.has_gemini_key || (status as any)?.hasGeminiKey ||
        status?.has_groq_key || (status as any)?.hasGroqKey ||
        status?.has_openrouter_key || (status as any)?.hasOpenrouterKey ||
        (status as any)?.has_mistral_key || (status as any)?.hasMistralKey ||
        (status as any)?.has_xai_key || (status as any)?.hasXaiKey ||
        (status as any)?.has_together_key || (status as any)?.hasTogetherKey ||
        (status as any)?.has_venice_key || (status as any)?.hasVeniceKey ||
        (status as any)?.has_cohere_key || (status as any)?.hasCohereKey ||
        (status as any)?.has_moonshot_key || (status as any)?.hasMoonshotKey ||
        (status as any)?.has_minimax_key || (status as any)?.hasMinimaxKey ||
        (status as any)?.has_nvidia_key || (status as any)?.hasNvidiaKey ||
        (status as any)?.has_xiaomi_key || (status as any)?.hasXiaomiKey
    );

    const unifiedModels = useMemo(() => {
        // Curated GGUF models only for llama.cpp/ollama; cloud models always shown
        const merged = isLlamaCpp
            ? [...models]
            : models.filter(m => m.category === 'Cloud');

        // ── Merge cloud-discovered models ──────────────────────────────────
        // Convert CloudModelEntry to ExtendedModelDefinition-like shape
        const existingCloudIds = new Set(merged.filter(m => (m as any).category === 'Cloud').map(m => m.id.toLowerCase()));

        const discoveredAsModels = cloudDiscovered
            .filter(cm => {
                // Only show chat models in the main browser (other modalities are in InferenceModeTab)
                if (cm.category !== 'chat') return false;
                // Deduplicate against hardcoded entries
                const fullId = `${cm.provider}-${cm.id}`.toLowerCase();
                return !existingCloudIds.has(fullId) && !existingCloudIds.has(cm.id.toLowerCase());
            })
            .map(cm => ({
                id: `${cm.provider}-${cm.id}`,
                name: cm.displayName,
                description: formatCloudDescription(cm),
                family: cm.providerName,
                category: 'Cloud' as const,
                tags: ['Cloud', cm.providerName],
                components: undefined as any,
                mmproj: undefined as any,
                variants: [{
                    name: cm.id,
                    filename: cm.id,
                    url: '',
                    size: 'Cloud',
                    vram_required_gb: 0,
                    recommended_min_ram: 0,
                }],
                // Extra metadata for display
                _cloudMeta: cm,
            }));

        const allMerged = [...merged, ...discoveredAsModels];

        // Helper to get basename
        const getBasename = (path: string) => path.split(/[\\/]/).pop() || path;

        // Collect all component filenames from models
        const curatedComponentFilenames = new Set(
            models.flatMap(m => [
                ...(m.components?.map(c => c.filename) || []),
                ...(m.mmproj ? [m.mmproj.filename] : [])
            ])
        );

        // Collect all variant filenames from models
        const curatedVariantFilenames = new Set(
            models.flatMap(m => m.variants.map(v => v.filename))
        );

        const localOnly = localModels.filter(local => {
            const basename = getBasename(local.name);
            return !curatedComponentFilenames.has(basename) && !curatedVariantFilenames.has(basename);
        });

        const curatedDisplay = allMerged.map(m => {
            // A curated model is "local" if its main variant is downloaded
            // Check if ANY variant matches a local file basename
            const downloadedVariants = m.variants.filter(v =>
                localModels.some(l => getBasename(l.name) === v.filename)
            );

            const isLocal = downloadedVariants.length > 0;
            const activeVariant = downloadedVariants[0] || m.variants[0] || { filename: "" };
            const local = localModels.find(l => getBasename(l.name) === activeVariant.filename);

            // Track status of components
            const componentsStatus = (m.components || []).map((c: any) => ({
                ...c,
                isDownloaded: localModels.some(l => getBasename(l.name) === c.filename)
            }));

            const mmprojStatus = m.mmproj ? {
                ...m.mmproj,
                isDownloaded: localModels.some(l => getBasename(l.name) === m.mmproj?.filename)
            } : null;

            return {
                ...m,
                localPath: local?.path || null,
                isLocal: isLocal,
                isCurated: true,
                displaySize: m.variants[0]?.size || "Cloud",
                filename: activeVariant.filename,
                relativeFilename: local?.name || activeVariant.filename,
                componentsStatus,
                mmprojStatus
            };
        });

        const localDisplay = localOnly.map(l => {
            const ext = l.path.split('.').pop()?.toLowerCase();
            const pathLower = l.path.replace(/\\/g, '/').toLowerCase();
            const nameLower = l.name.replace(/\\/g, '/').toLowerCase();

            // --- Path-based category detection (most reliable) ---
            // Models downloaded via HF discovery go into category subdirectories:
            //   models/Embedding/..., models/STT/..., models/Diffusion/..., models/TTS/...
            const inEmbeddingDir = nameLower.startsWith('embedding/') || pathLower.includes('/models/embedding/');
            const inSttDir = nameLower.startsWith('stt/') || pathLower.includes('/models/stt/');
            const inDiffusionDir = nameLower.startsWith('diffusion/') || pathLower.includes('/models/diffusion/');
            const inTtsDir = nameLower.startsWith('tts/') || pathLower.includes('/models/tts/');

            // --- Keyword heuristics (fallback for models not in category dirs) ---
            const diffusionKeywords = ["diffusion", "flux", "sd-", "stable-diffusion", "sdxl", "sd3"];
            const looksLikeDiffusion = !inDiffusionDir && diffusionKeywords.some(k => pathLower.includes(k) || nameLower.includes(k));
            const isImageGen = inDiffusionDir || looksLikeDiffusion ||
                (ext === "safetensors" || ext === "ckpt" || ext === "pt") && diffusionKeywords.some(k => pathLower.includes(k));

            const embeddingKeywords = ["embed", "nomic", "bge", "bert", "stella", "e5"];
            const isEmbedding = inEmbeddingDir || (!inSttDir && !inDiffusionDir && embeddingKeywords.some(k => pathLower.includes(k) || nameLower.includes(k)));

            const sttKeywords = ["whisper", "parakeet", "voxtral"];
            const isStt = inSttDir || (!inEmbeddingDir && !inDiffusionDir && sttKeywords.some(k => pathLower.includes(k) || nameLower.includes(k)));

            const isTts = inTtsDir;

            let tags: string[] = ["Local"];
            let family = "Unknown";
            let description = "Local Model";

            if (isImageGen) {
                tags.push("Image Gen", "Diffusion");
                family = "Stable Diffusion";
                description = "Local Diffusion/Image Model";
            } else if (isEmbedding) {
                tags.push("Embedding");
                family = "BERT/Embedding";
                description = "Local Embedding Model";
            } else if (isStt) {
                tags.push("STT");
                family = "Whisper";
                description = "Local Speech-to-Text Model";
            } else if (isTts) {
                tags.push("TTS");
                family = "TTS";
                description = "Local Text-to-Speech Model";
            } else {
                tags.push("Chat");
                description = "Local Chat/LLM Model";
            }

            return {
                name: l.name.split(/[\\/]/).pop() || l.name,
                description,
                filename: l.name,
                url: "",
                size: l.size.toString(),
                displaySize: (l.size / 1024 / 1024 / 1024).toFixed(2) + " GB",
                localPath: l.path,
                isLocal: true,
                isCurated: false,
                id: l.name,
                family,
                vram_required_gb: 0,
                recommended_min_ram: 0,
                tags,
                manual_download: false,
                info_url: undefined,
                relativeFilename: l.name
            };
        });

        const allModels = [...curatedDisplay, ...localDisplay].filter(m => {
            if (searchQuery.trim() === "") return true;
            const query = searchQuery.toLowerCase();
            return (
                m.name.toLowerCase().includes(query) ||
                m.description.toLowerCase().includes(query) ||
                m.family.toLowerCase().includes(query) ||
                m.tags?.some(t => t.toLowerCase().includes(query)) ||
                m.filename.toLowerCase().includes(query)
            );
        });

        // Sorting: Local first, then by family/name
        return allModels.sort((a, b) => {
            // Cloud Brains tab: group by family
            if (activeCategory === "Cloud Brains") {
                if (a.family !== b.family) return a.family.localeCompare(b.family);
                return a.name.localeCompare(b.name);
            }

            // "All" view: group by category, then local-before-cloud, then name
            if (activeCategory === "All") {
                const catOrder: Record<string, number> = { Cloud: 99 };
                const aCatRank = catOrder[(a as any).category] ?? 0;
                const bCatRank = catOrder[(b as any).category] ?? 0;
                if (aCatRank !== bCatRank) return aCatRank - bCatRank;
            }

            if (a.isLocal && !b.isLocal) return -1;
            if (!a.isLocal && b.isLocal) return 1;
            return a.name.localeCompare(b.name);
        });
    }, [models, localModels, searchQuery, activeCategory, isLlamaCpp, currentModelPath, currentEmbeddingModelPath, currentVisionModelPath, currentSttModelPath, currentImageGenModelPath, currentSummarizerModelPath, config, status, cloudDiscovered]);

    const isActive = (path: string | null) => path && currentModelPath && path === currentModelPath;
    const isEmbeddingActive = (path: string | null) => path && currentEmbeddingModelPath && path === currentEmbeddingModelPath;
    const isVisionActive = (path: string | null) => path && currentVisionModelPath && path === currentVisionModelPath;
    const isSttActive = (path: string | null) => path && currentSttModelPath && path === currentSttModelPath;
    const isImageGenActive = (path: string | null) => path && currentImageGenModelPath && path === currentImageGenModelPath;
    const isSummarizerActive = (path: string | null) => path && currentSummarizerModelPath && path === currentSummarizerModelPath;

    return (
        <div className="space-y-4">
            {/* Active engine indicator */}
            <div className="flex justify-end">
                <ActiveEngineChip />
            </div>

            {/* Top-level Tab Bar: Discover | My Models */}
            <div className="flex gap-1 bg-muted/30 p-1 rounded-xl border border-border/30">
                <button
                    onClick={() => setTopTab("discover")}
                    className={cn(
                        "flex-1 py-2 px-4 rounded-lg text-sm font-medium transition-all flex items-center justify-center gap-2",
                        topTab === "discover"
                            ? "bg-background text-foreground shadow-sm"
                            : "text-muted-foreground hover:text-foreground"
                    )}
                    id="tab-discover"
                >
                    <Globe className="w-3.5 h-3.5" />
                    Discover
                </button>
                <button
                    onClick={() => setTopTab("library")}
                    className={cn(
                        "flex-1 py-2 px-4 rounded-lg text-sm font-medium transition-all flex items-center justify-center gap-2",
                        topTab === "library"
                            ? "bg-background text-foreground shadow-sm"
                            : "text-muted-foreground hover:text-foreground"
                    )}
                    id="tab-library"
                >
                    My Models
                    {localModels.length > 0 && (
                        <span className="text-[10px] bg-muted/80 text-muted-foreground px-1.5 py-0.5 rounded-full font-mono">
                            {localModels.length}
                        </span>
                    )}
                </button>
            </div>

            {/* Engine Setup Banner (shown if MLX/vLLM needs bootstrap) */}
            <EngineSetupBanner />

            {/* Discover Tab — kept mounted so local state (file info cache) survives tab switches */}
            <div style={{ display: topTab === "discover" ? "block" : "none" }}>
                <HFDiscovery isVisible={topTab === "discover"} />
            </div>

            {/* Library Tab (existing content) */}
            {topTab === "library" && <>
                {/* Sticky Header Container */}
                <div className="sticky top-0 z-10 bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60 -mx-1 px-1 py-4 space-y-4">
                    <div className="flex flex-col gap-3">
                        <div className="flex justify-end items-center h-4 gap-2">
                            {cloudLoading && (
                                <span className="flex items-center gap-1 text-[10px] text-muted-foreground">
                                    <Loader2 className="w-3 h-3 animate-spin" />
                                    Discovering cloud models...
                                </span>
                            )}
                            {/* Cloud discovery error badge */}
                            {!cloudLoading && cloudProviders.some(p => p.error) && (
                                <span
                                    className="flex items-center gap-1 text-[10px] text-amber-500 cursor-help"
                                    title={cloudProviders.filter(p => p.error).map(p => `${p.provider}: ${p.error}`).join('\n')}
                                >
                                    ⚠️ {cloudProviders.filter(p => p.error).length} provider{cloudProviders.filter(p => p.error).length > 1 ? 's' : ''} failed
                                </span>
                            )}
                            {cloudError && (
                                <span className="text-[10px] text-destructive" title={cloudError}>
                                    Discovery failed
                                </span>
                            )}
                            <button
                                onClick={() => refreshCloudModels()}
                                className="p-1 hover:bg-accent rounded-md transition-colors"
                                title="Refresh cloud models"
                            >
                                <Globe className={cn("w-3.5 h-3.5 text-muted-foreground", cloudLoading && "animate-pulse")} />
                            </button>
                            <button onClick={refreshModels} disabled={isRefreshing} className="p-1 hover:bg-accent rounded-md transition-colors" title="Refresh">
                                <RefreshCw className={cn("w-4 h-4 text-muted-foreground", isRefreshing && "animate-spin")} />
                            </button>
                        </div>
                        <div className="relative">
                            <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
                            <input
                                type="text"
                                placeholder="Search models..."
                                value={searchQuery}
                                onChange={(e) => setSearchQuery(e.target.value)}
                                className="w-full pl-10 pr-4 py-2.5 text-sm bg-muted/50 border-none rounded-xl focus:outline-none focus:ring-1 focus:ring-primary/20 transition-all"
                            />
                        </div>
                    </div>

                    <div className="flex gap-2 pb-1 overflow-x-auto w-full min-w-0 no-scrollbar mask-fade-right scroll-smooth snap-x">
                        {(isLlamaCpp
                            ? ["All", ...(hasAnyCloud ? ["Cloud Brains"] : []), "Chat", "Summarizer", "Diffusion", "STT", "Embedding", "Standard"]
                            : ["All", ...(hasAnyCloud ? ["Cloud Brains"] : []), "Chat", "Diffusion", "STT", "Embedding"]
                        ).map((cat) => (
                            <button
                                key={cat}
                                onClick={() => {
                                    setActiveCategory(cat);
                                    setSearchQuery("");
                                }}
                                className={cn(
                                    "px-4 py-1.5 rounded-full text-xs font-medium transition-all whitespace-nowrap border flex-shrink-0 snap-start",
                                    activeCategory === cat
                                        ? "bg-foreground text-background border-foreground shadow-sm"
                                        : "bg-muted/50 text-muted-foreground border-transparent hover:bg-muted hover:text-foreground"
                                )}
                            >
                                {cat}
                            </button>
                        ))}
                    </div>
                </div>

                <div className="grid gap-4">
                    {/* Standard Assets Section — llama.cpp only (GGUF components) */}
                    {isLlamaCpp && activeCategory === "Standard" && (
                        <div className="space-y-4">
                            <div className="text-xs text-muted-foreground bg-muted/20 p-4 rounded-2xl border border-border/40 flex justify-between items-center">
                                <span className="leading-relaxed">
                                    These standard components (VAE, CLIP, T5, etc.) are used as fallbacks if your model is missing them.
                                    If a folder is empty, click download to restore the asset.
                                </span>
                                <button
                                    onClick={() => commands.openStandardModelsFolder()}
                                    className="bg-background border border-border/50 hover:bg-accent hover:border-border text-foreground px-3 py-1.5 rounded-xl transition-all text-xs font-medium flex items-center shrink-0 ml-4 shadow-sm"
                                >
                                    <FolderOpen className="w-3.5 h-3.5 mr-1.5" /> Open Folder
                                </button>
                            </div>
                            {standardAssets.length === 0 ? (
                                <div className="text-sm text-center py-4 text-emerald-600 dark:text-emerald-400 flex items-center justify-center gap-2">
                                    <CheckCircle2 className="w-4 h-4" /> All standard assets are present.
                                </div>
                            ) : (
                                standardAssets.map(asset => {
                                    const progress = downloading[asset.filename];
                                    const isDownloading = progress !== undefined;
                                    return (
                                        <div key={asset.filename} className="flex flex-col p-5 border border-border/50 rounded-2xl bg-card/40 hover:bg-card/60 transition-all duration-300">
                                            <div className="flex items-start justify-between mb-4">
                                                <div className="min-w-0">
                                                    <h3 className="font-semibold text-base flex items-center gap-2 mb-1" title={asset.name}>
                                                        <span className="truncate">{asset.name}</span>
                                                        <span className="text-[10px] bg-amber-500/10 text-amber-600 dark:text-amber-400 px-2 py-0.5 rounded-md uppercase font-bold tracking-wider border border-amber-500/20">{asset.category}</span>
                                                    </h3>
                                                    <p className="text-sm text-muted-foreground truncate" title={asset.filename}>{asset.filename}</p>
                                                </div>
                                                <div className="text-[11px] font-mono bg-muted/50 px-2.5 py-1 rounded-lg text-muted-foreground border border-border/5 whitespace-nowrap">
                                                    {(asset.size / 1024 / 1024).toFixed(1)} MB
                                                </div>
                                            </div>
                                            {isDownloading ? (
                                                <div className="space-y-2">
                                                    <div className="flex justify-between items-center text-xs text-muted-foreground">
                                                        <span>{progress === 0 ? "Starting..." : `Downloading... ${progress.toFixed(1)}%`}</span>
                                                    </div>
                                                    <Progress.Root className="relative overflow-hidden bg-secondary rounded-full w-full h-2" value={progress}>
                                                        <Progress.Indicator className="bg-primary w-full h-full transition-transform duration-500 ease-in-out" style={{ transform: `translateX(-${100 - (progress || 0)}%)` }} />
                                                    </Progress.Root>
                                                </div>
                                            ) : (
                                                <button
                                                    onClick={() => downloadStandardAsset(asset.filename)}
                                                    className="w-full border border-primary/30 hover:bg-primary hover:text-primary-foreground text-primary py-2.5 px-4 rounded-xl text-sm font-bold uppercase tracking-wider flex items-center justify-center transition-all shadow-sm hover:translate-y-[-1px]"
                                                >
                                                    <Download className="w-4 h-4 mr-2" /> Download Missing Asset
                                                </button>
                                            )}
                                        </div>
                                    )
                                })
                            )}
                        </div>
                    )}

                    {activeCategory !== "Standard" && unifiedModels.filter(m => {
                        // Global visibility check: only show cloud models if configured
                        if (!isCloudConfigured(m)) return false;

                        const isCloud = (m as any).category === "Cloud";

                        if (activeCategory === "All") return true;
                        if (activeCategory === "Cloud Brains") return isCloud;

                        // Exclude Cloud models from all other specific (Local) tabs
                        if (isCloud) return false;

                        if (activeCategory === "Chat" || activeCategory === "Summarizer") {
                            // Include local LLMs
                            return !m.tags?.some(t => ["Image Gen", "STT", "Embedding"].includes(t));
                        }
                        if (activeCategory === "Diffusion") return m.tags?.includes("Image Gen");
                        if (activeCategory === "STT") return m.tags?.includes("STT");
                        if (activeCategory === "Embedding") return m.tags?.includes("Embedding");
                        return true;
                    }).map((model) => {
                        const category = (model as any).category || "LLM";
                        const sanitizedName = model.name.replace(/[^a-zA-Z0-9-_]/g, "_");
                        const fullPath = `${category}/${sanitizedName}/${model.filename}`;
                        // Check full path (event) then short filename (initial)
                        // Use ?? to ensure 0 is treated as a valid value
                        const progress = downloading[fullPath] ?? downloading[model.filename];
                        const isDownloading = progress !== undefined;
                        const isModelActive = (model as any).category === 'Cloud' ? isActiveCloud(model) : isActive(model.localPath);
                        const isEmbedding = isEmbeddingActive(model.localPath);
                        const isVision = isVisionActive(model.localPath);
                        const isStt = isSttActive(model.localPath);
                        const isImageGen = isImageGenActive(model.localPath);
                        const isSummarizer = (model as any).category === 'Cloud'
                            ? (currentSummarizerModelPath === model.id)
                            : isSummarizerActive(model.localPath);
                        const isDownloaded = model.isLocal || (model as any).category === "Cloud";
                        const modelAny = model as any;
                        const rFilename = modelAny.relativeFilename || model.filename;
                        const isConfirming = confirmingDelete === rFilename;
                        const hasEmbeddingTag = model.tags && model.tags.includes("Embedding");
                        const hasVisionTag = model.tags && (model.tags.includes("Vision") || model.tags.includes("Multi-modal"));
                        const hasSttTag = model.tags && (model.tags.includes("STT") || model.family === "Whisper");
                        const hasImageGenTag = model.tags && (model.tags.includes("Image Gen") || model.family === "Stable Diffusion");

                        return (
                            <div key={model.id} className={cn(
                                "flex flex-col p-5 border rounded-2xl transition-all duration-300",
                                isModelActive
                                    ? "bg-accent/40 border-primary/20 shadow-inner"
                                    : "bg-card/40 border-border/50 hover:border-border hover:bg-card/60 shadow-sm"
                            )}>
                                <div className="flex items-start justify-between mb-4">
                                    <div className="min-w-0 flex-1">
                                        <h3 className="font-semibold text-base mb-1.5 flex items-center gap-2" title={model.name}>
                                            <span className="truncate">{model.name}</span>
                                            <div className="flex gap-1 flex-wrap">
                                                {isModelActive && <span className="text-[10px] uppercase tracking-wider font-bold bg-primary text-primary-foreground px-2 py-0.5 rounded-md">Primary</span>}
                                                {isSummarizer && <span className="text-[10px] uppercase tracking-wider font-bold bg-emerald-500 text-white px-2 py-0.5 rounded-md">Summarizer</span>}
                                                {isEmbedding && <span className="text-[10px] uppercase tracking-wider font-bold bg-cyan-500 text-white px-2 py-0.5 rounded-md">Embedding</span>}
                                                {isVision && <span className="text-[10px] uppercase tracking-wider font-bold bg-indigo-500 text-white px-2 py-0.5 rounded-md">Vision</span>}
                                                {isStt && <span className="text-[10px] uppercase tracking-wider font-bold bg-amber-500 text-white px-2 py-0.5 rounded-md">STT</span>}
                                                {isImageGen && <span className="text-[10px] uppercase tracking-wider font-bold bg-muted text-muted-foreground px-2 py-0.5 rounded-md">Image Gen</span>}
                                                {model.isCurated && model.isLocal && <span className="text-[10px] uppercase tracking-wider font-bold bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 px-2 py-0.5 rounded-md border border-emerald-500/10">Installed</span>}
                                                {!model.isCurated && <span className="text-[10px] uppercase tracking-wider font-bold bg-muted/50 text-muted-foreground/50 px-2 py-0.5 rounded-md border border-border/10">Local</span>}
                                                {category === "Cloud" && (() => {
                                                    const id = model.id.toLowerCase();
                                                    const badges: [string, string][] = [
                                                        ["anthropic", "Anthropic"], ["openai", "OpenAI"],
                                                        ["google", "Google"], ["gemini", "Google"],
                                                        ["groq", "Groq"], ["openrouter", "OpenRouter"],
                                                        ["mistral", "Mistral"], ["codestral", "Mistral"],
                                                        ["xai", "xAI"], ["together", "Together"],
                                                        ["venice", "Venice"], ["cohere", "Cohere"],
                                                        ["moonshot", "Moonshot"], ["minimax", "MiniMax"],
                                                        ["nvidia", "NVIDIA"], ["xiaomi", "Xiaomi"],
                                                    ];
                                                    const label = badges.find(([p]) => id.startsWith(p))?.[1] ?? "Cloud";
                                                    return (
                                                        <span className="text-[10px] uppercase tracking-wider font-bold bg-indigo-500/10 text-indigo-500 border border-indigo-500/20 px-2 py-0.5 rounded-md">
                                                            {label}
                                                        </span>
                                                    );
                                                })()}
                                            </div>
                                        </h3>
                                        <p className="text-sm text-muted-foreground line-clamp-2" title={model.description}>{model.description}</p>
                                    </div>
                                    <div className="text-xs font-mono bg-muted px-2 py-1 rounded text-muted-foreground whitespace-nowrap">
                                        {model.displaySize}
                                    </div>
                                </div>

                                {/* Nested Component Presence Check */}
                                {model.isCurated && !isDownloading && (((model as any).componentsStatus?.length > 0) || (model as any).mmprojStatus) && (
                                    <div className="mb-4 space-y-1.5 bg-muted/20 p-3 rounded-xl border border-border/5">
                                        <p className="text-[10px] uppercase tracking-wider font-bold text-muted-foreground/40 mb-1">Support Components</p>
                                        {[...((model as any).componentsStatus || []), (model as any).mmprojStatus].filter(Boolean).map((comp: any) => (
                                            <div key={comp.filename} className="flex items-center justify-between text-[11px]">
                                                <div className="flex items-center gap-2 min-w-0">
                                                    <div className={cn("w-1.5 h-1.5 rounded-full shrink-0", comp.isDownloaded ? "bg-emerald-500" : "bg-amber-500 animate-pulse")} />
                                                    <span className="truncate text-muted-foreground/80 font-mono text-[10px]">{comp.filename}</span>
                                                    <span className="text-[9px] bg-background/50 border border-border/10 px-1 rounded opacity-70 uppercase font-bold text-muted-foreground/60">{comp.type || 'proj'}</span>
                                                </div>
                                                {comp.isDownloaded ? (
                                                    <span className="text-emerald-600/70 dark:text-emerald-400/70 font-medium">Ready</span>
                                                ) : (
                                                    <button
                                                        onClick={() => {
                                                            const m = model as any;
                                                            const variant = m.variants?.find((v: any) => v.filename === m.filename) || m.variants?.[0];
                                                            startDownload(m, variant);
                                                        }}
                                                        className="text-primary hover:text-primary/80 transition-colors font-semibold"
                                                    >
                                                        {downloading[comp.filename] ? `${downloading[comp.filename].toFixed(0)}%` : "Download"}
                                                    </button>
                                                )}
                                            </div>
                                        ))}
                                    </div>
                                )}

                                {isDownloading ? (
                                    <div className="space-y-2">
                                        <div className="flex justify-between items-center text-xs text-muted-foreground">
                                            <span>
                                                {progress === 0 ? "Connecting..." : `Downloading... ${progress.toFixed(1)}%`}
                                            </span>
                                            <button
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    cancelDownload(fullPath);
                                                    if (fullPath !== model.filename) cancelDownload(model.filename);
                                                }}
                                                className="text-destructive hover:text-destructive/80 font-medium"
                                            >
                                                Cancel
                                            </button>
                                        </div>
                                        <Progress.Root className="relative overflow-hidden bg-secondary rounded-full w-full h-2" value={progress}>
                                            <Progress.Indicator
                                                className="bg-primary w-full h-full transition-transform duration-500 ease-in-out"
                                                style={{ transform: `translateX(-${100 - (progress || 0)}%)` }}
                                            />
                                        </Progress.Root>

                                        {/* Nested Component Progress */}
                                        {model.isCurated && (
                                            <div className="space-y-2 mt-3 pt-3 border-t border-border/10">
                                                {[...((model as any).components || []), (model as any).mmproj].filter(Boolean).map((comp: any) => {
                                                    const c = comp;
                                                    const category = (model as any).category || "LLM";
                                                    const sanitizedName = model.name.replace(/[^a-zA-Z0-9-_]/g, "_");
                                                    const fullPath = `${category}/${sanitizedName}/${c.filename}`;
                                                    const compProgress = downloading[fullPath] ?? downloading[c.filename]; // Check both full and short for safety

                                                    if (compProgress === undefined) return null;

                                                    return (
                                                        <div key={c.filename} className="pl-4 border-l-2 border-primary/20 space-y-1">
                                                            <div className="flex justify-between items-center text-[10px] text-muted-foreground opacity-80">
                                                                <span className="truncate max-w-[200px]">{c.filename}</span>
                                                                <span>{compProgress.toFixed(1)}%</span>
                                                            </div>
                                                            <Progress.Root className="relative overflow-hidden bg-secondary/50 rounded-full w-full h-1" value={compProgress}>
                                                                <Progress.Indicator
                                                                    className="bg-primary/60 w-full h-full transition-transform duration-500 ease-in-out"
                                                                    style={{ transform: `translateX(-${100 - compProgress}%)` }}
                                                                />
                                                            </Progress.Root>
                                                        </div>
                                                    );
                                                })}
                                            </div>
                                        )}
                                    </div>
                                ) : isDownloaded ? (
                                    <div className="flex gap-2">
                                        <button
                                            onClick={(e) => {
                                                e.preventDefault();
                                                e.stopPropagation();
                                                if (isConfirming) {
                                                    deleteModel(rFilename);
                                                    setConfirmingDelete(null);
                                                } else {
                                                    setConfirmingDelete(rFilename);
                                                    setTimeout(() => setConfirmingDelete(null), 3000);
                                                }
                                            }}
                                            className={cn(
                                                "py-2 px-3 rounded-md text-sm font-medium flex items-center justify-center transition-all duration-200",
                                                isConfirming
                                                    ? "bg-destructive text-destructive-foreground hover:bg-destructive/90 w-24"
                                                    : "text-muted-foreground hover:text-destructive hover:bg-destructive/10 w-10"
                                            )}
                                            title={isConfirming ? "Confirm Delete" : "Delete local model"}
                                        >
                                            {isConfirming ? "Confirm" : <Trash2 className="w-4 h-4" />}
                                        </button>

                                        {model.isCurated && (model as any).variants?.length > 1 && (
                                            <button
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    setSelectedModelVariants({ model, isOpen: true });
                                                }}
                                                className="bg-muted hover:bg-muted/80 text-muted-foreground hover:text-foreground py-2 px-3 rounded-md transition-colors border border-border/10"
                                                title="Get other quantizations/versions"
                                            >
                                                <FolderOpen className="w-4 h-4" />
                                            </button>
                                        )}

                                        <div className="flex flex-wrap gap-2 flex-1">
                                            {!hasEmbeddingTag && !hasSttTag && !hasImageGenTag && (
                                                <>
                                                    <button
                                                        onClick={async () => {
                                                            if (model.localPath) {
                                                                if (config?.selected_chat_provider !== "local") {
                                                                    try {
                                                                        const newConfig = { ...config, selected_chat_provider: "local" };
                                                                        await updateConfig(newConfig);
                                                                    } catch (e) {
                                                                        console.error(e);
                                                                    }
                                                                }
                                                                setModelPath(model.localPath, (model as any).template);
                                                            }
                                                        }}
                                                        className={cn(
                                                            "flex-1 py-2 px-3 rounded-xl text-xs font-bold uppercase tracking-wider transition-all",
                                                            isModelActive
                                                                ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20 shadow-sm ring-1 ring-emerald-500/10"
                                                                : "bg-secondary hover:bg-secondary/80 text-secondary-foreground border border-transparent shadow-sm hover:translate-y-[-1px]"
                                                        )}
                                                        disabled={!!isModelActive}
                                                    >
                                                        {isModelActive ? "Active" : "Chat"}
                                                    </button>
                                                    <button
                                                        onClick={() => model.localPath && setSummarizerModelPath(model.localPath)}
                                                        className={cn(
                                                            "flex-1 py-1.5 px-3 rounded-xl text-xs font-medium flex items-center justify-center border transition-all",
                                                            isSummarizer
                                                                ? "bg-muted text-muted-foreground border-border/50 cursor-default"
                                                                : "border-input hover:bg-accent hover:text-accent-foreground shadow-sm"
                                                        )}
                                                        disabled={!!isSummarizer}
                                                    >
                                                        {isSummarizer ? "Summ. Active" : "Set Summ."}
                                                    </button>
                                                </>
                                            )}

                                            {hasVisionTag && (
                                                <button
                                                    onClick={() => model.localPath && setVisionModelPath(model.localPath)}
                                                    className={cn(
                                                        "flex-1 py-1.5 px-3 rounded-xl text-xs font-medium flex items-center justify-center border transition-all",
                                                        isVision
                                                            ? "bg-muted text-muted-foreground border-border/50 cursor-default"
                                                            : "border-input hover:bg-accent hover:text-accent-foreground shadow-sm"
                                                    )}
                                                    disabled={!!isVision}
                                                >
                                                    {isVision ? "Vision Active" : "Set Vision"}
                                                </button>
                                            )}

                                            {hasSttTag && (
                                                <button
                                                    onClick={() => model.localPath && setSttModelPath(model.localPath)}
                                                    className={cn(
                                                        "flex-1 py-1.5 px-3 rounded-xl text-xs font-medium flex items-center justify-center border transition-all",
                                                        isStt
                                                            ? "bg-muted text-muted-foreground border-border/50 cursor-default"
                                                            : "border-input hover:bg-accent hover:text-accent-foreground shadow-sm"
                                                    )}
                                                    disabled={!!isStt}
                                                >
                                                    {isStt ? "STT Active" : "Set STT"}
                                                </button>
                                            )}

                                            {hasImageGenTag && (
                                                <button
                                                    onClick={() => model.localPath && setImageGenModelPath(model.localPath)}
                                                    className={cn(
                                                        "flex-1 py-1.5 px-3 rounded-xl text-xs font-medium flex items-center justify-center border transition-all",
                                                        isImageGen
                                                            ? "bg-muted text-muted-foreground border-border/50 cursor-default"
                                                            : "border-input hover:bg-accent hover:text-accent-foreground shadow-sm"
                                                    )}
                                                    disabled={!!isImageGen}
                                                >
                                                    {isImageGen ? "Gen Active" : "Set Image Gen"}
                                                </button>
                                            )}

                                            {hasEmbeddingTag && (
                                                <button
                                                    onClick={() => model.localPath && setEmbeddingModelPath(model.localPath)}
                                                    className={cn(
                                                        "flex-1 py-1.5 px-3 rounded-xl text-xs font-medium flex items-center justify-center border transition-all",
                                                        isEmbedding
                                                            ? "bg-muted text-muted-foreground border-border/50 cursor-default"
                                                            : "border-input hover:bg-accent hover:text-accent-foreground shadow-sm"
                                                    )}
                                                    disabled={!!isEmbedding}
                                                >
                                                    {isEmbedding ? "Embedder Active" : "Set Embedder"}
                                                </button>
                                            )}
                                        </div>
                                    </div>
                                ) : (model as any).category === "Cloud" ? (
                                    <div className="flex gap-2 flex-wrap w-full">
                                        <button
                                            onClick={async () => {
                                                try {
                                                    const id = model.id.toLowerCase();
                                                    const brainMap: [string, string][] = [
                                                        ["openrouter-", "openrouter"], ["groq-", "groq"],
                                                        ["anthropic-", "anthropic"], ["openai-", "openai"],
                                                        ["google-", "gemini"], ["gemini-", "gemini"],
                                                        ["mistral-", "mistral"], ["codestral-", "mistral"],
                                                        ["xai-", "xai"], ["together-", "together"],
                                                        ["venice-", "venice"], ["cohere-", "cohere"],
                                                        ["moonshot-", "moonshot"], ["minimax-", "minimax"],
                                                        ["nvidia-", "nvidia"], ["xiaomi-", "xiaomi"],
                                                    ];
                                                    const brain = brainMap.find(([p]) => id.startsWith(p))?.[1] ?? model.family.toLowerCase();
                                                    const modelId = model.id.split('-').slice(1).join('-');
                                                    // Propagate context window from discovery metadata
                                                    const contextSize = (model as any)._cloudMeta?.contextWindow ?? null;
                                                    const cfg = await commands.getUserConfig();
                                                    const newConfig = {
                                                        ...cfg,
                                                        selected_chat_provider: brain,
                                                        selected_cloud_brain: brain,
                                                        selected_cloud_model: modelId,
                                                        selected_model_context_size: contextSize ?? undefined,
                                                    };
                                                    await updateConfig(newConfig);
                                                    if (commands.openclawSaveSelectedCloudModel) {
                                                        await commands.openclawSaveSelectedCloudModel(modelId);
                                                    }
                                                    const providerName = brain === "gemini" ? "Google" : brain.charAt(0).toUpperCase() + brain.slice(1);
                                                    toast.success(`${model.name} selected as active ${providerName} Brain`);
                                                    const s = await commands.openclawGetStatus();
                                                    if (s.status === 'ok') setStatus(s.data);
                                                } catch (e) {
                                                    toast.error("Failed to select cloud model");
                                                }
                                            }}
                                            className={cn(
                                                "flex-1 py-2 px-3 rounded-xl text-xs font-bold uppercase tracking-wider transition-all",
                                                isModelActive
                                                    ? "bg-indigo-500/10 text-indigo-600 dark:text-indigo-400 border border-indigo-500/20 shadow-sm ring-1 ring-indigo-500/10"
                                                    : "bg-secondary hover:bg-secondary/80 text-secondary-foreground border border-transparent shadow-sm hover:translate-y-[-1px]"
                                            )}
                                            disabled={!!isModelActive}
                                        >
                                            {isModelActive ? "Active" : "Select Brain"}
                                        </button>
                                        <button
                                            onClick={() => setSummarizerModelPath(model.id)}
                                            className={cn(
                                                "flex-1 py-1.5 px-3 rounded-xl text-xs font-medium flex items-center justify-center border transition-all",
                                                isSummarizer
                                                    ? "bg-muted text-muted-foreground border-border/50 cursor-default"
                                                    : "border-input hover:bg-accent hover:text-accent-foreground shadow-sm"
                                            )}
                                            disabled={!!isSummarizer}
                                        >
                                            {isSummarizer ? "Summ. Active" : "Set Summ."}
                                        </button>
                                    </div>
                                ) : (
                                    <div className="flex gap-2">
                                        {(isModelActive || isEmbedding || isVision || isStt || isImageGen || isSummarizer) && (
                                            <button
                                                onClick={async () => {
                                                    if (isModelActive) {
                                                        if ((model as any).category === 'Cloud') {
                                                            // For cloud models, deactivation means switching back to Local Neural Link
                                                            const newConfig = { ...config, selected_chat_provider: null, selected_cloud_model: null };
                                                            await updateConfig(newConfig);
                                                            if (commands.openclawSaveSelectedCloudModel) {
                                                                await commands.openclawSaveSelectedCloudModel(null);
                                                            }
                                                            toast.success("Switched to Local Neural Link");
                                                            const s = await commands.openclawGetStatus();
                                                            if (s.status === 'ok') setStatus(s.data);
                                                        } else {
                                                            setModelPath("");
                                                        }
                                                    }
                                                    if (isEmbedding) setEmbeddingModelPath("");
                                                    if (isVision) setVisionModelPath("");
                                                    if (isStt) setSttModelPath("");
                                                    if (isImageGen) setImageGenModelPath("");
                                                    if (isSummarizer) setSummarizerModelPath("");
                                                }}
                                                className="flex-1 py-2.5 px-4 rounded-xl text-sm font-bold uppercase tracking-wider transition-all bg-destructive/10 text-destructive border border-destructive/20 hover:bg-destructive hover:text-destructive-foreground"
                                            >
                                                Deactivate
                                            </button>
                                        )}

                                        {(!isModelActive || !isEmbedding || !isVision || !isStt || !isImageGen || !isSummarizer) && (
                                            <button
                                                onClick={() => {
                                                    if (model.isCurated && (model as any).manual_download) {
                                                        const url = (model as any).info_url || (model as any).url;
                                                        if (url) invoke('open_url', { url });
                                                    } else if (model.isCurated && (model as any).variants?.length > 1) {
                                                        setSelectedModelVariants({ model, isOpen: true });
                                                    } else if (model.isCurated && (model as any).variants?.length === 1) {
                                                        startDownload(model as any, (model as any).variants[0]);
                                                    } else {
                                                        // Local model or legacy handled by select buttons
                                                    }
                                                }}
                                                className="w-full border border-primary/30 hover:bg-primary hover:text-primary-foreground text-primary py-2.5 px-4 rounded-xl text-sm font-bold uppercase tracking-wider flex items-center justify-center transition-all shadow-sm hover:translate-y-[-1px]"
                                            >
                                                <Download className="w-4 h-4 mr-2" />
                                                {model.isCurated && (model as any).manual_download
                                                    ? "Manual Download"
                                                    : (model.isCurated && (model as any).variants?.length > 1 ? "Select Quantization" : "Download")}
                                            </button>
                                        )}
                                    </div>
                                )}
                            </div>
                        );
                    })}
                </div>

                {unifiedModels.length === 0 && !isRefreshing && (
                    <div className="text-center py-12 space-y-3">
                        {isLlamaCpp ? (
                            <>
                                <p className="text-muted-foreground text-sm">
                                    No models found. Check your connection or add local files.
                                </p>
                            </>
                        ) : (
                            <>
                                <div className="text-muted-foreground/50">
                                    <Globe className="w-8 h-8 mx-auto mb-3 opacity-40" />
                                </div>
                                <p className="text-muted-foreground text-sm">
                                    No downloaded models yet
                                </p>
                                <p className="text-muted-foreground/60 text-xs">
                                    Head to the <strong>Discover</strong> tab to browse and download models from HuggingFace
                                </p>
                                <button
                                    onClick={() => setTopTab("discover")}
                                    className="mt-2 px-4 py-2 text-xs font-medium bg-primary/10 text-primary hover:bg-primary/20 rounded-xl transition-all border border-primary/20"
                                >
                                    <Globe className="w-3.5 h-3.5 inline-block mr-1.5 -mt-0.5" />
                                    Browse Models
                                </button>
                            </>
                        )}
                    </div>
                )}

                {/* Quantization selection modal */}
                {selectedModelVariants?.isOpen && selectedModelVariants.model && (
                    <div className="fixed inset-0 z-[100] flex items-center justify-center p-4">
                        <div className="absolute inset-0 bg-background/80 backdrop-blur-sm" onClick={() => setSelectedModelVariants(null)} />
                        <div className="relative bg-card border border-white/10 rounded-2xl shadow-2xl w-full max-w-md overflow-hidden animate-in fade-in zoom-in duration-200">
                            <div className="p-6 space-y-4">
                                <div>
                                    <h3 className="text-xl font-bold">{selectedModelVariants.model.name}</h3>
                                    <p className="text-sm text-muted-foreground">Select a quantization variant to download.</p>
                                </div>

                                <div className="space-y-2 max-h-[400px] overflow-y-auto pr-2">
                                    {selectedModelVariants.model.variants.map((v: any) => {
                                        // Use basename matching to support subfolders
                                        const isLocal = localModels.some(l => (l.name.split(/[\\/]/).pop() || l.name) === v.filename);

                                        // Robust progress lookup
                                        const category = (selectedModelVariants.model as any).category || "LLM";
                                        const sanitizedName = selectedModelVariants.model.name.replace(/[^a-zA-Z0-9-_]/g, "_");
                                        const fullPath = `${category}/${sanitizedName}/${v.filename}`;
                                        const progress = downloading[fullPath] ?? downloading[v.filename];

                                        const isDownloading = progress !== undefined;

                                        const isVariantActive = (selectedModelVariants.model as any).category === 'Cloud'
                                            ? isActiveCloud(selectedModelVariants.model)
                                            : (isLocal && localModels.find(l => (l.name.split(/[\\/]/).pop() || l.name) === v.filename)?.path === currentModelPath);

                                        return (
                                            <button
                                                key={v.filename}
                                                disabled={(isLocal && !isVariantActive) || isDownloading}
                                                onClick={() => {
                                                    if (isVariantActive) return;
                                                    startDownload(selectedModelVariants.model, v);
                                                    setSelectedModelVariants(null);
                                                }}
                                                className={cn(
                                                    "w-full flex items-center justify-between p-4 rounded-xl border transition-all text-left group",
                                                    (isLocal && !isVariantActive)
                                                        ? "bg-muted/50 border-border/50 opacity-60 cursor-default"
                                                        : isDownloading
                                                            ? "bg-primary/5 border-primary/20 animate-pulse"
                                                            : "bg-card border-border/50 hover:bg-accent hover:border-border",
                                                    isVariantActive && "border-primary/50 bg-primary/5"
                                                )}
                                            >
                                                <div className="space-y-1">
                                                    <div className="font-semibold flex items-center gap-2">
                                                        {v.name}
                                                        {isLocal && <span className="text-[9px] uppercase tracking-wider font-bold bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 px-1.5 py-0.5 rounded border border-emerald-500/10 ml-2">Installed</span>}
                                                    </div>
                                                    <div className="text-[10px] text-muted-foreground uppercase font-mono">{v.filename}</div>
                                                    <div className="flex gap-2 text-[10px] font-medium text-muted-foreground">
                                                        <span>{v.vram_required_gb}GB VRAM</span>
                                                        <span>•</span>
                                                        <span>{v.size}</span>
                                                    </div>
                                                    <div className="flex flex-wrap gap-1.5 mt-1.5">
                                                        <span className={cn(
                                                            "text-[10px] px-2 py-0.5 rounded-full uppercase font-bold tracking-wider border",
                                                            category === "Cloud" ? "bg-indigo-500/10 text-indigo-500 border-indigo-500/20" :
                                                                category === "Diffusion" ? "bg-pink-500/10 text-pink-500 border-pink-500/20" :
                                                                    category === "STT" ? "bg-amber-500/10 text-amber-500 border-amber-500/20" :
                                                                        category === "Embedding" ? "bg-cyan-500/10 text-cyan-500 border-cyan-500/20" :
                                                                            "bg-primary/10 text-primary border-primary/20"
                                                        )}>
                                                            {category === "Cloud" ? (
                                                                selectedModelVariants.model.id.toLowerCase().startsWith("anthropic") ? "Anthropic" :
                                                                    selectedModelVariants.model.id.toLowerCase().startsWith("openai") ? "OpenAI" :
                                                                        (selectedModelVariants.model.id.toLowerCase().startsWith("google") || selectedModelVariants.model.id.toLowerCase().startsWith("gemini")) ? "Google" :
                                                                            selectedModelVariants.model.id.toLowerCase().startsWith("groq") ? "Groq" :
                                                                                selectedModelVariants.model.id.toLowerCase().startsWith("openrouter") ? "OpenRouter" : "Cloud"
                                                            ) : category}
                                                        </span>
                                                        {selectedModelVariants.model.tags?.map((tag: string) => (
                                                            <span key={tag} className="text-[10px] bg-muted text-muted-foreground px-2 py-0.5 rounded-full border border-border/50">
                                                                {tag}
                                                            </span>
                                                        ))}
                                                    </div>
                                                </div>
                                                <div className="flex items-center gap-2">
                                                    {isVariantActive ? (
                                                        <div className="flex items-center gap-1.5 px-2.5 py-1 bg-primary/10 text-primary rounded-full text-[10px] font-bold uppercase tracking-wider border border-primary/20">
                                                            <CheckCircle2 className="w-3.5 h-3.5" />
                                                            Active
                                                        </div>
                                                    ) : (
                                                        !isLocal && !isDownloading && (
                                                            <Download className="w-4 h-4 text-primary opacity-0 group-hover:opacity-100 transition-opacity" />
                                                        )
                                                    )}
                                                </div>
                                            </button>
                                        );
                                    })}
                                </div>

                                <button
                                    onClick={() => setSelectedModelVariants(null)}
                                    className="w-full py-2 text-sm text-muted-foreground hover:text-foreground transition-colors"
                                >
                                    Cancel
                                </button>
                            </div>
                        </div>
                    </div>
                )}

                {/* Close the topTab === "library" conditional */}
            </>}
        </div>
    );
}
