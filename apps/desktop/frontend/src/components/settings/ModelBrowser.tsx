import { Trash2, RefreshCw, Download, Search, CheckCircle2, FolderOpen, Globe, Loader2 } from "lucide-react";
import * as Progress from '@radix-ui/react-progress';
import { cn } from "../../lib/utils";
import { useModelContext } from "../model-context";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands } from "../../lib/bindings";
import { commandClient } from "../../lib/command-client";
import { bridgeErrorMessage } from "../../lib/command-errors";
import { directCommands } from "../../lib/generated/direct-commands";
import { toast } from "sonner";
import { useConfig } from "../../hooks/use-config";
import { useCloudModels, type CloudModelEntry } from "../../hooks/use-cloud-models";
import { HFDiscovery } from "./HFDiscovery";
import { ActiveEngineChip } from "./ActiveEngineChip";
import { EngineSetupBanner } from "./EngineSetupBanner";
import {
    buildModelDeactivationPlan,
    buildModelLibraryCategories,
    getVisionSelectionState,
    isLocalSummarizerCandidate,
    isModelOnDiskInLibrary,
    normalizeModelLibraryCategory,
    shouldIncludeCuratedEntryInMyModels,
    supportsLocalSummarizer,
} from "../../lib/model-library-view";
import { reconcileSummarizerRuntime } from "../../lib/summarizer-runtime";
import { useOllamaModels } from "../../hooks/use-ollama-models";

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
        cancelDownload,
        deactivateModel,
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
        maxContext,
        engineInfo,
        runtimeSnapshot,
    } = useModelContext();

    // The curated model library is GGUF-only — only relevant for llama.cpp engine
    const isLlamaCpp = engineInfo?.id === 'llamacpp';
    const isOllama = engineInfo?.id === 'ollama';
    const {
        models: ollamaModels,
        status: ollamaModelsStatus,
        error: ollamaModelsError,
        refresh: refreshOllamaModels,
    } = useOllamaModels(isOllama);

    // Trigger standard asset check on mount
    useEffect(() => {
        checkStandardAssets();
    }, [checkStandardAssets]);

    const [searchQuery, setSearchQuery] = useState("");
    const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
    const [activeCategory, setActiveCategory] = useState("All");
    const [status, setStatus] = useState<any>(null);
    const [startingSummarizerPath, setStartingSummarizerPath] = useState<string | null>(null);
    const [summarizerRunning, setSummarizerRunning] = useState(false);
    const summarizerStartInFlightRef = useRef(false);
    const summarizerStatusGenerationRef = useRef(0);
    const { config, updateConfig } = useConfig();
    const summarizerSupported = useMemo(
        () => supportsLocalSummarizer(engineInfo, runtimeSnapshot),
        [engineInfo, runtimeSnapshot],
    );

    const refreshSummarizerStatus = useCallback(async (): Promise<boolean> => {
        const generation = ++summarizerStatusGenerationRef.current;
        if (!summarizerSupported) {
            setSummarizerRunning(false);
            return false;
        }

        try {
            const sidecarStatus = await directCommands.directRuntimeGetSidecarStatus();
            if (generation === summarizerStatusGenerationRef.current) {
                setSummarizerRunning(sidecarStatus.summarizer_running);
            }
            return sidecarStatus.summarizer_running;
        } catch (error) {
            if (generation === summarizerStatusGenerationRef.current) {
                setSummarizerRunning(false);
            }
            console.warn("Could not read summarizer status:", error);
            return false;
        }
    }, [summarizerSupported]);

    useEffect(() => {
        void refreshSummarizerStatus();
        return () => {
            summarizerStatusGenerationRef.current += 1;
        };
    }, [refreshSummarizerStatus, runtimeSnapshot]);

    const activateSummarizer = async (model: {
        name: string;
        localPath?: string | null;
        isLocal?: boolean | null;
        managedCategory?: string | null;
        compatible?: boolean | null;
    }) => {
        if (
            !isLocalSummarizerCandidate(model, summarizerSupported)
            || !model.localPath
        ) {
            toast.error("This model cannot run as a local summarizer");
            return;
        }
        if (summarizerStartInFlightRef.current) return;

        summarizerStartInFlightRef.current = true;
        summarizerStatusGenerationRef.current += 1;
        setStartingSummarizerPath(model.localPath);
        try {
            await reconcileSummarizerRuntime({
                modelPath: model.localPath,
                contextSize: maxContext,
                start: directCommands.directRuntimeStartSummarizerServer,
                persistSelection: setSummarizerModelPath,
            });
            setSummarizerRunning(true);
            toast.success(`${model.name} is ready as the summarizer`);
        } catch (error) {
            console.error("Failed to start summarizer:", error);
            toast.error(`Could not start the summarizer: ${bridgeErrorMessage(error)}`);
            await refreshSummarizerStatus();
        } finally {
            summarizerStartInFlightRef.current = false;
            setStartingSummarizerPath(null);
        }
    };

    const selectOllamaModel = async (model: string) => {
        try {
            if (config?.selected_chat_provider !== "local") {
                await updateConfig({
                    ...config,
                    selected_chat_provider: "local",
                    chat_backend: "local",
                });
            }
            setModelPath(model);
        } catch (error) {
            console.error("Failed to select Ollama model:", error);
            toast.error("Could not switch to the Ollama model");
        }
    };

    // Top-level tab: Discover (HF Hub, default) vs My Models (downloaded)
    const [topTab, setTopTab] = useState<"discover" | "library">("discover");

    // Cloud model discovery
    const { models: cloudDiscovered, loading: cloudLoading, refreshAll: directInferenceRefreshCloudModels, totalModels: cloudTotal, providers: cloudProviders, error: cloudError } = useCloudModels();
    // Suppress unused-var warnings for values used in JSX below
    void cloudTotal;

    useEffect(() => {
        const load = async () => {
            try {
                const s = await commands.thinclawGetStatus();
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
        const implicitProviders = ["mistral", "xai", "together", "venice", "cohere", "moonshot", "minimax", "nvidia"];
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
        (status as any)?.has_nvidia_key || (status as any)?.hasNvidiaKey
    );

    const unifiedModels = useMemo(() => {
        // This tab is inventory-backed. The curated local catalog used mutable
        // file URLs and basename matching, so acquisition now lives exclusively
        // in Discover while only cloud catalog entries are merged here.
        const merged = models.filter(shouldIncludeCuratedEntryInMyModels);

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

        const cloudDisplay = allMerged.map(m => {
            const activeVariant = m.variants[0] || { filename: "" };
            return {
                ...m,
                localPath: null,
                isLocal: false,
                isCurated: true,
                displaySize: activeVariant.size || "Cloud",
                filename: activeVariant.filename,
                relativeFilename: activeVariant.filename,
                componentsStatus: [],
                mmprojStatus: null,
            };
        });

        const localDisplay = localModels.map(l => {
            const metadata = {
                LLM: {
                    tags: ["Local", "Chat"],
                    family: "Local LLM",
                    description: "Local Chat/LLM Model",
                },
                Embedding: {
                    tags: ["Local", "Embedding"],
                    family: "Embedding",
                    description: "Local Embedding Model",
                },
                STT: {
                    tags: ["Local", "STT"],
                    family: "Speech-to-Text",
                    description: "Local Speech-to-Text Model",
                },
                Diffusion: {
                    tags: ["Local", "Image Gen", "Diffusion"],
                    family: "Image Generation",
                    description: "Local Diffusion/Image Model",
                },
                TTS: {
                    tags: ["Local", "TTS"],
                    family: "Text-to-Speech",
                    description: "Local Text-to-Speech Model",
                },
            }[l.category] ?? {
                tags: ["Local"],
                family: "Local Model",
                description: "Local Model",
            };
            const tags = [...metadata.tags];
            if (l.task === "vision") tags.push("Vision", "Multi-modal");
            if (!l.compatible) tags.push("Incompatible");
            const description = l.compatibility_reason
                ? `${metadata.description} · ${l.compatibility_reason}`
                : metadata.description;

            return {
                name: l.repo_id || l.relative_path.split(/[\\/]/).pop() || l.name,
                description,
                filename: l.relative_path,
                url: "",
                size: l.size.toString(),
                displaySize: (l.size / 1024 / 1024 / 1024).toFixed(2) + " GB",
                localPath: l.path,
                isLocal: true,
                isCurated: false,
                id: l.id,
                family: metadata.family,
                vram_required_gb: 0,
                recommended_min_ram: 0,
                tags,
                manual_download: false,
                info_url: undefined,
                relativeFilename: l.install_root,
                category: l.category,
                managedCategory: l.category,
                compatible: l.compatible,
                repoId: l.repo_id,
                artifactId: l.artifact_id,
                companionArtifactId: l.companion_artifact_id,
                managedTask: l.task,
            };
        });

        const allModels = [...cloudDisplay, ...localDisplay].filter(m => {
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
    const isSttActive = (path: string | null) => path && currentSttModelPath && path === currentSttModelPath;
    const isImageGenActive = (path: string | null) => path && currentImageGenModelPath && path === currentImageGenModelPath;
    const isSummarizerSelected = (path: string | null) => path && currentSummarizerModelPath && path === currentSummarizerModelPath;
    const modelCategories = useMemo(
        () => buildModelLibraryCategories({
            hasAnyCloud,
            isLlamaCpp,
            summarizerSupported,
            supportedCapabilities: runtimeSnapshot?.supportedCapabilities ?? [],
        }),
        [
            hasAnyCloud,
            isLlamaCpp,
            runtimeSnapshot?.supportedCapabilities,
            summarizerSupported,
        ],
    );

    useEffect(() => {
        setActiveCategory(current =>
            normalizeModelLibraryCategory(current, modelCategories)
        );
    }, [modelCategories]);

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
                            ? "bg-background text-foreground shadow-xs"
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
                            ? "bg-background text-foreground shadow-xs"
                            : "text-muted-foreground hover:text-foreground"
                    )}
                    id="tab-library"
                >
                    My Models
                    {(isOllama ? ollamaModels.length : localModels.length) > 0 && (
                        <span className="text-[10px] bg-muted/80 text-muted-foreground px-1.5 py-0.5 rounded-full font-mono">
                            {isOllama ? ollamaModels.length : localModels.length}
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
                {isOllama && (
                    <div className="rounded-xl border border-border/60 bg-card/50 p-5">
                        <div className="flex items-start justify-between gap-4">
                            <div>
                                <h3 className="font-semibold">Installed in Ollama</h3>
                                <p className="mt-1 text-xs text-muted-foreground">
                                    Ollama owns these models. Install or remove them with Ollama, then refresh this list.
                                </p>
                            </div>
                            <button
                                type="button"
                                onClick={() => void refreshOllamaModels()}
                                disabled={ollamaModelsStatus === "loading"}
                                className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs font-semibold hover:bg-accent disabled:opacity-50"
                            >
                                <RefreshCw className={cn(
                                    "h-3.5 w-3.5",
                                    ollamaModelsStatus === "loading" && "animate-spin",
                                )} />
                                Refresh
                            </button>
                        </div>

                        {ollamaModelsStatus === "loading" ? (
                            <div className="mt-4 flex items-center text-sm text-muted-foreground">
                                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                Reading the Ollama library…
                            </div>
                        ) : ollamaModelsError ? (
                            <div className="mt-4 rounded-lg border border-destructive/20 bg-destructive/5 p-3 text-sm text-destructive">
                                {ollamaModelsError}
                            </div>
                        ) : ollamaModels.length === 0 ? (
                            <div className="mt-4 rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 text-sm">
                                <p className="font-semibold text-amber-600 dark:text-amber-400">No Ollama models installed</p>
                                <p className="mt-1 text-muted-foreground">
                                    Run <code className="font-mono">ollama pull &lt;model&gt;</code>, then refresh.
                                </p>
                            </div>
                        ) : (
                            <div className="mt-4 grid gap-2 sm:grid-cols-2">
                                {ollamaModels.map(model => {
                                    const active = currentModelPath === model;
                                    return (
                                        <button
                                            key={model}
                                            type="button"
                                            onClick={() => void selectOllamaModel(model)}
                                            className={cn(
                                                "flex items-center justify-between gap-3 rounded-lg border px-3 py-2 text-left text-sm transition-colors",
                                                active
                                                    ? "border-primary/40 bg-primary/10 text-primary"
                                                    : "border-border hover:bg-accent",
                                            )}
                                        >
                                            <span className="truncate font-mono">{model}</span>
                                            {active && <CheckCircle2 className="h-4 w-4 shrink-0" />}
                                        </button>
                                    );
                                })}
                            </div>
                        )}
                    </div>
                )}
                {/* Sticky Header Container */}
                <div className="sticky top-0 z-10 bg-background/95 backdrop-blur-sm supports-backdrop-filter:bg-background/60 -mx-1 px-1 py-4 space-y-4">
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
                                onClick={() => directInferenceRefreshCloudModels()}
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
                                className="w-full pl-10 pr-4 py-2.5 text-sm bg-muted/50 border-none rounded-xl focus:outline-hidden focus:ring-1 focus:ring-primary/20 transition-all"
                            />
                        </div>
                    </div>

                    <div className="flex gap-2 pb-1 overflow-x-auto w-full min-w-0 no-scrollbar mask-fade-right scroll-smooth snap-x">
                        {modelCategories.map((cat) => (
                            <button
                                key={cat}
                                onClick={() => {
                                    setActiveCategory(cat);
                                    setSearchQuery("");
                                }}
                                className={cn(
                                    "px-4 py-1.5 rounded-full text-xs font-medium transition-all whitespace-nowrap border shrink-0 snap-start",
                                    activeCategory === cat
                                        ? "bg-foreground text-background border-foreground shadow-xs"
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
                                    className="bg-background border border-border/50 hover:bg-accent hover:border-border text-foreground px-3 py-1.5 rounded-xl transition-all text-xs font-medium flex items-center shrink-0 ml-4 shadow-xs"
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
                                                    className="w-full border border-primary/30 hover:bg-primary hover:text-primary-foreground text-primary py-2.5 px-4 rounded-xl text-sm font-bold uppercase tracking-wider flex items-center justify-center transition-all shadow-xs hover:-translate-y-px"
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
                        const managedCategory = (m as any).managedCategory as string | undefined;

                        if (activeCategory === "All") return true;
                        if (activeCategory === "Cloud Brains") return isCloud;

                        // Exclude Cloud models from all other specific (Local) tabs
                        if (isCloud) return false;

                        if (activeCategory === "Summarizer") {
                            return isLocalSummarizerCandidate(
                                m,
                                summarizerSupported,
                            );
                        }

                        if (managedCategory) {
                            if (activeCategory === "Chat") {
                                return managedCategory === "LLM";
                            }
                            return managedCategory === activeCategory;
                        }

                        if (activeCategory === "Chat") {
                            // Include local LLMs
                            return !m.tags?.some(t => ["Image Gen", "STT", "Embedding"].includes(t));
                        }
                        if (activeCategory === "Diffusion") return m.tags?.includes("Image Gen");
                        if (activeCategory === "STT") return m.tags?.includes("STT");
                        if (activeCategory === "Embedding") return m.tags?.includes("Embedding");
                        if (activeCategory === "TTS") return m.tags?.includes("TTS");
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
                        const visionSelection = getVisionSelectionState(
                            model.localPath,
                            currentModelPath,
                            currentVisionModelPath,
                        );
                        const isVisionSelected = visionSelection.selected;
                        const isVision = visionSelection.operational;
                        const isStt = isSttActive(model.localPath);
                        const isImageGen = isImageGenActive(model.localPath);
                        const isDownloaded = isModelOnDiskInLibrary(model);
                        const modelAny = model as any;
                        const isCompatible = modelAny.compatible !== false;
                        const canUseAsSummarizer = isLocalSummarizerCandidate(
                            modelAny,
                            summarizerSupported,
                        );
                        const isSummarizerSelectedForModel = Boolean(
                            isSummarizerSelected(model.localPath),
                        );
                        const isSummarizer = canUseAsSummarizer
                            && isSummarizerSelectedForModel
                            && summarizerRunning;
                        const isSummarizerStarting =
                            startingSummarizerPath === model.localPath;
                        const rFilename = modelAny.relativeFilename || model.filename;
                        const isConfirming = confirmingDelete === rFilename;
                        const hasEmbeddingTag = model.tags && model.tags.includes("Embedding");
                        const hasVisionTag = model.tags && (model.tags.includes("Vision") || model.tags.includes("Multi-modal"));
                        const hasSttTag = model.tags && (model.tags.includes("STT") || model.family === "Whisper");
                        const hasImageGenTag = model.tags && (model.tags.includes("Image Gen") || model.family === "Stable Diffusion");
                        const hasTtsTag = model.tags && model.tags.includes("TTS");
                        const deactivationPlan = buildModelDeactivationPlan({
                            chat: Boolean(isModelActive),
                            embedding: Boolean(isEmbedding),
                            vision: Boolean(isVisionSelected),
                            summarizer: isSummarizerSelectedForModel,
                            stt: Boolean(isStt),
                            image: Boolean(isImageGen),
                        }, engineInfo);

                        return (
                            <div key={model.id} className={cn(
                                "flex flex-col p-5 border rounded-2xl transition-all duration-300",
                                isModelActive
                                    ? "bg-accent/40 border-primary/20 shadow-inner"
                                    : "bg-card/40 border-border/50 hover:border-border hover:bg-card/60 shadow-xs"
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
                                                {!isCompatible && <span className="text-[10px] uppercase tracking-wider font-bold bg-amber-500/10 text-amber-600 dark:text-amber-400 px-2 py-0.5 rounded-md border border-amber-500/20">Incompatible</span>}
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
                                                        ["nvidia", "NVIDIA"],
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
                                                        onClick={() => setTopTab("discover")}
                                                        className="text-primary hover:text-primary/80 transition-colors font-semibold"
                                                    >
                                                        Reinstall from Discover
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

                                        {deactivationPlan.hasSelection && (
                                            <button
                                                onClick={() => void deactivateModel(rFilename)}
                                                className="py-2 px-3 rounded-md text-xs font-semibold text-destructive border border-destructive/20 hover:bg-destructive hover:text-destructive-foreground transition-colors"
                                            >
                                                Deactivate
                                            </button>
                                        )}

                                        <div className="flex flex-wrap gap-2 flex-1">
                                            {!hasEmbeddingTag && !hasSttTag && !hasImageGenTag && !hasTtsTag && (
                                                <>
                                                    <button
                                                        onClick={async () => {
                                                            if (model.localPath) {
                                                                if (config?.selected_chat_provider !== "local") {
                                                                    try {
                                                                        const newConfig = {
                                                                            ...config,
                                                                            selected_chat_provider: "local",
                                                                            chat_backend: "local",
                                                                        };
                                                                        await updateConfig(newConfig);
                                                                    } catch (e) {
                                                                        console.error(e);
                                                                        toast.error("Could not switch to local inference");
                                                                        return;
                                                                    }
                                                                }
                                                                if (hasVisionTag) {
                                                                    setVisionModelPath(model.localPath);
                                                                }
                                                                setModelPath(model.localPath, (model as any).template);
                                                            }
                                                        }}
                                                        className={cn(
                                                            "flex-1 py-2 px-3 rounded-xl text-xs font-bold uppercase tracking-wider transition-all",
                                                            isModelActive
                                                                ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20 shadow-xs ring-1 ring-emerald-500/10"
                                                                : "bg-secondary hover:bg-secondary/80 text-secondary-foreground border border-transparent shadow-xs hover:-translate-y-px"
                                                        )}
                                                        disabled={!isCompatible || !!isModelActive}
                                                    >
                                                        {isModelActive ? "Active" : "Chat"}
                                                    </button>
                                                    {canUseAsSummarizer && (
                                                        <button
                                                            onClick={() => void activateSummarizer(modelAny)}
                                                            className={cn(
                                                                "flex-1 py-1.5 px-3 rounded-xl text-xs font-medium flex items-center justify-center border transition-all",
                                                                isSummarizer
                                                                    ? "bg-muted text-muted-foreground border-border/50 cursor-default"
                                                                    : "border-input hover:bg-accent hover:text-accent-foreground shadow-xs"
                                                            )}
                                                            disabled={
                                                                isSummarizer
                                                                || startingSummarizerPath !== null
                                                            }
                                                        >
                                                            {isSummarizerStarting ? (
                                                                <>
                                                                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                                                                    Starting…
                                                                </>
                                                            ) : isSummarizer ? (
                                                                "Summ. Active"
                                                            ) : isSummarizerSelectedForModel ? (
                                                                "Restore Summ."
                                                            ) : (
                                                                "Set Summ."
                                                            )}
                                                        </button>
                                                    )}
                                                </>
                                            )}

                                            {hasVisionTag && (
                                                <button
                                                    onClick={async () => {
                                                        if (!model.localPath) return;
                                                        if (config?.selected_chat_provider !== "local") {
                                                            try {
                                                                await updateConfig({
                                                                    ...config,
                                                                    selected_chat_provider: "local",
                                                                    chat_backend: "local",
                                                                });
                                                            } catch (error) {
                                                                console.error(error);
                                                                toast.error("Could not switch to local inference");
                                                                return;
                                                            }
                                                        }
                                                        setVisionModelPath(model.localPath);
                                                        setModelPath(
                                                            model.localPath,
                                                            (model as any).template,
                                                        );
                                                    }}
                                                    className={cn(
                                                        "flex-1 py-1.5 px-3 rounded-xl text-xs font-medium flex items-center justify-center border transition-all",
                                                        isVision
                                                            ? "bg-muted text-muted-foreground border-border/50 cursor-default"
                                                            : "border-input hover:bg-accent hover:text-accent-foreground shadow-xs"
                                                    )}
                                                    disabled={!isCompatible || !!isVision}
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
                                                            : "border-input hover:bg-accent hover:text-accent-foreground shadow-xs"
                                                    )}
                                                    disabled={!isCompatible || !!isStt}
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
                                                            : "border-input hover:bg-accent hover:text-accent-foreground shadow-xs"
                                                    )}
                                                    disabled={!isCompatible || !!isImageGen}
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
                                                            : "border-input hover:bg-accent hover:text-accent-foreground shadow-xs"
                                                    )}
                                                    disabled={!isCompatible || !!isEmbedding}
                                                >
                                                    {isEmbedding ? "Embedder Active" : "Set Embedder"}
                                                </button>
                                            )}

                                            {hasTtsTag && (
                                                <span className="flex-1 py-2 px-3 text-center text-xs text-muted-foreground border border-border/50 rounded-xl">
                                                    TTS assets are configured by the speech runtime
                                                </span>
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
                                                        ["nvidia-", "nvidia"],
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
                                                    await commandClient.thinclawSaveSelectedCloudModel(modelId);
                                                    await updateConfig(newConfig);
                                                    const providerName = brain === "gemini" ? "Google" : brain.charAt(0).toUpperCase() + brain.slice(1);
                                                    toast.success(`${model.name} selected as active ${providerName} Brain`);
                                                    const s = await commands.thinclawGetStatus();
                                                    if (s.status === 'ok') setStatus(s.data);
                                                } catch (e) {
                                                    toast.error("Failed to select cloud model");
                                                }
                                            }}
                                            className={cn(
                                                "flex-1 py-2 px-3 rounded-xl text-xs font-bold uppercase tracking-wider transition-all",
                                                isModelActive
                                                    ? "bg-indigo-500/10 text-indigo-600 dark:text-indigo-400 border border-indigo-500/20 shadow-xs ring-1 ring-indigo-500/10"
                                                    : "bg-secondary hover:bg-secondary/80 text-secondary-foreground border border-transparent shadow-xs hover:-translate-y-px"
                                            )}
                                            disabled={!!isModelActive}
                                        >
                                            {isModelActive ? "Active" : "Select Brain"}
                                        </button>
                                    </div>
                                ) : (
                                    <div className="flex gap-2">
                                        <button
                                            onClick={() => setTopTab("discover")}
                                            className="w-full border border-primary/30 hover:bg-primary hover:text-primary-foreground text-primary py-2.5 px-4 rounded-xl text-sm font-bold uppercase tracking-wider flex items-center justify-center transition-all shadow-xs hover:-translate-y-px"
                                        >
                                            <Download className="w-4 h-4 mr-2" />
                                            Browse in Discover
                                        </button>
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

                {/* Close the topTab === "library" conditional */}
            </>}
        </div>
    );
}
