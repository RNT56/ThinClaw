import React, { createContext, useContext, useEffect, useState, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { appDataDir } from "@tauri-apps/api/path";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { ModelFile, SystemSpecs, commands, StandardAsset, EngineInfo, LocalRuntimeSnapshot } from "../lib/bindings";
import { directCommands } from "../lib/generated/direct-commands";
import { unwrapResult } from "../lib/guards";
import { getMigratedLocalStorageItem, isOnboardingInProgress, setMigratedLocalStorageItem } from "../lib/local-storage-migration";

import { MODEL_LIBRARY, ExtendedModelDefinition as ModelDefinition, ModelVariant } from "../lib/model-library";

// Enhanced Model Definitions interface re-export for convenience if needed, or just import from lib
export type { ModelVariant, ModelDefinition };
export { MODEL_LIBRARY };


// Re-export for compatibility with consumers expecting RECOMMENDED_MODELS
export const RECOMMENDED_MODELS = MODEL_LIBRARY;

interface DownloadEvent {
    filename: string;
    total: number;
    downloaded: number;
    percentage: number;
}

// Persistent discovery state — survives tab switches
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
    refreshModels: () => Promise<ModelFile[]>;
    startDownload: (model: ModelDefinition, variant?: ModelVariant) => Promise<void>;
    downloadSpeed: string;
    selectModel: (modelId: string) => void;
    activeCategory: string;
    setActiveCategory: (category: string) => void;
    cancelDownload: (filename: string) => Promise<void>;
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
    /** Download files from HuggingFace Hub (used by HFDiscovery) */
    downloadHfFiles: (repoId: string, files: string[], destSubdir?: string | null, category?: string) => Promise<void>;
    /** Active inference engine info (null while loading) */
    engineInfo: EngineInfo | null;
    /** Public local runtime snapshot; endpoint secrets are redacted by the backend. */
    runtimeSnapshot: LocalRuntimeSnapshot | null;
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
    const [models, setModels] = useState<ModelDefinition[]>(MODEL_LIBRARY);
    const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);
    const [runtimeSnapshot, setRuntimeSnapshot] = useState<LocalRuntimeSnapshot | null>(null);

    // Persistent discovery state — lifted from HFDiscovery so it survives tab switches
    const [discoveryState, setDiscoveryState] = useState<DiscoveryState>({
        searchQuery: "",
        results: [],
        hasSearched: false,
        expandedModel: null,
        downloadingFiles: new Set(),
        repoProgress: {},
    });

    const refreshRuntimeSnapshot = useCallback(async (): Promise<LocalRuntimeSnapshot | null> => {
        try {
            const result = await directCommands.directRuntimeSnapshot();
            if (result.status === "ok") {
                setRuntimeSnapshot(result.data);
                return result.data;
            }
            console.warn("Failed to get runtime snapshot:", result.error);
        } catch (err) {
            console.warn("Failed to get runtime snapshot:", err);
        }
        setRuntimeSnapshot(null);
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

    const syncRemoteCatalog = useCallback(async () => {
        try {
            // Check if server is available (currently hardcoded as per spec)
            const health = await fetch("http://localhost:8000/api/v1/health").catch(() => null);
            if (!health || !health.ok) {
                // Fallback to local DB cache
                const cached = await (commands as any).getRemoteModelCatalog();
                if (cached?.status === "ok" && cached.data.length > 0) {
                    setModels(cached.data.map((e: any) => e.metadata as ModelDefinition));
                }
                return;
            }

            const response = await fetch("http://localhost:8000/api/v1/models");
            if (response.ok) {
                const remoteModels: ModelDefinition[] = await response.json();
                setModels(remoteModels);

                // Persist to local DB for offline access
                const entries = remoteModels.map(m => ({
                    id: m.id,
                    name: m.name,
                    metadata: m,
                    local_version: null,
                    remote_version: (m as any).version || "1.0.0",
                    last_checked_at: Math.floor(Date.now() / 1000),
                    status: 'available'
                }));

                await (commands as any).updateRemoteModelCatalog(entries);
            }
        } catch (e) {
            console.warn("Failed to sync remote model catalog:", e);
        }
    }, []);

    useEffect(() => {
        syncRemoteCatalog();
    }, [syncRemoteCatalog]);

    const checkStandardAssets = useCallback(async () => {
        try {
            const result = await commands.checkMissingStandardAssets();
            if (result.status === "error") throw new Error(result.error);
            setStandardAssets(result.data);
        } catch (e) {
            console.error(e);
        }
    }, []);

    useEffect(() => {
        appDataDir().then(dir => {
            invoke("get_models_dir").then((_d: any) => {
                setModelsDir(`${dir}/models`);
            }).catch(() => {
                setModelsDir(`${dir}/models`);
            });
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

    const [currentModelTemplate, _setCurrentModelTemplate] = useState<string>(() => {
        return getMigratedLocalStorageItem('modelTemplate') || "chatml";
    });

    const setModelPath = useCallback((path: string, template?: string) => {
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
        _setCurrentEmbeddingModelPath(path);
        setMigratedLocalStorageItem('embeddingModelPath', path);
    }, []);

    const setVisionModelPath = useCallback((path: string) => {
        _setCurrentVisionModelPath(path);
        setMigratedLocalStorageItem('visionModelPath', path);
    }, []);

    const setSttModelPath = useCallback((path: string) => {
        _setCurrentSttModelPath(path);
        setMigratedLocalStorageItem('sttModelPath', path);
    }, []);

    const setImageGenModelPath = useCallback((path: string) => {
        _setCurrentImageGenModelPath(path);
        setMigratedLocalStorageItem('imageGenModelPath', path);
    }, []);

    const setSummarizerModelPath = useCallback((path: string) => {
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

    const refreshModels = useCallback(async () => {
        setIsRefreshing(true);
        try {
            const models = await invoke<ModelFile[]>("list_models");
            setLocalModels(models);
            return models;
        } catch (e) {
            console.error("Failed to list models", e);
            toast.error("Failed to list models");
            return [];
        } finally {
            setIsRefreshing(false);
        }
    }, []);

    const startDownload = useCallback(async (model: ModelDefinition, variant?: ModelVariant) => {
        const v = variant || (model.variants && model.variants.length > 0 ? model.variants[0] : null);

        if (!v) {
            toast.error("Invalid model selected: No variants found.");
            return;
        }

        // Check for Hugging Face Token if gated
        if (model.gated) {
            try {
                const tokenResult = await commands.getHfToken();
                if (tokenResult.status === "error" || !tokenResult.data) {
                    toast.error("Token Required", {
                        description: "This model requires a Hugging Face token. Please add it in Settings > Secrets.",
                        action: {
                            label: "Open Settings",
                            onClick: () => {
                                window.dispatchEvent(new CustomEvent('open-settings', { detail: 'secrets' }));
                            }
                        },
                        duration: 8000,
                    });
                    return;
                }
            } catch (e) {
                console.error("Failed to check HF token", e);
            }
        }

        // Determine target path: {category}/{model_name_sanitized}/{filename}
        const category = model.category || "LLM";
        const sanitizedName = model.name.replace(/[^a-zA-Z0-9-_]/g, "_");
        const getTargetPath = (filename: string) => `${category}/${sanitizedName}/${filename}`;

        const mainFullPath = getTargetPath(v.filename);

        // Guard against duplicate downloads — check ref buffer too
        if (downloadPctBufferRef.current[mainFullPath] !== undefined) return;

        console.log("Starting download for", mainFullPath);
        setDownloading(prev => ({ ...prev, [mainFullPath]: 0 }));
        toast.info(`Starting download: ${model.name} (${v.name})`);

        try {
            // Components handling (e.g. CLIP/VAE)
            if (model.components) {
                for (const comp of model.components) {
                    const compFullPath = getTargetPath(comp.filename);
                    if (downloadPctBufferRef.current[compFullPath] === undefined) {
                        console.log(`Starting component download: ${comp.filename} -> ${compFullPath}`);
                        setDownloading(prev => ({ ...prev, [compFullPath]: 0 }));
                        invoke("download_model", { url: comp.url, filename: compFullPath }).catch(e => {
                            console.error(`Component download failed: ${comp.filename}`, e);
                            setDownloading(prev => {
                                const c = { ...prev };
                                delete c[compFullPath];
                                return c;
                            });
                        });
                    }
                }
            }

            // Projector handling
            if (model.mmproj) {
                const projFullPath = getTargetPath(model.mmproj.filename);
                if (downloadPctBufferRef.current[projFullPath] === undefined) {
                    setDownloading(prev => ({ ...prev, [projFullPath]: 0 }));
                    invoke("download_model", { url: model.mmproj.url, filename: projFullPath }).catch(e => {
                        console.error("Projector download failed", e);
                        setDownloading(prev => {
                            const c = { ...prev };
                            delete c[projFullPath];
                            return c;
                        });
                    });
                }
            }

            await invoke("download_model", { url: v.url, filename: mainFullPath });
        } catch (e) {
            console.error("Download failed to start:", e);
            toast.error(`Failed to start download: ${e}`);
            setDownloading(prev => {
                const c = { ...prev };
                delete c[mainFullPath];
                return c;
            });
        }
    }, []);

    // Check hardware and recommend model on first empty run
    useEffect(() => {
        const checkHardware = async () => {
            try {
                // Fetch System Specs
                const specs = await commands.getSystemSpecs();
                if (specs) {
                    setSystemSpecs(specs);

                    // Check if we need to recommend
                    const hasChecked = getMigratedLocalStorageItem('firstRunCheck');
                    const localFiles = await refreshModels();

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
                                description: `We recommend ${model.name} for your system (${Math.round(ramGB)}GB RAM).`,
                                action: {
                                    label: "Download",
                                    onClick: () => startDownload(model, model.variants[0])
                                },
                                duration: 10000,
                            });
                        }
                        setMigratedLocalStorageItem('firstRunCheck', "true");
                    }
                }
            } catch (error) {
                console.error("Failed to init system specs:", error);
            }
        };

        checkHardware();

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
    }, [refreshModels, startDownload]);

    // -----------------------------------------------------------------------
    // Throttled progress buffer — prevents per-chunk re-renders of the entire
    // component tree.  Progress events fire many times per second during
    // downloads; we accumulate them in a ref and flush to state at ~4fps.
    // -----------------------------------------------------------------------
    const progressBufferRef = useRef<Record<string, RepoProgressInfo>>({});
    const downloadPctBufferRef = useRef<Record<string, number>>({});
    const progressFlushTimer = useRef<ReturnType<typeof setInterval> | null>(null);

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
            } else {
                // Per-file progress event — update filePct in matching repos
                for (const rp of Object.values(progressBufferRef.current)) {
                    if (rp.currentFile === filename || filename.includes('/')) {
                        continue;
                    }
                    rp.filePct = { ...rp.filePct, [filename]: percentage };
                }
            }

            if (event.payload.percentage >= 100) {
                // Download complete
                console.log("Download complete event for", event.payload.filename);
                // Ensure refresh happens after a slight delay to allow filesystem to settle/close handle
                setTimeout(() => {
                    refreshModels();
                    // Clean up buffers for completed download
                    delete downloadPctBufferRef.current[event.payload.filename];
                    setDownloading(prev => {
                        const copy = { ...prev };
                        delete copy[event.payload.filename];
                        return copy;
                    });
                    // Clean up discovery state for completed repo downloads
                    if (event.payload.filename.includes('/')) {
                        setTimeout(() => {
                            // Flush any remaining buffer for this repo first
                            delete progressBufferRef.current[event.payload.filename];
                            setDiscoveryState(prev => {
                                const rp = { ...prev.repoProgress };
                                delete rp[event.payload.filename];
                                const df = new Set(prev.downloadingFiles);
                                df.delete(event.payload.filename);
                                return { ...prev, repoProgress: rp, downloadingFiles: df };
                            });
                        }, 1500);
                    }
                    toast.success(`Download complete: ${event.payload.filename} `);
                }, 1000);
            }
        });

        return () => {
            unlisten.then(f => f());
        }
    }, [refreshModels]);


    const cancelDownload = useCallback(async (filename: string) => {
        try {
            await invoke("cancel_download", { filename });
            // Also try cancelling potential mmproj
            await invoke("cancel_download", { filename: `${filename}.mmproj` });
            toast.info("Download cancelled");
        } catch (e) {
            console.warn("Backend cancel failed (task might be finished):", e);
        } finally {
            delete downloadPctBufferRef.current[filename];
            setDownloading(prev => {
                const copy = { ...prev };
                delete copy[filename];
                return copy;
            });
        }
    }, []);

    const deleteModel = useCallback(async (filename: string) => {
        try {
            await invoke("delete_local_model", { filename });
            toast.success("Model deleted");
            await refreshModels();
        } catch (e) {
            console.error("Delete failed:", e);
            toast.error(`Failed to delete: ${e} `);
        }
    }, [refreshModels]);

    // Download files from HuggingFace Hub (shared via context)
    const downloadHfFiles = useCallback(async (repoId: string, files: string[], destSubdir?: string | null, category?: string) => {
        // Track each file in the global download state
        const trackKey = files.length === 1 ? files[0] : repoId;
        setDownloading(prev => ({ ...prev, [trackKey]: 0 }));

        try {
            unwrapResult(
                await directCommands.directRuntimeDownloadHfModelFiles(
                    repoId,
                    files,
                    destSubdir ?? null,
                    category ?? null
                ),
                "HuggingFace model download"
            );
            toast.success(`Downloaded: ${files.length === 1 ? files[0] : repoId}`);
            refreshModels();
        } catch (e: any) {
            const msg = typeof e === "string" ? e : "Download failed";
            toast.error(msg);
        } finally {
            delete downloadPctBufferRef.current[trackKey];
            setDownloading(prev => {
                const copy = { ...prev };
                delete copy[trackKey];
                return copy;
            });
        }
    }, [refreshModels]);

    const downloadStandardAsset = useCallback(async (filename: string) => {
        if (downloading[filename]) return;
        setDownloading(prev => ({ ...prev, [filename]: 0 }));
        toast.info(`Downloading Standard Asset: ${filename}`);
        try {
            await commands.downloadStandardAsset(filename);
        } catch (e) {
            toast.error(`Standard Asset Download Failed: ${e}`);
            setDownloading(prev => {
                const c = { ...prev };
                delete c[filename];
                return c;
            });
        }
    }, [downloading]);

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
        startDownload,
        downloadSpeed,
        selectModel,
        activeCategory,
        setActiveCategory,
        cancelDownload,
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
        downloadHfFiles,
        engineInfo,
        runtimeSnapshot,
        refreshRuntimeSnapshot,
    }), [
        models, localModels, currentModelPath,
        currentEmbeddingModelPath, currentVisionModelPath, currentSttModelPath,
        currentImageGenModelPath, currentSummarizerModelPath, currentModelTemplate,
        setModelPath, setEmbeddingModelPath, setVisionModelPath, setSttModelPath,
        setImageGenModelPath, setSummarizerModelPath, refreshModels, startDownload,
        downloadSpeed, selectModel, activeCategory, cancelDownload, deleteModel,
        isRefreshing, modelsDir, systemSpecs, standardAssets, checkStandardAssets,
        downloadStandardAsset, maxContext, isRestarting, downloadHfFiles, engineInfo,
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
