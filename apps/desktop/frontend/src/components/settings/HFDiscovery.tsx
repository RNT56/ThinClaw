/**
 * HuggingFace Hub Model Discovery Component
 *
 * Live search of HuggingFace models, filtered by the build's inference engine.
 * Shows model cards with downloads/likes, and for GGUF models provides a
 * quantization picker. For MLX/vLLM, shows total directory size.
 *
 * All persistent state (search results, downloads, progress) lives in
 * ModelContext.discoveryState so it survives tab switches.
 *
 * Features:
 * - Downloading models pinned to top of results
 * - Auto-expand of downloading model card on remount
 * - Per-file progress bars for multi-file downloads
 * - "Downloaded" badge for models already on disk
 */
import {
    Search,
    Download,
    Heart,
    ArrowDownToLine,
    Loader2,
    Shield,
    ExternalLink,
    ChevronDown,
    Info,
    CheckCircle2,
    Pin,
    Type,
    Eye,
    Layers,
    Database,
    Image,
    Mic,
    Video,
} from "lucide-react";
import { cn } from "../../lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { useModelContext } from "../model-context";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

// ---------------------------------------------------------------------------
// Types (match backend via specta)
// ---------------------------------------------------------------------------

interface EngineInfo {
    id: string;
    display_name: string;
    available: boolean;
    requires_setup: boolean;
    description: string;
    hf_tag: string;
    single_file_model: boolean;
}

interface HfModelCard {
    id: string;
    author: string;
    name: string;
    downloads: number;
    likes: number;
    tags: string[];
    last_modified: string;
    gated: boolean;
}

interface HfFileInfo {
    filename: string;
    size: number;
    size_display: string;
    quant_type: string | null;
    is_mmproj: boolean;
}

interface ModelDownloadInfo {
    repo_id: string;
    is_multi_file: boolean;
    files: HfFileInfo[];
    mmproj_file: HfFileInfo | null;
    total_size: number;
    total_size_display: string;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatDownloads(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return n.toString();
}

/** Sanitize a HF repo ID to the directory name used on disk */
function sanitizeRepoId(repoId: string): string {
    return repoId.replace(/\//g, "_");
}

// Pipeline filter definitions — maps UI filter to HF pipeline_tag(s)
type PipelineFilterId = 'all' | 'text' | 'vision' | 'embedding' | 'diffusion' | 'stt' | 'video';

interface PipelineFilterDef {
    id: PipelineFilterId;
    label: string;
    icon: typeof Layers;
    tags: string[];
    placeholder: string;
    /** Download category folder name — null means default (LLM) */
    downloadCategory: string | null;
    /** Default search query per engine — uses this for trending when empty. Key = engine id, '*' = all engines. */
    defaultQuery?: Record<string, string>;
}

const PIPELINE_FILTERS: PipelineFilterDef[] = [
    { id: 'all', label: 'All LLMs', icon: Layers, tags: ['text-generation', 'image-text-to-text'], placeholder: 'Search LLMs... (e.g. llama, qwen, gemma)', downloadCategory: null },
    { id: 'text', label: 'Text', icon: Type, tags: ['text-generation'], placeholder: 'Search text models... (e.g. llama, qwen, ministral)', downloadCategory: null },
    { id: 'vision', label: 'Vision', icon: Eye, tags: ['image-text-to-text'], placeholder: 'Search vision models... (e.g. pixtral, llava, gemma)', downloadCategory: null },
    { id: 'embedding', label: 'Embedding', icon: Database, tags: ['feature-extraction', 'sentence-similarity'], placeholder: 'Search embedding models... (e.g. bge, nomic, gte, qwen)', downloadCategory: 'Embedding' },
    { id: 'diffusion', label: 'Diffusion', icon: Image, tags: ['text-to-image', 'image-to-image'], placeholder: 'Search diffusion models... (e.g. flux, stable-diffusion, sdxl)', downloadCategory: 'Diffusion' },
    {
        id: 'stt',
        label: 'STT',
        icon: Mic,
        tags: ['automatic-speech-recognition'],
        placeholder: 'Search speech models... (e.g. mlx-community/whisper-large-v3-turbo)',
        downloadCategory: 'STT',
        // For MLX engine: default to mlx-community whisper collection so users see compatible models immediately
        defaultQuery: { mlx: 'mlx-community whisper', '*': '' },
    },
    { id: 'video', label: 'Video', icon: Video, tags: ['text-to-video'], placeholder: 'Search video gen models... (e.g. mochi, ltx-video)', downloadCategory: 'Diffusion' },
];

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function HFDiscovery({ isVisible = true }: { isVisible?: boolean }) {
    const {
        downloading,
        downloadHfFiles,
        engineInfo: contextEngineInfo,
        discoveryState,
        setDiscoveryState,
        localModels,
    } = useModelContext();

    // Engine info — prefer context, fallback to direct invoke
    const [localEngineInfo, setLocalEngineInfo] = useState<EngineInfo | null>(null);
    const engineInfo = contextEngineInfo ?? localEngineInfo;

    // Use persistent state from context
    const { searchQuery, results, hasSearched, expandedModel, downloadingFiles, repoProgress } =
        discoveryState;

    // Pipeline type filter
    const [pipelineFilter, setPipelineFilter] = useState<PipelineFilterId>('all');
    const activeFilterDef = PIPELINE_FILTERS.find(f => f.id === pipelineFilter) ?? PIPELINE_FILTERS[0];

    // Local-only ephemeral state
    const [isSearching, setIsSearching] = useState(false);
    const [debouncedQuery, setDebouncedQuery] = useState(searchQuery);
    const [fileInfoCache, setFileInfoCache] = useState<Record<string, ModelDownloadInfo>>({});

    const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

    // -----------------------------------------------------------------------
    // Derived: which models are already downloaded locally?
    // -----------------------------------------------------------------------
    const downloadedRepoIds = useMemo(() => {
        const ids = new Set<string>();
        for (const m of localModels) {
            // m.path looks like ".../models/LLM/google_gemma-3-4b-it"
            // Extract the last path segment and compare to sanitized repo IDs
            const segments = m.path.replace(/\\/g, "/").split("/");
            const dirName = segments[segments.length - 1] || segments[segments.length - 2];
            if (dirName) ids.add(dirName);
        }
        return ids;
    }, [localModels]);

    const isModelDownloaded = useCallback(
        (repoId: string) => downloadedRepoIds.has(sanitizeRepoId(repoId)),
        [downloadedRepoIds]
    );

    // -----------------------------------------------------------------------
    // Derived: sort results with downloading pinned to top, then downloaded next
    // -----------------------------------------------------------------------
    const sortedResults = useMemo(() => {
        if (results.length === 0) return results;

        return [...results].sort((a, b) => {
            const aDown = downloadingFiles.has(a.id) ? 2 : 0;
            const bDown = downloadingFiles.has(b.id) ? 2 : 0;
            const aLocal = isModelDownloaded(a.id) ? 1 : 0;
            const bLocal = isModelDownloaded(b.id) ? 1 : 0;
            // Higher priority first
            return (bDown + bLocal) - (aDown + aLocal);
        });
    }, [results, downloadingFiles, isModelDownloaded]);

    // -----------------------------------------------------------------------
    // Setters that write through to context
    // -----------------------------------------------------------------------
    const setSearchQuery = useCallback(
        (q: string) => {
            setDiscoveryState((prev) => ({ ...prev, searchQuery: q }));
        },
        [setDiscoveryState]
    );

    const setResults = useCallback(
        (r: HfModelCard[]) => {
            setDiscoveryState((prev) => ({ ...prev, results: r }));
        },
        [setDiscoveryState]
    );

    const setHasSearched = useCallback(
        (v: boolean) => {
            setDiscoveryState((prev) => ({ ...prev, hasSearched: v }));
        },
        [setDiscoveryState]
    );

    const setExpandedModel = useCallback(
        (id: string | null) => {
            setDiscoveryState((prev) => ({ ...prev, expandedModel: id }));
        },
        [setDiscoveryState]
    );

    const setDownloadingFiles = useCallback(
        (updater: (prev: Set<string>) => Set<string>) => {
            setDiscoveryState((prev) => ({
                ...prev,
                downloadingFiles: updater(prev.downloadingFiles),
            }));
        },
        [setDiscoveryState]
    );

    // Fallback engine info load if context doesn't have it yet
    useEffect(() => {
        if (!contextEngineInfo) {
            invoke<EngineInfo>("get_active_engine_info")
                .then(setLocalEngineInfo)
                .catch((err) => console.error("Failed to get engine info:", err));
        }
    }, [contextEngineInfo]);

    // -----------------------------------------------------------------------
    // File info loading (with cache) — declared before the auto-expand effect
    // -----------------------------------------------------------------------
    const loadFileInfoDirect = useCallback(
        async (repoId: string) => {
            if (!engineInfo) return;
            try {
                const info = await invoke<ModelDownloadInfo>("get_model_files", {
                    repoId,
                    engine: engineInfo.id,
                });
                setFileInfoCache((prev) => ({ ...prev, [repoId]: info }));
            } catch (err: any) {
                console.error("Failed to load file tree:", err);
                toast.error("Failed to load model files");
            }
        },
        [engineInfo]
    );

    // -----------------------------------------------------------------------
    // On becoming visible: auto-expand first downloading model & restore info
    // -----------------------------------------------------------------------
    useEffect(() => {
        if (!isVisible || !engineInfo) return;

        // If there's an actively downloading model, pin + expand it
        if (downloadingFiles.size > 0) {
            const firstDownloading = [...downloadingFiles].find((id) =>
                results.some((r) => r.id === id)
            );
            if (firstDownloading) {
                // Expand the card if not already expanded
                if (expandedModel !== firstDownloading) {
                    setExpandedModel(firstDownloading);
                }
                // Load file info if missing (e.g. cache lost on remount)
                if (!fileInfoCache[firstDownloading]) {
                    loadFileInfoDirect(firstDownloading);
                }
            }
        } else if (expandedModel && !fileInfoCache[expandedModel]) {
            // No active downloads but a card is expanded without file info —
            // reload it (handles returning to an expanded non-downloading card)
            loadFileInfoDirect(expandedModel);
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [isVisible]); // Only trigger when tab visibility changes

    // Debounce search input
    useEffect(() => {
        if (debounceTimer.current) clearTimeout(debounceTimer.current);
        debounceTimer.current = setTimeout(() => {
            setDebouncedQuery(searchQuery);
        }, 350);
        return () => {
            if (debounceTimer.current) clearTimeout(debounceTimer.current);
        };
    }, [searchQuery]);

    // Cache of auto-populated trending results per filter (avoid re-fetching)
    const [trendingCache, setTrendingCache] = useState<Record<PipelineFilterId, HfModelCard[]>>({} as any);
    const [isTrending, setIsTrending] = useState(false);

    // Trigger search when debounced query or pipeline filter changes
    // When query is empty, auto-populate with trending models for the active filter
    useEffect(() => {
        if (!engineInfo) return;

        const pipelineTags: string[] = activeFilterDef.tags;

        // Empty query → auto-populate with trending models
        if (!debouncedQuery.trim()) {
            // Check cache first
            if (trendingCache[pipelineFilter] && trendingCache[pipelineFilter].length > 0) {
                setResults(trendingCache[pipelineFilter]);
                setHasSearched(true);
                setIsTrending(true);
                return;
            }

            const fetchTrending = async () => {
                setIsSearching(true);
                setIsTrending(true);
                try {
                    // Use engine-specific default query if defined (e.g. 'mlx-community whisper' for MLX+STT)
                    const defaultQueryMap = activeFilterDef.defaultQuery || {};
                    const defaultQ = defaultQueryMap[engineInfo.id] ?? defaultQueryMap['*'] ?? '';

                    const models = await invoke<HfModelCard[]>("discover_hf_models", {
                        query: defaultQ,
                        engine: engineInfo.id,
                        limit: 15,
                        pipelineTags,
                    });
                    setResults(models);
                    setHasSearched(true);
                    // Cache the results for this filter
                    setTrendingCache(prev => ({ ...prev, [pipelineFilter]: models }));
                } catch (err: any) {
                    console.error("HF trending fetch failed:", err);
                    // Don't toast for initial load failures — not critical
                    setResults([]);
                    setHasSearched(false);
                } finally {
                    setIsSearching(false);
                }
            };

            fetchTrending();
            return;
        }

        // Non-empty query → regular search
        setIsTrending(false);

        const doSearch = async () => {
            setIsSearching(true);
            setHasSearched(true);
            try {
                const models = await invoke<HfModelCard[]>("discover_hf_models", {
                    query: debouncedQuery,
                    engine: engineInfo.id,
                    limit: 20,
                    pipelineTags,
                });
                setResults(models);
            } catch (err: any) {
                console.error("HF search failed:", err);
                toast.error(typeof err === "string" ? err : "HuggingFace search failed");
                setResults([]);
            } finally {
                setIsSearching(false);
            }
        };

        doSearch();
    }, [debouncedQuery, engineInfo, pipelineFilter]);

    // Toggle expand
    const handleExpand = useCallback(
        (repoId: string) => {
            if (expandedModel === repoId) {
                setExpandedModel(null);
            } else {
                setExpandedModel(repoId);
                if (!fileInfoCache[repoId]) {
                    loadFileInfoDirect(repoId);
                }
            }
        },
        [expandedModel, loadFileInfoDirect, fileInfoCache]
    );

    // Download a single GGUF file (+ optional mmproj)
    const handleDownloadSingle = useCallback(
        async (repoId: string, file: HfFileInfo, mmproj?: HfFileInfo | null) => {
            const files = [file.filename];
            if (mmproj) files.push(mmproj.filename);

            setDownloadingFiles((prev) => new Set([...prev, file.filename]));

            try {
                await downloadHfFiles(repoId, files, null, activeFilterDef.downloadCategory ?? undefined);
            } finally {
                setDownloadingFiles((prev) => {
                    const next = new Set(prev);
                    next.delete(file.filename);
                    return next;
                });
            }
        },
        [downloadHfFiles, activeFilterDef]
    );

    // Download all files (MLX/vLLM directory) — track by repoId
    const handleDownloadAll = useCallback(
        async (repoId: string, files: HfFileInfo[]) => {
            const filenames = files.map((f) => f.filename);
            setDownloadingFiles((prev) => new Set([...prev, repoId]));

            try {
                await downloadHfFiles(repoId, filenames, null, activeFilterDef.downloadCategory ?? undefined);
            } finally {
                // repoProgress listener handles cleanup after 100%
            }
        },
        [downloadHfFiles, activeFilterDef]
    );

    // Open external URL
    const openExternal = useCallback((url: string) => {
        invoke("open_url", { url }).catch((err) =>
            console.warn("open_url failed:", err)
        );
    }, []);

    // -----------------------------------------------------------------------
    // Render
    // -----------------------------------------------------------------------
    return (
        <div className="space-y-4">
            {/* Engine Badge + Model Type Filter */}
            <div className="flex items-center justify-between gap-3">
                {engineInfo && (
                    <div className="flex items-center gap-2 text-xs text-muted-foreground bg-muted/30 px-3 py-2 rounded-xl border border-border/30 flex-1 min-w-0">
                        <Info className="w-3.5 h-3.5 text-primary/60 shrink-0" />
                        <span className="truncate">
                            Searching for{" "}
                            <span className="font-semibold text-foreground">
                                {engineInfo.hf_tag.toUpperCase()}
                            </span>{" "}
                            models compatible with{" "}
                            <span className="font-semibold text-foreground">
                                {engineInfo.display_name}
                            </span>
                        </span>
                    </div>
                )}

                {/* Pipeline Type Scrollable Filter Bar */}
                <div className="flex gap-1.5 overflow-x-auto no-scrollbar shrink-0" id="hf-pipeline-filter">
                    {PIPELINE_FILTERS.map(({ id, label, icon: Icon }) => (
                        <button
                            key={id}
                            onClick={() => setPipelineFilter(id)}
                            className={cn(
                                "flex items-center gap-1 px-2.5 py-1.5 rounded-full text-[10px] font-bold uppercase tracking-wider transition-all duration-200 whitespace-nowrap border shrink-0",
                                pipelineFilter === id
                                    ? "bg-foreground text-background border-foreground shadow-sm"
                                    : "bg-muted/40 text-muted-foreground border-transparent hover:bg-muted hover:text-foreground"
                            )}
                            id={`hf-filter-${id}`}
                        >
                            <Icon className="w-3 h-3" />
                            <span>{label}</span>
                        </button>
                    ))}
                </div>
            </div>

            {/* Search Input */}
            <div className="relative">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
                <input
                    type="text"
                    placeholder={activeFilterDef.placeholder}
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    className="w-full pl-10 pr-4 py-2.5 text-sm bg-background border border-border/50 rounded-xl focus:outline-none focus:ring-2 focus:ring-primary/20 transition-all text-foreground placeholder:text-muted-foreground/50"
                    id="hf-search-input"
                />
                {isSearching && (
                    <Loader2 className="absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground animate-spin" />
                )}
            </div>

            {/* Trending header when showing auto-populated results */}
            {isTrending && sortedResults.length > 0 && !isSearching && (
                <div className="flex items-center gap-2 text-xs text-muted-foreground/60 mb-1">
                    <Heart className="w-3 h-3" />
                    <span>Popular {activeFilterDef.label} models</span>
                </div>
            )}

            {/* Results */}
            <div className="grid gap-3">
                {sortedResults.map((model) => {
                    const isExpanded = expandedModel === model.id;
                    const rp = repoProgress[model.id];
                    const isDownloading = downloadingFiles.has(model.id);
                    const isDownloaded = isModelDownloaded(model.id);
                    const fileInfo = fileInfoCache[model.id] ?? null;
                    const isVision = model.tags.some((t) => t === "image-text-to-text");
                    const isEmbeddingModel = model.tags.some((t) => t === "feature-extraction" || t === "sentence-similarity");
                    const isDiffusionModel = model.tags.some((t) => t === "text-to-image" || t === "image-to-image");
                    const isSttModel = model.tags.some((t) => t === "automatic-speech-recognition");
                    const isVideoModel = model.tags.some((t) => t === "text-to-video");
                    const categoryBadge = isEmbeddingModel ? { label: 'Embedding', icon: Database, color: 'cyan' }
                        : isDiffusionModel ? { label: 'Diffusion', icon: Image, color: 'pink' }
                            : isSttModel ? { label: 'STT', icon: Mic, color: 'amber' }
                                : isVideoModel ? { label: 'Video', icon: Video, color: 'rose' }
                                    : null;

                    return (
                        <div
                            key={model.id}
                            className={cn(
                                "border rounded-xl bg-card/40 hover:bg-card/60 transition-all duration-300 overflow-hidden shadow-sm",
                                isDownloading
                                    ? "border-primary/40 ring-1 ring-primary/10 bg-primary/[0.02]"
                                    : isDownloaded
                                        ? "border-green-500/30"
                                        : "border-border/50"
                            )}
                        >
                            {/* Card Header */}
                            <button
                                onClick={() => handleExpand(model.id)}
                                className="w-full text-left p-4 flex items-start justify-between gap-3"
                                id={`hf-model-${model.id.replace("/", "-")}`}
                            >
                                <div className="min-w-0 flex-1">
                                    <h3 className="font-semibold text-sm mb-1 flex items-center gap-2 flex-wrap">
                                        <span className="truncate max-w-[300px]">
                                            {model.name}
                                        </span>

                                        {/* Vision badge */}
                                        {isVision && (
                                            <span className="text-[9px] uppercase tracking-wider font-bold bg-violet-500/10 text-violet-600 dark:text-violet-400 px-1.5 py-0.5 rounded border border-violet-500/20 flex items-center gap-1 shrink-0">
                                                <Eye className="w-2.5 h-2.5" />
                                                Vision
                                            </span>
                                        )}

                                        {/* Category badge (non-LLM models) */}
                                        {categoryBadge && (
                                            <span className={cn(
                                                "text-[9px] uppercase tracking-wider font-bold px-1.5 py-0.5 rounded border flex items-center gap-1 shrink-0",
                                                categoryBadge.color === 'cyan' && 'bg-cyan-500/10 text-cyan-600 dark:text-cyan-400 border-cyan-500/20',
                                                categoryBadge.color === 'pink' && 'bg-pink-500/10 text-pink-600 dark:text-pink-400 border-pink-500/20',
                                                categoryBadge.color === 'amber' && 'bg-amber-500/10 text-amber-600 dark:text-amber-400 border-amber-500/20',
                                                categoryBadge.color === 'rose' && 'bg-rose-500/10 text-rose-600 dark:text-rose-400 border-rose-500/20',
                                            )}>
                                                <categoryBadge.icon className="w-2.5 h-2.5" />
                                                {categoryBadge.label}
                                            </span>
                                        )}

                                        {/* Downloaded badge */}
                                        {isDownloaded && !isDownloading && (
                                            <span className="text-[9px] uppercase tracking-wider font-bold bg-green-500/10 text-green-600 dark:text-green-400 px-1.5 py-0.5 rounded border border-green-500/20 flex items-center gap-1 shrink-0">
                                                <CheckCircle2 className="w-2.5 h-2.5" />
                                                Downloaded
                                            </span>
                                        )}

                                        {/* Gated badge */}
                                        {model.gated && (
                                            <span className="text-[9px] uppercase tracking-wider font-bold bg-amber-500/10 text-amber-600 dark:text-amber-400 px-1.5 py-0.5 rounded border border-amber-500/20 flex items-center gap-1 shrink-0">
                                                <Shield className="w-2.5 h-2.5" />
                                                Gated
                                            </span>
                                        )}

                                        {/* Active download badge (visible when collapsed) */}
                                        {isDownloading && !isExpanded && rp && (
                                            <span className="text-[9px] font-bold bg-primary/10 text-primary px-1.5 py-0.5 rounded border border-primary/20 flex items-center gap-1 shrink-0 animate-pulse">
                                                <Loader2 className="w-2.5 h-2.5 animate-spin" />
                                                {rp.pct.toFixed(0)}%
                                            </span>
                                        )}

                                        {/* Pinned indicator for downloading models */}
                                        {isDownloading && (
                                            <Pin className="w-3 h-3 text-primary/50 shrink-0 rotate-45" />
                                        )}
                                    </h3>
                                    <p className="text-xs text-muted-foreground/70 truncate">
                                        {model.author}
                                    </p>
                                    <div className="flex items-center gap-3 mt-2 text-[11px] text-muted-foreground/60">
                                        <span className="flex items-center gap-1">
                                            <ArrowDownToLine className="w-3 h-3" />
                                            {formatDownloads(model.downloads)}
                                        </span>
                                        <span className="flex items-center gap-1">
                                            <Heart className="w-3 h-3" />
                                            {formatDownloads(model.likes)}
                                        </span>
                                    </div>

                                    {/* Compact progress bar when collapsed & downloading */}
                                    {isDownloading && !isExpanded && rp && (
                                        <div className="relative overflow-hidden bg-secondary rounded-full w-full h-1.5 mt-2">
                                            <div
                                                className="bg-primary h-full rounded-full transition-all duration-300 ease-out"
                                                style={{ width: `${rp.pct || 0}%` }}
                                            />
                                        </div>
                                    )}
                                </div>
                                <ChevronDown
                                    className={cn(
                                        "w-4 h-4 text-muted-foreground/50 transition-transform duration-200 shrink-0 mt-1",
                                        isExpanded && "rotate-180"
                                    )}
                                />
                            </button>

                            {/* Expanded: File Tree / Quant Picker */}
                            {isExpanded && (
                                <div className="border-t border-border/30 p-4 bg-muted/10">
                                    {!fileInfo ? (
                                        <div className="flex items-center justify-center py-6 text-sm text-muted-foreground gap-2">
                                            <Loader2 className="w-4 h-4 animate-spin" />
                                            Loading files...
                                        </div>
                                    ) : (
                                        <div className="space-y-3">
                                            {/* Total size badge */}
                                            <div className="flex items-center justify-between text-xs text-muted-foreground">
                                                <span>
                                                    {fileInfo.files.length} file
                                                    {fileInfo.files.length !== 1 ? "s" : ""}
                                                </span>
                                                {!fileInfo.is_multi_file && (
                                                    <span className="font-mono bg-muted/50 px-2 py-0.5 rounded border border-border/10">
                                                        Total: {fileInfo.total_size_display}
                                                    </span>
                                                )}
                                            </div>

                                            {/* mmproj indicator */}
                                            {fileInfo.mmproj_file && (
                                                <div className="text-[11px] bg-indigo-500/5 text-indigo-500 border border-indigo-500/10 rounded-lg px-3 py-1.5 flex items-center gap-2">
                                                    <Info className="w-3 h-3" />
                                                    Vision projector will be included:{" "}
                                                    <span className="font-mono">
                                                        {fileInfo.mmproj_file.filename}
                                                    </span>
                                                    <span className="font-mono text-muted-foreground">
                                                        ({fileInfo.mmproj_file.size_display})
                                                    </span>
                                                </div>
                                            )}

                                            {/* Already downloaded notice */}
                                            {isDownloaded && !isDownloading && (
                                                <div className="text-[11px] bg-green-500/5 text-green-600 dark:text-green-400 border border-green-500/10 rounded-lg px-3 py-2 flex items-center gap-2">
                                                    <CheckCircle2 className="w-3.5 h-3.5 shrink-0" />
                                                    This model is already downloaded and available in your Library.
                                                </div>
                                            )}

                                            {/* File list */}
                                            {fileInfo.is_multi_file ? (
                                                // MLX/vLLM: Download all button + per-file progress
                                                <div className="space-y-2">
                                                    <div className="max-h-[200px] overflow-y-auto space-y-1 pr-1">
                                                        {fileInfo.files.map((f) => {
                                                            const isCurrentFile =
                                                                rp?.currentFile === f.filename;
                                                            const filePct =
                                                                rp?.filePct?.[f.filename];
                                                            const isFileComplete =
                                                                filePct !== undefined &&
                                                                filePct >= 100;

                                                            return (
                                                                <div
                                                                    key={f.filename}
                                                                    className="py-1 px-2 rounded hover:bg-muted/30"
                                                                >
                                                                    <div className="flex items-center justify-between text-[11px]">
                                                                        <span
                                                                            className={cn(
                                                                                "truncate font-mono max-w-[250px] transition-colors",
                                                                                isCurrentFile
                                                                                    ? "text-primary font-semibold"
                                                                                    : isFileComplete
                                                                                        ? "text-green-500"
                                                                                        : "text-muted-foreground"
                                                                            )}
                                                                        >
                                                                            {f.filename}
                                                                        </span>
                                                                        <div className="flex items-center gap-2 shrink-0 ml-2">
                                                                            {filePct !==
                                                                                undefined &&
                                                                                filePct < 100 && (
                                                                                    <span className="text-[10px] font-mono text-primary/70 tabular-nums">
                                                                                        {filePct.toFixed(
                                                                                            0
                                                                                        )}
                                                                                        %
                                                                                    </span>
                                                                                )}
                                                                            {isFileComplete && (
                                                                                <span className="text-[10px] text-green-500">
                                                                                    ✓
                                                                                </span>
                                                                            )}
                                                                            <span className="font-mono text-muted-foreground/50">
                                                                                {f.size_display}
                                                                            </span>
                                                                        </div>
                                                                    </div>
                                                                    {/* Per-file progress bar */}
                                                                    {filePct !== undefined &&
                                                                        filePct < 100 && (
                                                                            <div className="relative overflow-hidden bg-secondary rounded-full w-full h-1 mt-1">
                                                                                <div
                                                                                    className="bg-primary/60 h-full rounded-full transition-all duration-300 ease-out"
                                                                                    style={{
                                                                                        width: `${filePct || 0}%`,
                                                                                    }}
                                                                                />
                                                                            </div>
                                                                        )}
                                                                </div>
                                                            );
                                                        })}
                                                    </div>

                                                    {/* Overall progress bar for multi-file downloads */}
                                                    {rp && (
                                                        <div className="space-y-1">
                                                            <div className="flex items-center justify-between text-[10px] text-muted-foreground">
                                                                <span className="truncate font-mono max-w-[220px] text-primary/70">
                                                                    {rp.currentFile ||
                                                                        "Preparing..."}
                                                                </span>
                                                                <span className="shrink-0 ml-2 tabular-nums">
                                                                    {rp.fileIndex + 1}/
                                                                    {rp.fileCount} ·{" "}
                                                                    {rp.pct.toFixed(0)}%
                                                                </span>
                                                            </div>
                                                            <div className="relative overflow-hidden bg-secondary rounded-full w-full h-2">
                                                                <div
                                                                    className="bg-primary h-full rounded-full transition-all duration-300 ease-out"
                                                                    style={{
                                                                        width: `${rp.pct || 0}%`,
                                                                    }}
                                                                />
                                                            </div>
                                                        </div>
                                                    )}

                                                    <button
                                                        onClick={() =>
                                                            handleDownloadAll(
                                                                model.id,
                                                                fileInfo.files
                                                            )
                                                        }
                                                        disabled={
                                                            isDownloading || isDownloaded
                                                        }
                                                        className={cn(
                                                            "w-full border border-primary/30 hover:bg-primary hover:text-primary-foreground text-primary py-2.5 px-4 rounded-xl text-sm font-bold uppercase tracking-wider flex items-center justify-center transition-all shadow-sm hover:translate-y-[-1px]",
                                                            (isDownloading || isDownloaded) &&
                                                            "opacity-50 cursor-not-allowed hover:translate-y-0"
                                                        )}
                                                    >
                                                        {isDownloading ? (
                                                            <>
                                                                <Loader2 className="w-4 h-4 mr-2 animate-spin" />{" "}
                                                                Downloading...
                                                            </>
                                                        ) : isDownloaded ? (
                                                            <>
                                                                <CheckCircle2 className="w-4 h-4 mr-2" />{" "}
                                                                Already Downloaded
                                                            </>
                                                        ) : (
                                                            <>
                                                                <Download className="w-4 h-4 mr-2" />{" "}
                                                                Download All (
                                                                {fileInfo.total_size_display})
                                                            </>
                                                        )}
                                                    </button>
                                                </div>
                                            ) : (
                                                // GGUF: Quantization picker
                                                <div className="space-y-1.5">
                                                    {fileInfo.files.map((f) => {
                                                        const isDownloadingThis =
                                                            downloadingFiles.has(f.filename);
                                                        const fileProgress =
                                                            downloading[f.filename];

                                                        return (
                                                            <div
                                                                key={f.filename}
                                                                className="flex items-center gap-2 py-1.5 px-2 rounded-lg hover:bg-muted/30 transition-colors"
                                                            >
                                                                <div className="flex-1 min-w-0">
                                                                    <div className="flex items-center gap-2">
                                                                        {f.quant_type && (
                                                                            <span className="text-[10px] font-bold uppercase tracking-wider bg-primary/10 text-primary px-1.5 py-0.5 rounded border border-primary/20 shrink-0">
                                                                                {f.quant_type}
                                                                            </span>
                                                                        )}
                                                                        <span className="text-[11px] font-mono text-muted-foreground truncate">
                                                                            {f.filename}
                                                                        </span>
                                                                    </div>
                                                                    {fileProgress !== undefined && (
                                                                        <div className="relative overflow-hidden bg-secondary rounded-full w-full h-1.5 mt-1">
                                                                            <div
                                                                                className="bg-primary h-full rounded-full transition-all duration-500 ease-in-out"
                                                                                style={{
                                                                                    width: `${fileProgress || 0}%`,
                                                                                }}
                                                                            />
                                                                        </div>
                                                                    )}
                                                                </div>
                                                                <span className="text-[10px] font-mono text-muted-foreground/50 shrink-0">
                                                                    {f.size_display}
                                                                </span>
                                                                <button
                                                                    onClick={(e) => {
                                                                        e.stopPropagation();
                                                                        handleDownloadSingle(
                                                                            model.id,
                                                                            f,
                                                                            fileInfo.mmproj_file
                                                                        );
                                                                    }}
                                                                    disabled={isDownloadingThis}
                                                                    className={cn(
                                                                        "p-1.5 rounded-lg transition-all shrink-0",
                                                                        isDownloadingThis
                                                                            ? "opacity-50 cursor-not-allowed"
                                                                            : "hover:bg-primary/10 text-muted-foreground hover:text-primary"
                                                                    )}
                                                                    title={`Download ${f.quant_type || f.filename}`}
                                                                >
                                                                    {isDownloadingThis ? (
                                                                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                                                                    ) : (
                                                                        <Download className="w-3.5 h-3.5" />
                                                                    )}
                                                                </button>
                                                            </div>
                                                        );
                                                    })}
                                                </div>
                                            )}

                                            {/* Open on HF */}
                                            <button
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    openExternal(
                                                        `https://huggingface.co/${model.id}`
                                                    );
                                                }}
                                                className="flex items-center gap-1.5 text-[11px] text-muted-foreground/50 hover:text-primary transition-colors mt-2 cursor-pointer"
                                            >
                                                <ExternalLink className="w-3 h-3" />
                                                View on HuggingFace
                                            </button>
                                        </div>
                                    )}
                                </div>
                            )}
                        </div>
                    );
                })}
            </div>

            {/* Empty states */}
            {hasSearched && !isSearching && results.length === 0 && debouncedQuery.trim() && (
                <div className="text-center py-8 text-muted-foreground text-sm">
                    No models found for &quot;{debouncedQuery}&quot;. Try a different search term.
                </div>
            )}

            {/* Loading state for initial trending fetch */}
            {!hasSearched && isSearching && (
                <div className="text-center py-12 text-muted-foreground/50 text-sm space-y-2">
                    <Loader2 className="w-6 h-6 mx-auto mb-3 animate-spin opacity-40" />
                    <p>Loading popular {activeFilterDef.label} models...</p>
                </div>
            )}

            {/* Fallback: no trending results loaded and not searching */}
            {!hasSearched && !isSearching && results.length === 0 && (
                <div className="text-center py-12 text-muted-foreground/50 text-sm space-y-2">
                    <Search className="w-8 h-8 mx-auto mb-3 opacity-30" />
                    <p>Search for {activeFilterDef.label} models</p>
                    <p className="text-xs opacity-60">
                        {pipelineFilter === 'embedding' ? 'Try "bge", "nomic", "gte", or "qwen-embedding"'
                            : pipelineFilter === 'diffusion' ? 'Try "flux", "stable-diffusion", or "sdxl"'
                                : pipelineFilter === 'stt' ? 'Try "whisper", "parakeet", or "voxtral"'
                                    : pipelineFilter === 'video' ? 'Try "mochi", "ltx-video", or "cogvideo"'
                                        : 'Try "llama", "qwen", "gemma", or "phi"'
                        }
                    </p>
                </div>
            )}
        </div>
    );
}
