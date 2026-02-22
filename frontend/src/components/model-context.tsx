import React, { createContext, useContext, useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { appDataDir } from "@tauri-apps/api/path";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { ModelFile, SystemSpecs, commands, StandardAsset } from "../lib/bindings";

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
}

const ModelContext = createContext<ModelContextType | undefined>(undefined);

const STORAGE_KEY = "scrappy_model_path";
const EMBEDDING_STORAGE_KEY = "scrappy_embedding_model_path";
const VISION_STORAGE_KEY = "scrappy_vision_model_path";
const STT_STORAGE_KEY = "scrappy_stt_model_path";
const IMAGE_GEN_STORAGE_KEY = "scrappy_image_gen_model_path";
const SUMMARIZER_STORAGE_KEY = "scrappy_summarizer_model_path";
const TEMPLATE_STORAGE_KEY = "scrappy_model_template";
const MAX_CONTEXT_STORAGE_KEY = "scrappy_max_context";
const DEFAULT_PATH = "";
const FIRST_RUN_KEY = "scrappy_first_run_check_v3"; // Bumped version to re-trigger if needed

export function ModelProvider({ children }: { children: React.ReactNode }) {
    const [localModels, setLocalModels] = useState<ModelFile[]>([]);
    const [downloading, setDownloading] = useState<Record<string, number>>({});
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [isRestarting, setIsRestarting] = useState(false);
    const [systemSpecs, setSystemSpecs] = useState<SystemSpecs | null>(null);
    const [_currentModel, setCurrentModel] = useState<ModelDefinition | null>(null);
    const [_downloadedModels, _setDownloadedModels] = useState<string[]>([]);
    const [activeCategory, setActiveCategory] = useState("Chat");
    const [modelsDir, setModelsDir] = useState<string | null>(null);
    const [downloadSpeed, _setDownloadSpeed] = useState("");
    const [standardAssets, setStandardAssets] = useState<StandardAsset[]>([]);
    const [models, setModels] = useState<ModelDefinition[]>(MODEL_LIBRARY);

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

    // Helper to check downloaded status (Mock for now or rely on localModels)
    // const _checkDownloadedModels = useCallback(async () => {
    //     // Logic mainly relies on localModels from refreshModels
    // }, []);

    const selectModel = (modelId: string) => {
        const model = models.find(m => m.id === modelId);
        if (model) setCurrentModel(model);
    };

    // Model Selection State
    const [currentModelPath, _setCurrentModelPath] = useState<string>(() => {
        return localStorage.getItem(STORAGE_KEY) || DEFAULT_PATH;
    });

    const [currentEmbeddingModelPath, _setCurrentEmbeddingModelPath] = useState<string>(() => {
        return localStorage.getItem(EMBEDDING_STORAGE_KEY) || DEFAULT_PATH;
    });

    const [currentVisionModelPath, _setCurrentVisionModelPath] = useState<string>(() => {
        return localStorage.getItem(VISION_STORAGE_KEY) || DEFAULT_PATH;
    });

    const [currentSttModelPath, _setCurrentSttModelPath] = useState<string>(() => {
        return localStorage.getItem(STT_STORAGE_KEY) || DEFAULT_PATH;
    });

    const [currentImageGenModelPath, _setCurrentImageGenModelPath] = useState<string>(() => {
        return localStorage.getItem(IMAGE_GEN_STORAGE_KEY) || DEFAULT_PATH;
    });

    const [currentSummarizerModelPath, _setCurrentSummarizerModelPath] = useState<string>(() => {
        return localStorage.getItem(SUMMARIZER_STORAGE_KEY) || DEFAULT_PATH;
    });

    const [currentModelTemplate, _setCurrentModelTemplate] = useState<string>(() => {
        return localStorage.getItem(TEMPLATE_STORAGE_KEY) || "chatml";
    });

    const setModelPath = (path: string, template?: string) => {
        _setCurrentModelPath(path);
        localStorage.setItem(STORAGE_KEY, path);
        if (template) {
            _setCurrentModelTemplate(template);
            localStorage.setItem(TEMPLATE_STORAGE_KEY, template);
        } else {
            // Heuristic if not provided (e.g. local scan)
            let inferred = "chatml";
            const lower = path.toLowerCase();
            if (lower.includes("llama-3") || lower.includes("llama3")) inferred = "llama3";
            else if (lower.includes("mistral") || lower.includes("mixtral")) inferred = "mistral";
            else if (lower.includes("gemma")) inferred = "gemma";
            else if (lower.includes("qwen")) inferred = "qwen";

            _setCurrentModelTemplate(inferred);
            localStorage.setItem(TEMPLATE_STORAGE_KEY, inferred);
        }
    };

    const setEmbeddingModelPath = (path: string) => {
        _setCurrentEmbeddingModelPath(path);
        localStorage.setItem(EMBEDDING_STORAGE_KEY, path);
    };

    const setVisionModelPath = (path: string) => {
        _setCurrentVisionModelPath(path);
        localStorage.setItem(VISION_STORAGE_KEY, path);
    };

    const setSttModelPath = (path: string) => {
        _setCurrentSttModelPath(path);
        localStorage.setItem(STT_STORAGE_KEY, path);
    };

    const setImageGenModelPath = (path: string) => {
        _setCurrentImageGenModelPath(path);
        localStorage.setItem(IMAGE_GEN_STORAGE_KEY, path);
    };

    const setSummarizerModelPath = (path: string) => {
        _setCurrentSummarizerModelPath(path);
        localStorage.setItem(SUMMARIZER_STORAGE_KEY, path);
    };

    const [maxContext, _setMaxContext] = useState<number>(() => {
        const stored = localStorage.getItem(MAX_CONTEXT_STORAGE_KEY);
        return stored ? parseInt(stored) : 32768; // Default to 32k
    });

    const setMaxContext = (size: number) => {
        _setMaxContext(size);
        localStorage.setItem(MAX_CONTEXT_STORAGE_KEY, size.toString());
    };

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

        if (downloading[mainFullPath]) return;

        console.log("Starting download for", mainFullPath);
        setDownloading(prev => ({ ...prev, [mainFullPath]: 0 }));
        toast.info(`Starting download: ${model.name} (${v.name})`);

        try {
            // Components handling (e.g. CLIP/VAE)
            if (model.components) {
                for (const comp of model.components) {
                    const compFullPath = getTargetPath(comp.filename);
                    if (!downloading[compFullPath]) {
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
                if (!downloading[projFullPath]) {
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
    }, [downloading]);

    // Check hardware and recommend model on first empty run
    useEffect(() => {
        const checkHardware = async () => {
            try {
                // Fetch System Specs
                const specs = await commands.getSystemSpecs();
                if (specs) {
                    setSystemSpecs(specs);

                    // Check if we need to recommend
                    const hasChecked = localStorage.getItem(FIRST_RUN_KEY);
                    const localFiles = await refreshModels();

                    if (!hasChecked && localFiles.length === 0) {
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
                        localStorage.setItem(FIRST_RUN_KEY, "true");
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

    // Listen for download progress globally
    useEffect(() => {
        const unlisten = listen<DownloadEvent>("download_progress", (event) => {
            console.log("Received download progress:", event.payload.filename, event.payload.percentage);
            setDownloading(prev => ({
                ...prev,
                [event.payload.filename]: event.payload.percentage
            }));

            if (event.payload.percentage >= 100) {
                // Download complete
                console.log("Download complete event for", event.payload.filename);
                // Ensure refresh happens after a slight delay to allow filesystem to settle/close handle
                setTimeout(() => {
                    refreshModels();
                    setDownloading(prev => {
                        const copy = { ...prev };
                        delete copy[event.payload.filename];
                        return copy;
                    });
                    toast.success(`Download complete: ${event.payload.filename} `);
                }, 1000);
            }
        });

        return () => {
            unlisten.then(f => f());
        }
    }, [refreshModels]);


    const cancelDownload = async (filename: string) => {
        try {
            await invoke("cancel_download", { filename });
            // Also try cancelling potential mmproj
            await invoke("cancel_download", { filename: `${filename}.mmproj` });
            toast.info("Download cancelled");
        } catch (e) {
            console.warn("Backend cancel failed (task might be finished):", e);
        } finally {
            setDownloading(prev => {
                const copy = { ...prev };
                delete copy[filename];
                return copy;
            });
        }
    };

    const deleteModel = async (filename: string) => {
        try {
            await invoke("delete_local_model", { filename });
            toast.success("Model deleted");
            await refreshModels();
        } catch (e) {
            console.error("Delete failed:", e);
            toast.error(`Failed to delete: ${e} `);
        }
    };

    return (
        <ModelContext.Provider value={{
            models,
            localModels,
            downloading,
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
            downloadStandardAsset: async (filename: string) => {
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
            },
            maxContext,
            setMaxContext,
            isRestarting,
            setIsRestarting
        }}>
            {children}
        </ModelContext.Provider>
    );
}

export function useModelContext() {
    const context = useContext(ModelContext);
    if (!context) throw new Error("useModelContext must be used within ModelProvider");
    return context;
}
