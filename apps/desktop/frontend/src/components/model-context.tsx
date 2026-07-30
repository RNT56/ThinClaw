import React, { createContext, useContext, useEffect, useState, useCallback, useMemo, useRef } from "react";
import { appDataDir } from "@tauri-apps/api/path";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import {
    ModelFile,
    SystemSpecs,
    commands,
    StandardAsset,
    EngineInfo,
    LocalRuntimeSnapshot,
    HfDownloadSelectionRequest,
    HfDownloadResult,
    HfModelCard,
} from "../lib/bindings";
import { directCommands } from "../lib/generated/direct-commands";
import { commandClient } from "../lib/command-client";
import { unwrapResult } from "../lib/guards";
import { getMigratedLocalStorageItem, isOnboardingInProgress, setMigratedLocalStorageItem } from "../lib/local-storage-migration";
import { bridgeErrorMessage } from "../lib/command-errors";
import { hfDownloadSelectionFingerprint } from "../lib/hf-models";
import {
    isModelPathAffectedByRemoval,
    modelRemovalPaths,
    selectedModelRolesForRemoval,
    selectedRolesForModelRemoval,
    type ModelSelectionSnapshot,
} from "../lib/model-library-view";
import { loadInitialModelState } from "../lib/model-initialization";

import { MODEL_LIBRARY, ExtendedModelDefinition as ModelDefinition } from "../lib/model-library";

// Enhanced Model Definitions interface re-export for convenience if needed, or just import from lib
export type { ModelDefinition };
export { MODEL_LIBRARY };


// Re-export for compatibility with consumers expecting RECOMMENDED_MODELS
export const RECOMMENDED_MODELS = MODEL_LIBRARY;

interface DownloadEvent {
    filename: string;
    total: number;
    downloaded: number;
    percentage: number;
}

interface RepoProgressInfo {
    pct: number;
    currentFile: string;
    fileIndex: number;
    fileCount: number;
    /** Per-file progress: filename → percentage */
    filePct: Record<string, number>;
}

interface DiscoveryState {
    searchQuery: string;
    results: HfModelCard[];
    hasSearched: boolean;
    /** The backend stopped at the requested/bounded result window. */
    hasMore: boolean;
    expandedModel: string | null;
    downloadingFiles: Set<string>;
    repoProgress: Record<string, RepoProgressInfo>;
}

// ---------------------------------------------------------------------------
// Context type (single API surface — consumers don't need to know about the
// internal two-context split)
// ---------------------------------------------------------------------------
interface ModelContextType {
    models: ModelDefinition[];
    localModels: ModelFile[];
    downloading: Record<string, number>;
    currentModelPath: string;
    currentEmbeddingModelPath: string;
    currentVisionModelPath: string;
    currentSttModelPath: string;
    currentImageGenModelPath: string;
    currentSummarizerModelPath: string;
    currentModelTemplate: string;
    setModelPath: (path: string, template?: string) => void;
    setEmbeddingModelPath: (path: string) => void;
    setVisionModelPath: (path: string) => void;
    setSttModelPath: (path: string) => void;
    setImageGenModelPath: (path: string) => void;
    setSummarizerModelPath: (path: string) => void;
    /**
     * Returns the accepted inventory, or null when the request failed or was
     * superseded by a newer refresh.
     */
    refreshModels: () => Promise<ModelFile[] | null>;
    downloadSpeed: string;
    selectModel: (modelId: string) => void;
    activeCategory: string;
    setActiveCategory: (category: string) => void;
    cancelDownload: (filename: string) => Promise<void>;
    deactivateModel: (installRoot: string) => Promise<void>;
    deleteModel: (filename: string) => Promise<void>;
    isRefreshing: boolean;
    systemSpecs: SystemSpecs | null;
    modelsDir: string | null;
    standardAssets: StandardAsset[];
    checkStandardAssets: () => Promise<void>;
    downloadStandardAsset: (filename: string) => Promise<void>;
    maxContext: number;
    setMaxContext: (size: number) => void;
    isRestarting: boolean;
    setIsRestarting: (val: boolean) => void;
    /** Download one backend-validated Hugging Face artifact selection. */
    downloadHfSelection: (
        request: HfDownloadSelectionRequest,
        downloadId: string,
    ) => Promise<HfDownloadResult>;
    /** Active inference engine info (null while loading) */
    engineInfo: EngineInfo | null;
    /** Public local runtime snapshot; endpoint secrets are redacted by the backend. */
    runtimeSnapshot: LocalRuntimeSnapshot | null;
    /**
     * Returns the accepted snapshot, or null when the request failed or was
     * superseded by a newer refresh. A stale completion is never returned.
     */
    refreshRuntimeSnapshot: () => Promise<LocalRuntimeSnapshot | null>;
    /** Persistent HF discovery state (survives tab switches) */
    discoveryState: DiscoveryState;
    setDiscoveryState: React.Dispatch<React.SetStateAction<DiscoveryState>>;
}

// ---------------------------------------------------------------------------
// Internal contexts: state (rarely changes) vs progress (changes during DL)
// ---------------------------------------------------------------------------

/** Stable state — models, paths, engine info, system specs, categories.
 *  Only changes on user action (model select, category switch, etc). */
type ModelStateContextType = Omit<ModelContextType, 'downloading' | 'discoveryState' | 'setDiscoveryState'>;

/** Hot state — download progress, discovery state.
 *  Changes at ~4fps during active downloads (throttled). */
interface ModelProgressContextType {
    downloading: Record<string, number>;
    discoveryState: DiscoveryState;
    setDiscoveryState: React.Dispatch<React.SetStateAction<DiscoveryState>>;
}

const ModelStateContext = createContext<ModelStateContextType | undefined>(undefined);
const ModelProgressContext = createContext<ModelProgressContextType | undefined>(undefined);

const DEFAULT_PATH = "";

export function ModelProvider({ children }: { children: React.ReactNode }) {
    const [localModels, setLocalModels] = useState<ModelFile[]>([]);
    const [downloading, setDownloading] = useState<Record<string, number>>({});
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [isRestarting, setIsRestarting] = useState(false);
    const [systemSpecs, setSystemSpecs] = useState<SystemSpecs | null>(null);
    const [_currentModel, setCurrentModel] = useState<ModelDefinition | null>(null);
    const [activeCategory, setActiveCategory] = useState("Chat");
    const [modelsDir, setModelsDir] = useState<string | null>(null);
    const [downloadSpeed] = useState("");
    const [standardAssets, setStandardAssets] = useState<StandardAsset[]>([]);
    // The curated MODEL_LIBRARY is the catalog source; it has no runtime updater
    // (the former remote-catalog sync to localhost:8000 was dead and was removed).
    const [models] = useState<ModelDefinition[]>(MODEL_LIBRARY);
    const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);
    const [runtimeSnapshot, setRuntimeSnapshot] = useState<LocalRuntimeSnapshot | null>(null);
    const runtimeSnapshotGenerationRef = useRef(0);
    const modelInventoryGenerationRef = useRef(0);

    // Persistent discovery state — lifted from HFDiscovery so it survives tab switches
    const [discoveryState, setDiscoveryState] = useState<DiscoveryState>({
        searchQuery: "",
        results: [],
        hasSearched: false,
        hasMore: false,
        expandedModel: null,
        downloadingFiles: new Set(),
        repoProgress: {},
    });

    const refreshRuntimeSnapshot = useCallback(async (): Promise<LocalRuntimeSnapshot | null> => {
        const generation = ++runtimeSnapshotGenerationRef.current;
        try {
            const result = await directCommands.directRuntimeSnapshot();
            if (result.status === "ok") {
                if (generation === runtimeSnapshotGenerationRef.current) {
                    setRuntimeSnapshot(result.data);
                    return result.data;
                }
                return null;
            }
            if (generation === runtimeSnapshotGenerationRef.current) {
                console.warn("Failed to get runtime snapshot:", result.error);
            }
        } catch (err) {
            if (generation === runtimeSnapshotGenerationRef.current) {
                console.warn("Failed to get runtime snapshot:", err);
            }
        }
        if (generation === runtimeSnapshotGenerationRef.current) {
            setRuntimeSnapshot(null);
        }
        return null;
    }, []);

    // Load engine info and public runtime snapshot on mount, then refresh when
    // local runtime lifecycle events can change readiness or capabilities.
    useEffect(() => {
        directCommands.directRuntimeGetActiveEngineInfo()
            .then(setEngineInfo)
            .catch(err => console.warn("Failed to get engine info:", err));
        refreshRuntimeSnapshot();

        const onFocus = () => { refreshRuntimeSnapshot(); };
        window.addEventListener("focus", onFocus);

        const unlistenSidecar = listen("sidecar_event", () => {
            refreshRuntimeSnapshot();
        });
        const unlistenSetup = listen<{ stage: string }>("engine_setup_progress", (event) => {
            if (event.payload.stage === "complete" || event.payload.stage === "error") {
                refreshRuntimeSnapshot();
            }
        });

        return () => {
            window.removeEventListener("focus", onFocus);
            unlistenSidecar.then(fn => fn());
            unlistenSetup.then(fn => fn());
        };
    }, [refreshRuntimeSnapshot]);

    const checkStandardAssets = useCallback(async () => {
        try {
            const result = await commands.checkMissingStandardAssets();
            if (result.status === "error") throw new Error(bridgeErrorMessage(result.error));
            setStandardAssets(result.data);
        } catch (e) {
            console.error(e);
        }
    }, []);

    useEffect(() => {
        appDataDir().then(dir => {
            setModelsDir(`${dir}/models`);
        });
    }, []);

    // -----------------------------------------------------------------------
    // Memoized callbacks — stable identity between renders
    // -----------------------------------------------------------------------

    const selectModel = useCallback((modelId: string) => {
        const model = models.find(m => m.id === modelId);
        if (model) setCurrentModel(model);
    }, [models]);

    // Model Selection State
    const [currentModelPath, _setCurrentModelPath] = useState<string>(() => {
        return getMigratedLocalStorageItem('modelPath') || DEFAULT_PATH;
    });

    const [currentEmbeddingModelPath, _setCurrentEmbeddingModelPath] = useState<string>(() => {
        return getMigratedLocalStorageItem('embeddingModelPath') || DEFAULT_PATH;
    });

    const [currentVisionModelPath, _setCurrentVisionModelPath] = useState<string>(() => {
        return getMigratedLocalStorageItem('visionModelPath') || DEFAULT_PATH;
    });

    const [currentSttModelPath, _setCurrentSttModelPath] = useState<string>(() => {
        return getMigratedLocalStorageItem('sttModelPath') || DEFAULT_PATH;
    });

    const [currentImageGenModelPath, _setCurrentImageGenModelPath] = useState<string>(() => {
        return getMigratedLocalStorageItem('imageGenModelPath') || DEFAULT_PATH;
    });

    const [currentSummarizerModelPath, _setCurrentSummarizerModelPath] = useState<string>(() => {
        return getMigratedLocalStorageItem('summarizerModelPath') || DEFAULT_PATH;
    });

    const selectedModelPathsRef = useRef<ModelSelectionSnapshot>({
        chat: currentModelPath,
        embedding: currentEmbeddingModelPath,
        vision: currentVisionModelPath,
        stt: currentSttModelPath,
        diffusion: currentImageGenModelPath,
        summarizer: currentSummarizerModelPath,
    });
    const deletingModelPathsRef = useRef<Set<string>>(new Set());
    const mutatingModelRootsRef = useRef<Set<string>>(new Set());
    const mutatingModelPathsRef = useRef<Set<string>>(new Set());

    const [currentModelTemplate, _setCurrentModelTemplate] = useState<string>(() => {
        return getMigratedLocalStorageItem('modelTemplate') || "chatml";
    });

    const setModelPath = useCallback((path: string, template?: string) => {
        if (
            path
            && (
                isModelPathAffectedByRemoval(path, deletingModelPathsRef.current)
                || isModelPathAffectedByRemoval(path, mutatingModelPathsRef.current)
            )
        ) {
            toast.error("This model is currently being changed");
            return;
        }
        selectedModelPathsRef.current.chat = path;
        _setCurrentModelPath(path);
        setMigratedLocalStorageItem('modelPath', path);
        if (template) {
            _setCurrentModelTemplate(template);
            setMigratedLocalStorageItem('modelTemplate', template);
        } else {
            // Heuristic if not provided (e.g. local scan)
            let inferred = "chatml";
            const lower = path.toLowerCase();
            if (lower.includes("llama-3") || lower.includes("llama3")) inferred = "llama3";
            else if (lower.includes("mistral") || lower.includes("mixtral")) inferred = "mistral";
            else if (lower.includes("gemma")) inferred = "gemma";
            else if (lower.includes("qwen")) inferred = "qwen";

            _setCurrentModelTemplate(inferred);
            setMigratedLocalStorageItem('modelTemplate', inferred);
        }
    }, []);

    const setEmbeddingModelPath = useCallback((path: string) => {
        if (
            path
            && (
                isModelPathAffectedByRemoval(path, deletingModelPathsRef.current)
                || isModelPathAffectedByRemoval(path, mutatingModelPathsRef.current)
            )
        ) {
            toast.error("This model is currently being changed");
            return;
        }
        selectedModelPathsRef.current.embedding = path;
        _setCurrentEmbeddingModelPath(path);
        setMigratedLocalStorageItem('embeddingModelPath', path);
    }, []);

    const setVisionModelPath = useCallback((path: string) => {
        if (
            path
            && (
                isModelPathAffectedByRemoval(path, deletingModelPathsRef.current)
                || isModelPathAffectedByRemoval(path, mutatingModelPathsRef.current)
            )
        ) {
            toast.error("This model is currently being changed");
            return;
        }
        selectedModelPathsRef.current.vision = path;
        _setCurrentVisionModelPath(path);
        setMigratedLocalStorageItem('visionModelPath', path);
    }, []);

    const setSttModelPath = useCallback((path: string) => {
        if (
            path
            && (
                isModelPathAffectedByRemoval(path, deletingModelPathsRef.current)
                || isModelPathAffectedByRemoval(path, mutatingModelPathsRef.current)
            )
        ) {
            toast.error("This model is currently being changed");
            return;
        }
        selectedModelPathsRef.current.stt = path;
        _setCurrentSttModelPath(path);
        setMigratedLocalStorageItem('sttModelPath', path);
    }, []);

    const setImageGenModelPath = useCallback((path: string) => {
        if (
            path
            && (
                isModelPathAffectedByRemoval(path, deletingModelPathsRef.current)
                || isModelPathAffectedByRemoval(path, mutatingModelPathsRef.current)
            )
        ) {
            toast.error("This model is currently being changed");
            return;
        }
        selectedModelPathsRef.current.diffusion = path;
        _setCurrentImageGenModelPath(path);
        setMigratedLocalStorageItem('imageGenModelPath', path);
    }, []);

    const setSummarizerModelPath = useCallback((path: string) => {
        if (
            path
            && (
                isModelPathAffectedByRemoval(path, deletingModelPathsRef.current)
                || isModelPathAffectedByRemoval(path, mutatingModelPathsRef.current)
            )
        ) {
            toast.error("This model is currently being changed");
            return;
        }
        selectedModelPathsRef.current.summarizer = path;
        _setCurrentSummarizerModelPath(path);
        setMigratedLocalStorageItem('summarizerModelPath', path);
    }, []);

    const [maxContext, _setMaxContext] = useState<number>(() => {
        const stored = getMigratedLocalStorageItem('maxContext');
        return stored ? parseInt(stored) : 32768; // Default to 32k
    });

    const setMaxContext = useCallback((size: number) => {
        _setMaxContext(size);
        setMigratedLocalStorageItem('maxContext', size.toString());
    }, []);

    const refreshModels = useCallback(async (): Promise<ModelFile[] | null> => {
        const generation = ++modelInventoryGenerationRef.current;
        setIsRefreshing(true);
        try {
            const models = await commandClient.listModels();
            if (generation === modelInventoryGenerationRef.current) {
                setLocalModels(models);
                return models;
            }
            return null;
        } catch (e) {
            if (generation === modelInventoryGenerationRef.current) {
                console.error("Failed to list models", e);
                toast.error("Failed to list models");
            }
            return null;
        } finally {
            if (generation === modelInventoryGenerationRef.current) {
                setIsRefreshing(false);
            }
        }
    }, []);

    // Check hardware and recommend model on first empty run
    useEffect(() => {
        const initializeModelsAndHardware = async () => {
            const {
                inventory: localFiles,
                specs,
                specsError,
            } = await loadInitialModelState({
                refreshInventory: refreshModels,
                getSystemSpecs: commands.getSystemSpecs,
            });
            if (specs) setSystemSpecs(specs);
            if (specsError) {
                console.error("Failed to init system specs:", specsError);
            }
            if (!specs || localFiles === null) return;

            // Check if we need to recommend
            const hasChecked = getMigratedLocalStorageItem('firstRunCheck');
            if (!hasChecked && localFiles.length === 0) {
                // Skip if onboarding wizard is handling model selection
                if (isOnboardingInProgress()) {
                    setMigratedLocalStorageItem('firstRunCheck', "true");
                    return;
                }
                const ramGB = specs.total_memory / (1024 * 1024 * 1024);

                let recommendedId = "qwen3-vl-4b-instruct"; // Safe default for < 8GB
                if (ramGB >= 24) recommendedId = "gemma-3-27b-it-qat";
                else if (ramGB >= 8) recommendedId = "gemma-3-12b-it-qat";

                const model = models.find(m => m.id === recommendedId);

                if (model) {
                    toast("Hardware Detected", {
                        description: `We recommend ${model.name} for your system (${Math.round(ramGB)}GB RAM). Open Models → Discover to choose a validated, revision-pinned artifact.`,
                        duration: 10000,
                    });
                }
                setMigratedLocalStorageItem('firstRunCheck', "true");
            }
        };

        initializeModelsAndHardware();

        // Polling loop for real-time resource tracking (30 second default)
        const interval = setInterval(async () => {
            try {
                const specs = await commands.getSystemSpecs();
                if (specs) setSystemSpecs(specs);
            } catch (e) {
                console.error("Health poll failed:", e);
            }
        }, 30000);

        return () => clearInterval(interval);
    }, [models, refreshModels]);

    // -----------------------------------------------------------------------
    // Throttled progress buffer — prevents per-chunk re-renders of the entire
    // component tree.  Progress events fire many times per second during
    // downloads; we accumulate them in a ref and flush to state at ~4fps.
    // -----------------------------------------------------------------------
    const progressBufferRef = useRef<Record<string, RepoProgressInfo>>({});
    const downloadPctBufferRef = useRef<Record<string, number>>({});
    const progressFlushTimer = useRef<ReturnType<typeof setInterval> | null>(null);
    const hfDownloadPromisesRef = useRef<Map<string, {
        fingerprint: string;
        promise: Promise<HfDownloadResult>;
    }>>(new Map());
    const standardDownloadPromisesRef = useRef<Map<string, Promise<void>>>(new Map());
    const locallyOwnedDownloadsRef = useRef<Set<string>>(new Set());
    const finalizedDownloadIdsRef = useRef<Set<string>>(new Set());

    // Start/stop the flush timer based on active downloads
    useEffect(() => {
        const hasActiveDownloads = Object.keys(downloading).length > 0;

        if (hasActiveDownloads && !progressFlushTimer.current) {
            progressFlushTimer.current = setInterval(() => {
                // Flush download percentages
                const pctBuf = downloadPctBufferRef.current;
                if (Object.keys(pctBuf).length > 0) {
                    setDownloading(prev => ({ ...prev, ...pctBuf }));
                }
                // Flush discovery progress
                const discBuf = progressBufferRef.current;
                if (Object.keys(discBuf).length > 0) {
                    setDiscoveryState(prev => ({
                        ...prev,
                        repoProgress: { ...prev.repoProgress, ...discBuf },
                    }));
                }
            }, 250); // ~4fps
        } else if (!hasActiveDownloads && progressFlushTimer.current) {
            clearInterval(progressFlushTimer.current);
            progressFlushTimer.current = null;
            // Final flush
            const pctBuf = downloadPctBufferRef.current;
            if (Object.keys(pctBuf).length > 0) {
                setDownloading(prev => ({ ...prev, ...pctBuf }));
                downloadPctBufferRef.current = {};
            }
            const discBuf = progressBufferRef.current;
            if (Object.keys(discBuf).length > 0) {
                setDiscoveryState(prev => ({
                    ...prev,
                    repoProgress: { ...prev.repoProgress, ...discBuf },
                }));
                progressBufferRef.current = {};
            }
        }

        return () => {
            if (progressFlushTimer.current) {
                clearInterval(progressFlushTimer.current);
                progressFlushTimer.current = null;
            }
        };
    }, [downloading, setDiscoveryState]);

    // Listen for download progress globally
    useEffect(() => {
        const unlisten = listen<DownloadEvent>("download_progress", (event) => {
            const { filename, percentage } = event.payload;
            if (finalizedDownloadIdsRef.current.has(filename)) return;

            // Buffer percentage — flushed to state by the timer above
            downloadPctBufferRef.current[filename] = percentage;

            // Buffer per-file progress updates — flushed to state by the timer above
            const payload = event.payload as any;
            if (payload.current_file || payload.file_count) {
                // Repo-level progress event
                const existing = progressBufferRef.current[filename];
                progressBufferRef.current[filename] = {
                    pct: percentage,
                    currentFile: payload.current_file ?? "",
                    fileIndex: payload.file_index ?? 0,
                    fileCount: payload.file_count ?? 1,
                    filePct: {
                        ...(existing?.filePct ?? {}),
                        ...(payload.current_file ? { [payload.current_file]: payload.file_percentage ?? 0 } : {}),
                    },
                };
            }

            if (event.payload.percentage >= 100) {
                if (!locallyOwnedDownloadsRef.current.has(filename)) {
                    setTimeout(() => {
                        if (locallyOwnedDownloadsRef.current.has(filename)) return;
                        finalizedDownloadIdsRef.current.add(filename);
                        delete downloadPctBufferRef.current[filename];
                        delete progressBufferRef.current[filename];
                        setDownloading(previous => {
                            const next = { ...previous };
                            delete next[filename];
                            return next;
                        });
                        setDiscoveryState(previous => {
                            const repoProgress = { ...previous.repoProgress };
                            delete repoProgress[filename];
                            const downloadingFiles = new Set(previous.downloadingFiles);
                            downloadingFiles.delete(filename);
                            return { ...previous, repoProgress, downloadingFiles };
                        });
                        refreshModels();
                    }, 250);
                }
            }
        });

        return () => {
            unlisten.then(f => f());
        }
    }, [refreshModels]);


    const cancelDownload = useCallback(async (filename: string) => {
        try {
            await commandClient.cancelDownload(filename);
            toast.info("Cancellation requested");
        } catch (e) {
            console.warn("Backend cancel failed (task might be finished):", e);
        } finally {
            if (!locallyOwnedDownloadsRef.current.has(filename)) {
                finalizedDownloadIdsRef.current.add(filename);
                delete downloadPctBufferRef.current[filename];
                delete progressBufferRef.current[filename];
                setDownloading(prev => {
                    const copy = { ...prev };
                    delete copy[filename];
                    return copy;
                });
                setDiscoveryState(previous => {
                    const repoProgress = { ...previous.repoProgress };
                    delete repoProgress[filename];
                    const downloadingFiles = new Set(previous.downloadingFiles);
                    downloadingFiles.delete(filename);
                    return { ...previous, repoProgress, downloadingFiles };
                });
            }
        }
    }, []);

    const deactivateModel = useCallback(async (installRoot: string) => {
        if (mutatingModelRootsRef.current.has(installRoot)) {
            toast.info("This model is already being changed");
            return;
        }
        const model = localModels.find(candidate => candidate.install_root === installRoot);
        if (!model) {
            toast.error("The selected model is no longer in the local inventory");
            return;
        }
        const affectedPaths = modelRemovalPaths(model);
        const roles = selectedModelRolesForRemoval(
            affectedPaths,
            selectedModelPathsRef.current,
        );
        if (!Object.values(roles).some(Boolean)) return;

        mutatingModelRootsRef.current.add(installRoot);
        for (const path of affectedPaths) mutatingModelPathsRef.current.add(path);
        try {
            unwrapResult(
                await directCommands.directRuntimeDeactivateModelServices(
                    installRoot,
                    roles.chat,
                    roles.embedding,
                    roles.summarizer,
                    roles.stt,
                    roles.image,
                ),
                "stop local model services",
            );

            // Compare after the awaited backend operation. A different model
            // selected in the meantime must never be erased by a stale action.
            const latest = selectedModelPathsRef.current;
            if (isModelPathAffectedByRemoval(latest.chat, affectedPaths)) {
                setModelPath(DEFAULT_PATH);
            }
            if (isModelPathAffectedByRemoval(latest.embedding, affectedPaths)) {
                setEmbeddingModelPath(DEFAULT_PATH);
            }
            if (isModelPathAffectedByRemoval(latest.vision, affectedPaths)) {
                setVisionModelPath(DEFAULT_PATH);
            }
            if (isModelPathAffectedByRemoval(latest.stt, affectedPaths)) {
                setSttModelPath(DEFAULT_PATH);
            }
            if (isModelPathAffectedByRemoval(latest.diffusion, affectedPaths)) {
                setImageGenModelPath(DEFAULT_PATH);
            }
            if (isModelPathAffectedByRemoval(latest.summarizer, affectedPaths)) {
                setSummarizerModelPath(DEFAULT_PATH);
            }
            await refreshRuntimeSnapshot();
            toast.success("Model deactivated");
        } catch (error) {
            toast.error("Could not deactivate model", {
                description: bridgeErrorMessage(error),
            });
        } finally {
            mutatingModelRootsRef.current.delete(installRoot);
            for (const path of affectedPaths) mutatingModelPathsRef.current.delete(path);
        }
    }, [
        localModels,
        refreshRuntimeSnapshot,
        setEmbeddingModelPath,
        setImageGenModelPath,
        setModelPath,
        setSttModelPath,
        setSummarizerModelPath,
        setVisionModelPath,
    ]);

    const deleteModel = useCallback(async (filename: string) => {
        if (mutatingModelRootsRef.current.has(filename)) {
            toast.info("This model is already being changed");
            return;
        }
        const model = localModels.find(candidate => candidate.install_root === filename);
        const removedSelections = modelRemovalPaths(model);
        const selectedRoles = selectedRolesForModelRemoval(
            removedSelections,
            selectedModelPathsRef.current,
        );

        // Never unlink files that a local runtime may still have mapped or may
        // lazily read. The role can be deactivated from the same model card.
        if (selectedRoles.length > 0) {
            toast.error("Deactivate this model before deleting it", {
                description: `It is still selected for ${selectedRoles.join(", ")}.`,
            });
            return;
        }

        mutatingModelRootsRef.current.add(filename);
        for (const path of removedSelections) deletingModelPathsRef.current.add(path);
        for (const path of removedSelections) mutatingModelPathsRef.current.add(path);
        try {
            await commandClient.deleteLocalModel(filename);
            if (model) {
                // Defensive cleanup for a preference changed concurrently
                // while the backend deletion was in progress.
                const latestSelections = selectedModelPathsRef.current;
                if (isModelPathAffectedByRemoval(latestSelections.chat, removedSelections)) {
                    setModelPath(DEFAULT_PATH);
                }
                if (isModelPathAffectedByRemoval(latestSelections.embedding, removedSelections)) {
                    setEmbeddingModelPath(DEFAULT_PATH);
                }
                if (isModelPathAffectedByRemoval(latestSelections.vision, removedSelections)) {
                    setVisionModelPath(DEFAULT_PATH);
                }
                if (isModelPathAffectedByRemoval(latestSelections.stt, removedSelections)) {
                    setSttModelPath(DEFAULT_PATH);
                }
                if (isModelPathAffectedByRemoval(latestSelections.diffusion, removedSelections)) {
                    setImageGenModelPath(DEFAULT_PATH);
                }
                if (isModelPathAffectedByRemoval(latestSelections.summarizer, removedSelections)) {
                    setSummarizerModelPath(DEFAULT_PATH);
                }
            }
            await refreshModels();
            toast.success("Model deleted");
        } catch (e) {
            console.error("Delete failed:", e);
            toast.error(`Failed to delete: ${e} `);
        } finally {
            mutatingModelRootsRef.current.delete(filename);
            for (const path of removedSelections) deletingModelPathsRef.current.delete(path);
            for (const path of removedSelections) mutatingModelPathsRef.current.delete(path);
        }
    }, [
        localModels,
        refreshModels,
        setEmbeddingModelPath,
        setImageGenModelPath,
        setModelPath,
        setSttModelPath,
        setSummarizerModelPath,
        setVisionModelPath,
    ]);

    // Download a backend-produced artifact selection. The download ID is
    // emitted by the backend on every progress event and is the only tracking
    // key used across context and discovery UI.
    const downloadHfSelection = useCallback(async (
        request: HfDownloadSelectionRequest,
        downloadId: string,
    ): Promise<HfDownloadResult> => {
        const requestFingerprint = hfDownloadSelectionFingerprint(request);
        const existing = hfDownloadPromisesRef.current.get(downloadId);
        if (existing) {
            if (existing.fingerprint !== requestFingerprint) {
                const message =
                    "A different selection for this Hugging Face artifact is already downloading";
                toast.error("Hugging Face download conflict", { description: message });
                throw new Error(message);
            }
            return existing.promise;
        }

        finalizedDownloadIdsRef.current.delete(downloadId);
        locallyOwnedDownloadsRef.current.add(downloadId);
        setDownloading(prev => ({ ...prev, [downloadId]: 0 }));
        setDiscoveryState(prev => ({
            ...prev,
            downloadingFiles: new Set([...prev.downloadingFiles, downloadId]),
        }));

        let operation: Promise<HfDownloadResult>;
        operation = (async () => {
            try {
                const result = unwrapResult(
                    await directCommands.directRuntimeDownloadHfSelection(request),
                    "HuggingFace model download"
                );
                if (result.download_id !== downloadId) {
                    throw new Error("HuggingFace download returned an unexpected progress identity");
                }
                await refreshModels();
                toast.success(`Downloaded ${result.repo_id}`);
                return result;
            } catch (error) {
                const message = bridgeErrorMessage(error);
                if (message.toLowerCase().includes("cancel")) {
                    toast.info("Hugging Face download cancelled");
                } else {
                    toast.error("HuggingFace download failed", {
                        description: message,
                    });
                }
                throw error;
            } finally {
                if (
                    hfDownloadPromisesRef.current.get(downloadId)?.fingerprint
                    === requestFingerprint
                ) {
                    hfDownloadPromisesRef.current.delete(downloadId);
                    locallyOwnedDownloadsRef.current.delete(downloadId);
                    finalizedDownloadIdsRef.current.add(downloadId);
                    delete downloadPctBufferRef.current[downloadId];
                    delete progressBufferRef.current[downloadId];
                    setDownloading(prev => {
                        const copy = { ...prev };
                        delete copy[downloadId];
                        return copy;
                    });
                    setDiscoveryState(prev => {
                        const repoProgress = { ...prev.repoProgress };
                        delete repoProgress[downloadId];
                        const downloadingFiles = new Set(prev.downloadingFiles);
                        downloadingFiles.delete(downloadId);
                        return { ...prev, repoProgress, downloadingFiles };
                    });
                }
            }
        })();
        hfDownloadPromisesRef.current.set(downloadId, {
            fingerprint: requestFingerprint,
            promise: operation,
        });
        return operation;
    }, [refreshModels]);

    const downloadStandardAsset = useCallback(async (filename: string) => {
        if (standardDownloadPromisesRef.current.has(filename)) {
            return standardDownloadPromisesRef.current.get(filename);
        }

        finalizedDownloadIdsRef.current.delete(filename);
        locallyOwnedDownloadsRef.current.add(filename);
        setDownloading(prev => ({ ...prev, [filename]: 0 }));
        toast.info(`Downloading Standard Asset: ${filename}`);
        let operation: Promise<void>;
        operation = (async () => {
            try {
                await commandClient.downloadStandardAsset(filename);
                await checkStandardAssets();
                toast.success(`Downloaded ${filename}`);
            } catch (e) {
                toast.error(`Standard Asset Download Failed: ${bridgeErrorMessage(e)}`);
            } finally {
                standardDownloadPromisesRef.current.delete(filename);
                locallyOwnedDownloadsRef.current.delete(filename);
                finalizedDownloadIdsRef.current.add(filename);
                delete downloadPctBufferRef.current[filename];
                delete progressBufferRef.current[filename];
                setDownloading(prev => {
                    const copy = { ...prev };
                    delete copy[filename];
                    return copy;
                });
            }
        })();
        standardDownloadPromisesRef.current.set(filename, operation);
        return operation;
    }, [checkStandardAssets]);

    // -----------------------------------------------------------------------
    // Memoized context values — split into stable state vs hot progress
    // -----------------------------------------------------------------------

    const stateValue = useMemo<ModelStateContextType>(() => ({
        models,
        localModels,
        currentModelPath,
        currentEmbeddingModelPath,
        currentVisionModelPath,
        currentSttModelPath,
        currentImageGenModelPath,
        currentSummarizerModelPath,
        currentModelTemplate,
        setModelPath,
        setEmbeddingModelPath,
        setVisionModelPath,
        setSttModelPath,
        setImageGenModelPath,
        setSummarizerModelPath,
        refreshModels,
        downloadSpeed,
        selectModel,
        activeCategory,
        setActiveCategory,
        cancelDownload,
        deactivateModel,
        deleteModel,
        isRefreshing,
        modelsDir,
        systemSpecs,
        standardAssets,
        checkStandardAssets,
        downloadStandardAsset,
        maxContext,
        setMaxContext,
        isRestarting,
        setIsRestarting,
        downloadHfSelection,
        engineInfo,
        runtimeSnapshot,
        refreshRuntimeSnapshot,
    }), [
        models, localModels, currentModelPath,
        currentEmbeddingModelPath, currentVisionModelPath, currentSttModelPath,
        currentImageGenModelPath, currentSummarizerModelPath, currentModelTemplate,
        setModelPath, setEmbeddingModelPath, setVisionModelPath, setSttModelPath,
        setImageGenModelPath, setSummarizerModelPath, refreshModels,
        downloadSpeed, selectModel, activeCategory, cancelDownload, deactivateModel, deleteModel,
        isRefreshing, modelsDir, systemSpecs, standardAssets, checkStandardAssets,
        downloadStandardAsset, maxContext, isRestarting, downloadHfSelection, engineInfo,
        runtimeSnapshot, refreshRuntimeSnapshot,
    ]);

    const progressValue = useMemo<ModelProgressContextType>(() => ({
        downloading,
        discoveryState,
        setDiscoveryState,
    }), [downloading, discoveryState]);

    return (
        <ModelStateContext.Provider value={stateValue}>
            <ModelProgressContext.Provider value={progressValue}>
                {children}
            </ModelProgressContext.Provider>
        </ModelStateContext.Provider>
    );
}

/**
 * Single hook to access the full model context.
 *
 * Internally reads from two contexts: `ModelStateContext` (stable) and
 * `ModelProgressContext` (hot during downloads).  Components that only
 * use state fields (paths, models, engine info, etc.) won't re-render
 * when download progress changes.
 */
export function useModelContext(): ModelContextType {
    const state = useContext(ModelStateContext);
    const progress = useContext(ModelProgressContext);
    if (!state || !progress) throw new Error("useModelContext must be used within ModelProvider");
    return useMemo(() => ({ ...state, ...progress }), [state, progress]);
}
