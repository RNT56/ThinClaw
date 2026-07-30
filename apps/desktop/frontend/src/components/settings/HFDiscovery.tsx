/**
 * Hugging Face model discovery backed by ThinClaw's runtime capability and
 * artifact-plan APIs. The frontend never invents compatibility, categories,
 * file groups, or destination paths.
 */
import {
    AlertTriangle,
    ArrowDownToLine,
    CheckCircle2,
    ChevronDown,
    Database,
    Download,
    ExternalLink,
    Eye,
    Heart,
    Image,
    Info,
    Loader2,
    Mic,
    RefreshCw,
    Search,
    Shield,
    Type,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import type {
    EngineInfo,
    HfCapabilityProfileDto,
    HfDownloadArtifact,
    HfModelCard,
    HfModelFilePlan,
} from "../../lib/bindings";
import { commandClient } from "../../lib/command-client";
import { directCommands } from "../../lib/generated/direct-commands";
import { unwrapResult } from "../../lib/guards";
import {
    createRequestGenerationGuard,
    createHfSearchCache,
    classifyHfHubError,
    effectiveHfCompanionArtifactId,
    findInstalledArtifactSelection,
    filtersFromProfiles,
    huggingFaceRepositoryUrl,
    isRepositoryInstalled,
    mergeHfModelCards,
    requiresHfCompanionArtifact,
    type HfHubRemediationKind,
    type HfModelTaskId,
} from "../../lib/hf-models";
import { cn } from "../../lib/utils";
import { useModelContext } from "../model-context";

interface FilePlanState {
    status: "loading" | "ready" | "error";
    plan?: HfModelFilePlan;
    error?: string;
}

const TRENDING_INITIAL_LIMIT = 15;
const QUERY_INITIAL_LIMIT = 20;
const SEARCH_PAGE_SIZE = 20;
const MAX_SEARCH_LIMIT = 100;
const TRENDING_CACHE_TTL_MS = 5 * 60 * 1_000;
const HF_TOKEN_SETTINGS_URL = "https://huggingface.co/settings/tokens";
const trendingSearchCache = createHfSearchCache<HfModelCard>(
    TRENDING_CACHE_TTL_MS,
);

interface SearchWindow {
    requestKey: string;
    requestedLimit: number;
}

function formatDownloads(value: number): string {
    if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
    if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
    return value.toString();
}

function boundedProgress(value: number | undefined): number | null {
    if (value === undefined) return null;
    if (!Number.isFinite(value)) return 0;
    return Math.min(100, Math.max(0, value));
}

function taskIcon(task: HfModelTaskId) {
    switch (task) {
        case "vision":
            return Eye;
        case "embedding":
            return Database;
        case "stt":
            return Mic;
        case "diffusion":
            return Image;
        default:
            return Type;
    }
}

function errorMessage(error: unknown): string {
    if (error instanceof Error) return error.message;
    return typeof error === "string" ? error : "Unknown Hugging Face error";
}

interface HfRemediationProps {
    kind: HfHubRemediationKind;
    onOpenUrl: (url: string) => void;
    repoUrl?: string | null;
    onRetry?: () => void;
}

function HfRemediation({
    kind,
    onOpenUrl,
    repoUrl,
    onRetry,
}: HfRemediationProps) {
    const isRateLimit = kind === "rate-limit";
    return (
        <div
            className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 text-xs text-muted-foreground"
            data-testid={`hf-${kind}-remediation`}
            role="note"
        >
            <div className="flex gap-2">
                <Shield className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-500" />
                <div>
                    <p className="font-semibold text-foreground">
                        {isRateLimit ? "Hugging Face request limit reached" : "Hugging Face access required"}
                    </p>
                    <p className="mt-1">
                        {isRateLimit
                            ? "Wait briefly and retry. A Hugging Face read token can also provide higher authenticated limits."
                            : "Open the repository and accept its license or request access. Then create a read token and save it under Settings → Secrets → Hugging Face Token."}
                    </p>
                </div>
            </div>
            <div className="mt-2 flex flex-wrap gap-3 pl-5">
                {repoUrl && (
                    <button
                        type="button"
                        onClick={() => onOpenUrl(repoUrl)}
                        className="inline-flex items-center gap-1 font-semibold text-primary hover:underline"
                        data-testid="hf-open-access-page"
                    >
                        <ExternalLink className="h-3 w-3" />
                        Open access page
                    </button>
                )}
                <button
                    type="button"
                    onClick={() => onOpenUrl(HF_TOKEN_SETTINGS_URL)}
                    className="inline-flex items-center gap-1 font-semibold text-primary hover:underline"
                    data-testid="hf-open-token-settings"
                >
                    <ExternalLink className="h-3 w-3" />
                    Create or manage token
                </button>
                {onRetry && (
                    <button
                        type="button"
                        onClick={onRetry}
                        className="inline-flex items-center gap-1 font-semibold text-primary hover:underline"
                    >
                        <RefreshCw className="h-3 w-3" />
                        Retry
                    </button>
                )}
            </div>
        </div>
    );
}

export function HFDiscovery({ isVisible = true }: { isVisible?: boolean }) {
    const {
        downloading,
        downloadHfSelection,
        cancelDownload,
        engineInfo: contextEngineInfo,
        discoveryState,
        setDiscoveryState,
        localModels,
    } = useModelContext();
    const [localEngineInfo, setLocalEngineInfo] = useState<EngineInfo | null>(null);
    const engineInfo = contextEngineInfo ?? localEngineInfo;
    const {
        searchQuery,
        results,
        hasSearched,
        hasMore,
        expandedModel,
        downloadingFiles,
        repoProgress,
    } =
        discoveryState;

    const [profiles, setProfiles] = useState<HfCapabilityProfileDto[]>([]);
    const [profilesLoading, setProfilesLoading] = useState(true);
    const [profilesError, setProfilesError] = useState<string | null>(null);
    const [profilesAttempt, setProfilesAttempt] = useState(0);
    const filters = useMemo(
        () => filtersFromProfiles(profiles, engineInfo?.id),
        [profiles, engineInfo?.id],
    );
    const [activeTask, setActiveTask] = useState<HfModelTaskId | null>(null);
    const activeFilter = filters.find(filter => filter.task === activeTask) ?? filters[0] ?? null;

    const [debouncedQuery, setDebouncedQuery] = useState(searchQuery);
    const [debounceAttempt, setDebounceAttempt] = useState(0);
    const [isSearching, setIsSearching] = useState(false);
    const [searchError, setSearchError] = useState<string | null>(null);
    const [searchAttempt, setSearchAttempt] = useState(0);
    const [searchWindow, setSearchWindow] = useState<SearchWindow | null>(null);
    const [isLoadingMore, setIsLoadingMore] = useState(false);
    const [loadMoreError, setLoadMoreError] = useState<string | null>(null);
    const [filePlans, setFilePlans] = useState<Record<string, FilePlanState>>({});
    const [selectedCompanions, setSelectedCompanions] = useState<Record<string, string>>({});
    const searchGuard = useRef(createRequestGenerationGuard());
    const lastRawQuery = useRef(searchQuery);

    const setSearchQuery = useCallback((query: string) => {
        // Invalidate in the input event itself. Waiting for a passive effect
        // leaves a small window where a just-finished old request can paint
        // results that no longer match the visible query.
        searchGuard.current.invalidate();
        setIsSearching(true);
        setSearchError(null);
        setIsLoadingMore(false);
        setLoadMoreError(null);
        setDiscoveryState(previous => ({ ...previous, searchQuery: query }));
    }, [setDiscoveryState]);
    const setSearchResults = useCallback((models: HfModelCard[], more: boolean) => {
        setDiscoveryState(previous => ({
            ...previous,
            results: models,
            hasMore: more,
        }));
    }, [setDiscoveryState]);
    const setExpandedModel = useCallback((repoId: string | null) => {
        setDiscoveryState(previous => ({ ...previous, expandedModel: repoId }));
    }, [setDiscoveryState]);
    const selectTask = useCallback((task: HfModelTaskId) => {
        if (task === activeFilter?.task) return;
        searchGuard.current.invalidate();
        setIsSearching(true);
        setSearchError(null);
        setIsLoadingMore(false);
        setLoadMoreError(null);
        setSearchResults([], false);
        setActiveTask(task);
    }, [activeFilter?.task, setSearchResults]);

    useEffect(() => {
        if (!contextEngineInfo) {
            directCommands.directRuntimeGetActiveEngineInfo()
                .then(setLocalEngineInfo)
                .catch(error => console.error("Failed to get engine info:", error));
        }
    }, [contextEngineInfo]);

    useEffect(() => {
        let cancelled = false;
        setProfilesLoading(true);
        setProfilesError(null);
        directCommands.directRuntimeGetHfCapabilities()
            .then(value => {
                if (!cancelled) setProfiles(value);
            })
            .catch(error => {
                if (!cancelled) {
                    setProfiles([]);
                    setProfilesError(errorMessage(error));
                }
            })
            .finally(() => {
                if (!cancelled) setProfilesLoading(false);
            });
        return () => {
            cancelled = true;
        };
    }, [engineInfo?.id, profilesAttempt]);

    useEffect(() => {
        if (filters.length === 0) {
            setActiveTask(null);
            return;
        }
        if (!activeTask || !filters.some(filter => filter.task === activeTask)) {
            setActiveTask(filters[0].task);
        }
    }, [activeTask, filters]);

    useEffect(() => {
        if (lastRawQuery.current === searchQuery) return;
        lastRawQuery.current = searchQuery;
        // Invalidate immediately so an old response cannot flash while the new
        // query is still inside the debounce window.
        searchGuard.current.invalidate();
        setIsSearching(true);
        setSearchError(null);
        setIsLoadingMore(false);
        setLoadMoreError(null);
        const timer = window.setTimeout(() => {
            setDebouncedQuery(searchQuery);
            // A user can type and return to the already-debounced value before
            // this timer fires. The value setter is then a no-op, so this
            // generation still has to trigger a replacement search/cache read.
            setDebounceAttempt(attempt => attempt + 1);
        }, 350);
        return () => window.clearTimeout(timer);
    }, [searchQuery]);

    useEffect(() => {
        if (!isVisible || !engineInfo || !activeFilter) return;
        const cacheKey = `${engineInfo.id}:${activeFilter.task}`;
        const query = debouncedQuery.trim();
        // A task/engine/visibility change can rerun this effect while an input
        // edit is still debouncing. Do not search or restore cached cards for
        // a query different from the text the user currently sees.
        if (query !== searchQuery.trim()) return;
        const generation = searchGuard.current.begin();
        const requestKey = JSON.stringify([engineInfo.id, activeFilter.task, query]);
        const cached = !query ? trendingSearchCache.get(cacheKey) : undefined;
        if (cached) {
            setSearchResults([...cached.models], cached.hasMore);
            setDiscoveryState(previous => ({ ...previous, hasSearched: true }));
            setSearchWindow({
                requestKey,
                requestedLimit: cached.requestedLimit,
            });
            setSearchError(null);
            setIsSearching(false);
            setIsLoadingMore(false);
            setLoadMoreError(null);
            return;
        }

        const requestedLimit = query ? QUERY_INITIAL_LIMIT : TRENDING_INITIAL_LIMIT;
        setIsSearching(true);
        setIsLoadingMore(false);
        setSearchError(null);
        setLoadMoreError(null);
        setSearchWindow({ requestKey, requestedLimit });
        setSearchResults([], false);
        void directCommands.directRuntimeDiscoverHfModelsV2(
            query,
            activeFilter.task,
            requestedLimit,
        ).then(result => {
            const response = unwrapResult(result, "Hugging Face model search");
            if (!searchGuard.current.isCurrent(generation)) return;
            if (
                response.engine_id !== engineInfo.id
                || response.task !== activeFilter.task
            ) {
                throw new Error("Hugging Face search returned results for a different runtime filter");
            }
            setSearchResults(response.models, response.has_more);
            setDiscoveryState(previous => ({ ...previous, hasSearched: true }));
            if (!query) {
                trendingSearchCache.set(cacheKey, {
                    models: response.models,
                    hasMore: response.has_more,
                    requestedLimit,
                });
            }
        }).catch(error => {
            if (!searchGuard.current.isCurrent(generation)) return;
            const message = errorMessage(error);
            setSearchError(message);
            setSearchResults([], false);
            if (query) toast.error("Hugging Face search failed", { description: message });
        }).finally(() => {
            if (searchGuard.current.isCurrent(generation)) setIsSearching(false);
        });

        return () => searchGuard.current.invalidate();
    }, [
        activeFilter?.task,
        debounceAttempt,
        debouncedQuery,
        engineInfo?.id,
        isVisible,
        searchAttempt,
        setDiscoveryState,
        setSearchResults,
    ]);

    const retrySearch = useCallback(() => {
        searchGuard.current.invalidate();
        setSearchAttempt(attempt => attempt + 1);
    }, []);

    const loadMore = useCallback(async () => {
        if (
            !engineInfo
            || !activeFilter
            || isSearching
            || isLoadingMore
            || !hasMore
        ) {
            return;
        }
        const query = debouncedQuery.trim();
        if (query !== searchQuery.trim()) return;

        const requestKey = JSON.stringify([engineInfo.id, activeFilter.task, query]);
        const currentLimit = searchWindow?.requestKey === requestKey
            ? searchWindow.requestedLimit
            : Math.max(
                query ? QUERY_INITIAL_LIMIT : TRENDING_INITIAL_LIMIT,
                results.length,
            );
        const requestedLimit = Math.min(
            MAX_SEARCH_LIMIT,
            Math.max(currentLimit + SEARCH_PAGE_SIZE, results.length + SEARCH_PAGE_SIZE),
        );
        if (requestedLimit <= currentLimit) return;

        const generation = searchGuard.current.begin();
        setIsLoadingMore(true);
        setLoadMoreError(null);
        try {
            const response = unwrapResult(
                await directCommands.directRuntimeDiscoverHfModelsV2(
                    query,
                    activeFilter.task,
                    requestedLimit,
                ),
                "Hugging Face model search",
            );
            if (!searchGuard.current.isCurrent(generation)) return;
            if (
                response.engine_id !== engineInfo.id
                || response.task !== activeFilter.task
            ) {
                throw new Error("Hugging Face search returned results for a different runtime filter");
            }

            const merged = mergeHfModelCards(results, response.models)
                .slice(0, requestedLimit);
            setSearchResults(merged, response.has_more);
            setSearchWindow({ requestKey, requestedLimit });
            if (!query) {
                trendingSearchCache.set(
                    `${engineInfo.id}:${activeFilter.task}`,
                    {
                        models: merged,
                        hasMore: response.has_more,
                        requestedLimit,
                    },
                );
            }
        } catch (error) {
            if (!searchGuard.current.isCurrent(generation)) return;
            setLoadMoreError(errorMessage(error));
        } finally {
            if (searchGuard.current.isCurrent(generation)) setIsLoadingMore(false);
        }
    }, [
        activeFilter,
        debouncedQuery,
        engineInfo,
        hasMore,
        isLoadingMore,
        isSearching,
        results,
        searchQuery,
        searchWindow,
        setSearchResults,
    ]);

    useEffect(() => {
        setFilePlans({});
        setSelectedCompanions({});
        setExpandedModel(null);
    }, [activeFilter?.task, engineInfo?.id, setExpandedModel]);

    const planKey = useCallback(
        (repoId: string) => `${engineInfo?.id ?? "none"}:${activeFilter?.task ?? "none"}:${repoId}`,
        [activeFilter?.task, engineInfo?.id],
    );

    const loadPlan = useCallback(async (repoId: string) => {
        if (!activeFilter || !engineInfo) return;
        const key = planKey(repoId);
        setFilePlans(previous => ({
            ...previous,
            [key]: { status: "loading" },
        }));
        try {
            const plan = unwrapResult(
                await directCommands.directRuntimeGetModelFilesV2(repoId, activeFilter.task),
                "Hugging Face artifact plan",
            );
            if (
                plan.repo_id !== repoId
                || plan.engine_id !== engineInfo.id
                || plan.task !== activeFilter.task
            ) {
                throw new Error(
                    "Hugging Face artifact plan returned a different repository or runtime filter",
                );
            }
            setFilePlans(previous => ({
                ...previous,
                [key]: { status: "ready", plan },
            }));
        } catch (error) {
            setFilePlans(previous => ({
                ...previous,
                [key]: { status: "error", error: errorMessage(error) },
            }));
        }
    }, [activeFilter, engineInfo, planKey]);

    const toggleExpanded = useCallback((repoId: string) => {
        if (expandedModel === repoId) {
            setExpandedModel(null);
            return;
        }
        setExpandedModel(repoId);
        const current = filePlans[planKey(repoId)];
        if (!current || current.status === "error") void loadPlan(repoId);
    }, [expandedModel, filePlans, loadPlan, planKey, setExpandedModel]);

    const downloadArtifact = useCallback(async (
        plan: HfModelFilePlan,
        artifact: HfDownloadArtifact,
    ) => {
        const companionKey = JSON.stringify([
            plan.repo_id,
            plan.revision,
            plan.task,
            artifact.id,
        ]);
        const companionArtifactId = effectiveHfCompanionArtifactId(
            plan,
            selectedCompanions[companionKey],
        );
        if (requiresHfCompanionArtifact(plan) && !companionArtifactId) {
            throw new Error(
                `No compatible vision projector is available for ${plan.repo_id}`,
            );
        }
        await downloadHfSelection({
            repo_id: plan.repo_id,
            revision: plan.revision,
            task: plan.task,
            artifact_id: artifact.id,
            companion_artifact_id: companionArtifactId,
            destination_name: null,
        }, artifact.download_id);
    }, [downloadHfSelection, selectedCompanions]);

    const openHfUrl = useCallback((url: string) => {
        void commandClient.openUrl(url).catch(error => {
            toast.error("Could not open Hugging Face", {
                description: errorMessage(error),
            });
        });
    }, []);

    const openRepository = useCallback((repoId: string) => {
        const url = huggingFaceRepositoryUrl(repoId);
        if (!url) {
            toast.error("Invalid Hugging Face repository link");
            return;
        }
        openHfUrl(url);
    }, [openHfUrl]);

    const sortedResults = useMemo(() => {
        return [...results].sort((left, right) => {
            const leftPlan = filePlans[planKey(left.id)]?.plan;
            const rightPlan = filePlans[planKey(right.id)]?.plan;
            const leftDownloading = leftPlan?.artifacts.some(
                artifact => downloadingFiles.has(artifact.download_id),
            ) ? 2 : 0;
            const rightDownloading = rightPlan?.artifacts.some(
                artifact => downloadingFiles.has(artifact.download_id),
            ) ? 2 : 0;
            const leftInstalled = isRepositoryInstalled(localModels, left.id) ? 1 : 0;
            const rightInstalled = isRepositoryInstalled(localModels, right.id) ? 1 : 0;
            return (rightDownloading + rightInstalled) - (leftDownloading + leftInstalled);
        });
    }, [downloadingFiles, filePlans, localModels, planKey, results]);
    const currentRequestKey = engineInfo && activeFilter
        ? JSON.stringify([engineInfo.id, activeFilter.task, debouncedQuery.trim()])
        : null;
    const reachedSearchLimit = Boolean(
        currentRequestKey
        && searchWindow?.requestKey === currentRequestKey
        && searchWindow.requestedLimit >= MAX_SEARCH_LIMIT,
    );
    const searchRemediation = searchError
        ? classifyHfHubError(searchError)
        : null;
    const loadMoreRemediation = loadMoreError
        ? classifyHfHubError(loadMoreError)
        : null;

    if (profilesLoading) {
        return (
            <div className="flex items-center justify-center py-16 text-sm text-muted-foreground">
                <Loader2 className="w-4 h-4 mr-2 animate-spin" /> Loading runtime model support…
            </div>
        );
    }

    if (profilesError) {
        return (
            <div className="rounded-xl border border-destructive/20 bg-destructive/5 p-5 text-sm">
                <div className="flex items-center gap-2 font-semibold text-destructive">
                    <AlertTriangle className="w-4 h-4" /> Could not load model capabilities
                </div>
                <p className="mt-2 text-muted-foreground">{profilesError}</p>
                <button
                    onClick={() => setProfilesAttempt(attempt => attempt + 1)}
                    className="mt-3 inline-flex items-center gap-1 font-semibold text-primary"
                >
                    <RefreshCw className="w-3.5 h-3.5" /> Retry
                </button>
            </div>
        );
    }

    if (!engineInfo || filters.length === 0) {
        const isOllama = engineInfo?.id === "ollama";
        return (
            <div className="rounded-xl border border-border/60 bg-muted/20 p-6 text-sm">
                <div className="flex items-center gap-2 font-semibold">
                    <Info className="w-4 h-4 text-primary" />
                    {isOllama ? "Ollama manages its own model library" : "No local model runtime"}
                </div>
                <p className="mt-2 text-muted-foreground">
                    {isOllama
                        ? "Raw Hugging Face files are not imported into Ollama. Use Ollama’s pull/create workflow, then select the model from its library."
                        : "This build uses cloud inference and cannot install Hugging Face models locally."}
                </p>
            </div>
        );
    }

    return (
        <div
            className="space-y-4"
            aria-busy={isSearching || isLoadingMore}
        >
            <div className="flex items-center justify-between gap-3">
                <div>
                    <h3 className="font-semibold">Hugging Face Models</h3>
                    <p className="text-xs text-muted-foreground">
                        Compatible with {engineInfo.display_name}; artifacts are validated before download
                    </p>
                </div>
                <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                    <Shield className="w-3 h-3" />
                    {activeFilter?.formatTag.toUpperCase()}
                </div>
            </div>

            <div className="flex gap-2 overflow-x-auto pb-1">
                {filters.map(filter => {
                    const Icon = taskIcon(filter.task);
                    return (
                        <button
                            key={filter.task}
                            type="button"
                            onClick={() => selectTask(filter.task)}
                            aria-pressed={activeFilter?.task === filter.task}
                            aria-label={`Show ${filter.label} models for ${engineInfo.display_name}`}
                            className={cn(
                                "flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs whitespace-nowrap border transition-colors",
                                activeFilter?.task === filter.task
                                    ? "bg-foreground text-background border-foreground"
                                    : "bg-muted/50 text-muted-foreground border-transparent hover:text-foreground",
                            )}
                        >
                            <Icon className="w-3 h-3" /> {filter.label}
                        </button>
                    );
                })}
            </div>

            {activeFilter?.compatibilityHint && (
                <div className="flex gap-2 rounded-lg border border-blue-500/15 bg-blue-500/5 p-3 text-xs text-muted-foreground">
                    <Info className="w-3.5 h-3.5 shrink-0 text-blue-500" />
                    {activeFilter.compatibilityHint}
                </div>
            )}

            <div className="relative">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
                <input
                    type="search"
                    value={searchQuery}
                    onChange={event => setSearchQuery(event.target.value)}
                    placeholder={activeFilter?.placeholder}
                    aria-label="Search Hugging Face models"
                    autoComplete="off"
                    className="w-full rounded-xl border border-border/60 bg-muted/30 py-2.5 pl-10 pr-10 text-sm outline-hidden focus:ring-1 focus:ring-primary/30"
                />
                {isSearching && (
                    <Loader2 className="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 animate-spin text-muted-foreground" />
                )}
            </div>

            {searchError && !isSearching && (
                <div className="space-y-2">
                    <div
                        className="rounded-lg border border-destructive/20 bg-destructive/5 p-4 text-sm"
                        role="alert"
                    >
                        <p className="text-destructive">{searchError}</p>
                        {!searchRemediation && (
                            <button
                                type="button"
                                onClick={retrySearch}
                                className="mt-2 inline-flex items-center gap-1 text-xs font-semibold text-primary"
                            >
                                <RefreshCw className="h-3 w-3" /> Retry search
                            </button>
                        )}
                    </div>
                    {searchRemediation && (
                        <HfRemediation
                            kind={searchRemediation}
                            onOpenUrl={openHfUrl}
                            onRetry={retrySearch}
                        />
                    )}
                </div>
            )}

            {!isSearching && hasSearched && !searchError && sortedResults.length === 0 && (
                <div className="py-12 text-center text-sm text-muted-foreground">
                    No runtime-compatible models found.
                </div>
            )}

            <div className="grid gap-3">
                {sortedResults.map((model, index) => {
                    const key = planKey(model.id);
                    const planState = filePlans[key];
                    const plan = planState?.plan;
                    const repositoryUrl = huggingFaceRepositoryUrl(model.id);
                    const detailsId = `hf-model-details-${index}`;
                    const planRemediation = planState?.error
                        ? classifyHfHubError(planState.error)
                        : null;
                    const installedRepo = isRepositoryInstalled(localModels, model.id);
                    const hasActiveDownload = plan?.artifacts.some(
                        artifact => downloadingFiles.has(artifact.download_id),
                    ) ?? false;
                    return (
                        <div
                            key={model.id}
                            className={cn(
                                "rounded-xl border bg-card/40 transition-colors",
                                hasActiveDownload ? "border-primary/40" : "border-border/50",
                            )}
                            data-testid="hf-model-card"
                            data-repo-id={model.id}
                        >
                            <button
                                type="button"
                                onClick={() => toggleExpanded(model.id)}
                                aria-expanded={expandedModel === model.id}
                                aria-controls={detailsId}
                                aria-label={`${expandedModel === model.id ? "Collapse" : "Expand"} ${model.id}`}
                                className="w-full p-4 text-left flex items-start gap-3"
                            >
                                <div className="flex-1 min-w-0">
                                    <div className="flex items-center gap-2">
                                        <span className="font-semibold truncate">{model.id}</span>
                                        {model.gated && (
                                            <span className="text-[9px] font-bold text-amber-500 border border-amber-500/20 bg-amber-500/10 px-1.5 py-0.5 rounded">GATED</span>
                                        )}
                                        {installedRepo && (
                                            <span className="text-[9px] font-bold text-emerald-500 border border-emerald-500/20 bg-emerald-500/10 px-1.5 py-0.5 rounded">ON DISK</span>
                                        )}
                                    </div>
                                    <div className="mt-1 flex gap-3 text-[11px] text-muted-foreground">
                                        <span className="flex items-center gap-1"><ArrowDownToLine className="w-3 h-3" />{formatDownloads(model.downloads)}</span>
                                        <span className="flex items-center gap-1"><Heart className="w-3 h-3" />{model.likes}</span>
                                    </div>
                                </div>
                                <ChevronDown className={cn(
                                    "w-4 h-4 text-muted-foreground transition-transform",
                                    expandedModel === model.id && "rotate-180",
                                )} />
                            </button>

                            {expandedModel === model.id && (
                                <div
                                    id={detailsId}
                                    className="border-t border-border/40 p-4 space-y-3"
                                >
                                    {repositoryUrl && (
                                        <button
                                            type="button"
                                            onClick={() => openRepository(model.id)}
                                            aria-label={`Open ${model.id} on Hugging Face`}
                                            className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-primary"
                                        >
                                            <ExternalLink className="w-3 h-3" /> View repository
                                        </button>
                                    )}

                                    {model.gated && planState?.status !== "ready" && (
                                        <HfRemediation
                                            kind="access"
                                            onOpenUrl={openHfUrl}
                                            repoUrl={repositoryUrl}
                                        />
                                    )}

                                    {planState?.status === "loading" && (
                                        <div className="flex items-center py-4 text-xs text-muted-foreground">
                                            <Loader2 className="w-3.5 h-3.5 mr-2 animate-spin" /> Resolving revision and artifacts…
                                        </div>
                                    )}

                                    {planState?.status === "error" && (
                                        <div className="rounded-lg border border-destructive/20 bg-destructive/5 p-3 text-xs">
                                            <p className="text-destructive">{planState.error}</p>
                                            <button
                                                type="button"
                                                onClick={() => void loadPlan(model.id)}
                                                aria-label={`Retry artifact plan for ${model.id}`}
                                                className="mt-2 inline-flex items-center gap-1 font-semibold text-primary"
                                            >
                                                <RefreshCw className="w-3 h-3" /> Retry
                                            </button>
                                        </div>
                                    )}

                                    {planRemediation && !model.gated && (
                                        <HfRemediation
                                            kind={planRemediation}
                                            onOpenUrl={openHfUrl}
                                            repoUrl={
                                                planRemediation === "access"
                                                    ? repositoryUrl
                                                    : null
                                            }
                                        />
                                    )}

                                    {plan?.warnings.map(warning => (
                                        <div key={warning} className="flex gap-2 text-xs text-amber-600 dark:text-amber-400">
                                            <AlertTriangle className="w-3.5 h-3.5 shrink-0" /> {warning}
                                        </div>
                                    ))}

                                    {plan?.artifacts.map(artifact => {
                                        const companionKey = JSON.stringify([
                                            plan.repo_id,
                                            plan.revision,
                                            plan.task,
                                            artifact.id,
                                        ]);
                                        const companionRequired =
                                            requiresHfCompanionArtifact(plan);
                                        const companionArtifactId =
                                            effectiveHfCompanionArtifactId(
                                                plan,
                                                selectedCompanions[companionKey],
                                            );
                                        const missingRequiredCompanion =
                                            companionRequired && !companionArtifactId;
                                        const installed = Boolean(findInstalledArtifactSelection(
                                            localModels,
                                            {
                                                repoId: plan.repo_id,
                                                revision: plan.revision,
                                                engineId: plan.engine_id,
                                                task: plan.task,
                                                artifactId: artifact.id,
                                                companionArtifactId,
                                            },
                                        ));
                                        const active = downloadingFiles.has(artifact.download_id);
                                        const progress = downloading[artifact.download_id];
                                        const detailedProgress = repoProgress[artifact.download_id];
                                        const visibleProgress = boundedProgress(
                                            detailedProgress?.pct ?? progress,
                                        );
                                        return (
                                            <div key={artifact.id} className="rounded-lg border border-border/50 bg-muted/20 p-3 space-y-2">
                                                <div className="flex items-start justify-between gap-3">
                                                    <div className="min-w-0">
                                                        <div className="flex items-center gap-2">
                                                            <span className="text-xs font-semibold">{artifact.label}</span>
                                                            {artifact.files.length > 1 && (
                                                                <span className="text-[9px] text-muted-foreground">{artifact.files.length} shards</span>
                                                            )}
                                                        </div>
                                                        <p className="text-[10px] font-mono text-muted-foreground truncate">
                                                            {artifact.primary_file ?? "Complete repository"}
                                                        </p>
                                                    </div>
                                                    <span className="text-[10px] font-mono text-muted-foreground shrink-0">
                                                        {artifact.total_size_display}
                                                    </span>
                                                </div>

                                                {plan.companion_artifacts.length > 0 && (
                                                    <label className="block text-[10px] text-muted-foreground">
                                                        Vision projector {companionRequired ? "(required)" : "(optional)"}
                                                        <select
                                                            value={companionArtifactId ?? ""}
                                                            onChange={event => setSelectedCompanions(previous => ({
                                                                ...previous,
                                                                [companionKey]: event.target.value,
                                                            }))}
                                                            className="mt-1 w-full rounded-md border border-border/60 bg-background px-2 py-1.5 text-xs"
                                                            disabled={active}
                                                        >
                                                            {!companionRequired && (
                                                                <option value="">No projector</option>
                                                            )}
                                                            {plan.companion_artifacts.map(companion => (
                                                                <option key={companion.id} value={companion.id}>
                                                                    {companion.label} · {companion.total_size_display}
                                                                </option>
                                                            ))}
                                                        </select>
                                                    </label>
                                                )}

                                                {missingRequiredCompanion && (
                                                    <div className="flex gap-2 text-xs text-destructive">
                                                        <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
                                                        This vision model has no compatible projector and cannot be installed.
                                                    </div>
                                                )}

                                                {visibleProgress !== null && (
                                                    <div className="space-y-1">
                                                        <div className="flex justify-between text-[10px] text-muted-foreground">
                                                            <span className="truncate">{detailedProgress?.currentFile || "Preparing…"}</span>
                                                            <span>{Math.round(visibleProgress)}%</span>
                                                        </div>
                                                        <div
                                                            className="h-1.5 overflow-hidden rounded-full bg-secondary"
                                                            role="progressbar"
                                                            aria-label={`Downloading ${artifact.label} from ${model.id}`}
                                                            aria-valuemin={0}
                                                            aria-valuemax={100}
                                                            aria-valuenow={Math.round(visibleProgress)}
                                                        >
                                                            <div
                                                                className="h-full bg-primary transition-all"
                                                                style={{ width: `${visibleProgress}%` }}
                                                            />
                                                        </div>
                                                    </div>
                                                )}

                                                <button
                                                    type="button"
                                                    onClick={() => {
                                                        if (active) {
                                                            void cancelDownload(artifact.download_id);
                                                        } else {
                                                            void downloadArtifact(plan, artifact).catch(() => undefined);
                                                        }
                                                    }}
                                                    disabled={installed || missingRequiredCompanion}
                                                    aria-label={
                                                        active
                                                            ? `Cancel download of ${artifact.label} from ${model.id}`
                                                            : installed
                                                                ? `${artifact.label} from ${model.id} is installed`
                                                                : `Download ${artifact.label} from ${model.id}`
                                                    }
                                                    className={cn(
                                                        "w-full rounded-lg border py-2 text-xs font-semibold flex items-center justify-center gap-1.5 transition-colors",
                                                        installed
                                                            ? "border-border text-muted-foreground opacity-60"
                                                            : active
                                                                ? "border-destructive/30 text-destructive hover:bg-destructive hover:text-destructive-foreground"
                                                                : "border-primary/30 text-primary hover:bg-primary hover:text-primary-foreground",
                                                    )}
                                                >
                                                    {active ? (
                                                        <><Loader2 className="w-3.5 h-3.5 animate-spin" /> Cancel download</>
                                                    ) : installed ? (
                                                        <><CheckCircle2 className="w-3.5 h-3.5" /> Installed</>
                                                    ) : (
                                                        <><Download className="w-3.5 h-3.5" /> Download this artifact</>
                                                    )}
                                                </button>
                                            </div>
                                        );
                                    })}
                                </div>
                            )}
                        </div>
                    );
                })}
            </div>

            {!isSearching && loadMoreError && (
                <div className="space-y-2">
                    <div
                        className="rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-xs"
                        role="alert"
                    >
                        <span className="text-destructive">
                            Could not load more models: {loadMoreError}
                        </span>
                    </div>
                    {loadMoreRemediation && (
                        <HfRemediation
                            kind={loadMoreRemediation}
                            onOpenUrl={openHfUrl}
                            onRetry={() => void loadMore()}
                        />
                    )}
                </div>
            )}

            {!isSearching
                && !searchError
                && hasMore
                && !reachedSearchLimit
                && (
                <button
                    type="button"
                    onClick={() => void loadMore()}
                    disabled={isLoadingMore}
                    aria-label="Load more Hugging Face models"
                    data-testid="hf-load-more"
                    className="flex w-full items-center justify-center gap-2 rounded-lg border border-border/60 bg-muted/20 px-3 py-2.5 text-xs font-semibold text-primary transition-colors hover:bg-muted/40 disabled:cursor-wait disabled:opacity-70"
                >
                    {isLoadingMore ? (
                        <>
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            Loading more compatible models…
                        </>
                    ) : (
                        <>
                            <ChevronDown className="h-3.5 w-3.5" />
                            Load more compatible models
                        </>
                    )}
                </button>
            )}

            {!isSearching
                && !searchError
                && hasMore
                && reachedSearchLimit
                && (
                <div className="rounded-lg border border-border/50 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
                    Reached the {MAX_SEARCH_LIMIT}-result search window. Refine the search to explore more compatible models.
                </div>
            )}
        </div>
    );
}
