import { useState, useEffect, useRef, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { useModelContext, RECOMMENDED_MODELS } from '../model-context';
import { ChevronDown, Check, Box, Sparkles, Cloud, Monitor, RefreshCw } from 'lucide-react';
import { cn } from '../../lib/utils';
import { useConfig } from '../../hooks/use-config';
import { useCloudModels } from '../../hooks/use-cloud-models';
import { useInferenceBackends } from '../../hooks/use-inference-backends';
import { isCompatibleManagedModelForCategory } from '../../lib/hf-models';
import { toast } from 'sonner';
import { useOllamaModels } from '../../hooks/use-ollama-models';
import { chooseInstalledOllamaModel } from '../../lib/ollama-models';

export function ModelSelector({ onManageClick, isAutoMode, toggleAutoMode }: { onManageClick: () => void, isAutoMode: boolean, toggleAutoMode: (v: boolean) => void }) {
    const {
        localModels,
        currentModelPath: modelPath,
        setModelPath,
        downloading,
        setIsRestarting,
        engineInfo,
    } = useModelContext();
    const [isOpen, setIsOpen] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);
    const { config, updateConfig } = useConfig();
    const { available } = useInferenceBackends();

    // The backend inventory owns category and runtime compatibility. This
    // deliberately avoids filename guesses such as treating a chat model with
    // "embed" in its name as an embedding model.
    const isOllama = engineInfo?.id === 'ollama';
    const {
        models: ollamaModels,
        status: ollamaModelsStatus,
        error: ollamaModelsError,
        refresh: refreshOllamaModels,
    } = useOllamaModels(isOllama);
    const filteredLocal = localModels.filter(m =>
        !isOllama
        &&
        isCompatibleManagedModelForCategory(m, 'LLM')
        && downloading[m.name] === undefined
        && downloading[m.id] === undefined
    );
    const ollamaLocalModels = ollamaModels.map(id => ({
        id,
        path: id,
        name: id,
        family: 'Ollama',
        type: 'local' as const,
    }));

    useEffect(() => {
        if (!isOllama || ollamaModelsStatus !== 'ready') return;
        const selected = chooseInstalledOllamaModel(ollamaModels, modelPath);
        if ((selected ?? '') !== modelPath) {
            setModelPath(selected ?? '');
        }
    }, [
        isOllama,
        modelPath,
        ollamaModels,
        ollamaModelsStatus,
        setModelPath,
    ]);
    // Unified provider lookup used across cloud model filtering, selection, and badge rendering
    const PROVIDER_LOOKUP: [string, string][] = [
        ["openrouter-", "openrouter"], ["groq-", "groq"],
        ["anthropic-", "anthropic"], ["openai-", "openai"],
        ["google-", "gemini"], ["gemini-", "gemini"],
        ["mistral-", "mistral"], ["codestral-", "mistral"],
        ["xai-", "xai"], ["together-", "together"],
        ["venice-", "venice"], ["cohere-", "cohere"],
        ["moonshot-", "moonshot"], ["minimax-", "minimax"],
        ["nvidia-", "nvidia"],
    ];

    const resolveProvider = (id: string, fallbackFamily?: string): string => {
        const lower = id.toLowerCase();
        const match = PROVIDER_LOOKUP.find(([p]) => lower.startsWith(p));
        return match ? match[1] : (fallbackFamily?.toLowerCase() ?? "");
    };

    const availableChatProviders = useMemo(() => new Set(
        (available.chat ?? [])
            .filter(backend => backend.available)
            .map(backend => backend.id)
    ), [available.chat]);

    const cloudModels = RECOMMENDED_MODELS.filter(m => {
        if ((m as any).category !== "Cloud") return false;
        const provider = resolveProvider(m.id, m.family);
        if (config?.disabled_providers?.includes(provider)) return false;
        return availableChatProviders.has(provider);
    });

    // ── Merge cloud-discovered chat models ──────────────────────────────
    const { models: discoveredModels } = useCloudModels();

    const allCloudModels = useMemo(() => {
        const hardcodedIds = new Set(cloudModels.map(m => m.id.toLowerCase()));

        const discovered = discoveredModels
            .filter(cm => {
                if (cm.category !== 'chat') return false;
                // Deduplicate
                const fullId = `${cm.provider}-${cm.id}`.toLowerCase();
                return !hardcodedIds.has(fullId) && !hardcodedIds.has(cm.id.toLowerCase());
            })
            .map(cm => ({
                path: `${cm.provider}-${cm.id}`,
                name: cm.displayName,
                type: 'cloud' as const,
                family: cm.providerName,
                id: `${cm.provider}-${cm.id}`,
                _contextWindow: cm.contextWindow,
                _pricing: cm.pricing,
            }));

        const hardcoded = cloudModels.map(m => ({
            path: m.id,
            name: m.name,
            type: 'cloud' as const,
            family: m.family,
            id: m.id,
            _contextWindow: null as number | null,
            _pricing: null as any,
        }));

        return [...hardcoded, ...discovered];
    }, [cloudModels, discoveredModels]);

    const models = [
        ...ollamaLocalModels,
        ...filteredLocal.map(m => ({ ...m, type: 'local' as const })),
        ...allCloudModels,
    ];

    const selectedChatProvider = config?.chat_backend ?? config?.selected_chat_provider ?? "local";
    const selectedChatModel = config?.inference_models?.chat ?? null;

    const modelIdForProvider = (id: string, provider: string): string => {
        const providerPrefix = `${provider}-`;
        if (id.toLowerCase().startsWith(providerPrefix)) {
            return id.slice(providerPrefix.length);
        }
        return id.split('-').slice(1).join('-') || id;
    };

    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
                setIsOpen(false);
            }
        };
        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, []);

    const handleSelect = async (path: string, type: 'local' | 'cloud') => {
        if (path === "auto") {
            toggleAutoMode(!isAutoMode);
            setIsOpen(false);
            return;
        }

        if (type === 'cloud') {
            try {
                const modelDef = cloudModels.find(m => m.id === path)
                    || allCloudModels.find(m => m.id === path);
                if (!modelDef) return;

                const brain = resolveProvider(modelDef.id, modelDef.family);
                const modelId = modelIdForProvider(modelDef.id, brain);

                // Propagate the discovered model's context window to the backend.
                // `_contextWindow` is set for cloud-discovered models; null for hardcoded fallback entries.
                const contextSize = (modelDef as any)._contextWindow as number | null;

                const newConfig = {
                    ...config,
                    selected_chat_provider: brain,
                    chat_backend: brain,
                    inference_models: {
                        ...(config?.inference_models ?? {}),
                        chat: modelId,
                    },
                    selected_model_context_size: contextSize,
                };

                await updateConfig(newConfig);

                setIsOpen(false);
                return;
            } catch (e) {
                console.error(e);
                toast.error("Could not switch cloud model");
                return;
            }
        }

        if (path === modelPath && selectedChatProvider === "local") {
            setIsRestarting(false);
            setIsOpen(false);
            return;
        }

        // Trigger immediate UI block
        setIsRestarting(true);

        // If switching from cloud to local but path is same, we need to force a trigger.
        // We'll update the config first.
        if (type === 'local' && selectedChatProvider !== "local") {
            try {
                const newConfig = { ...config, selected_chat_provider: "local", chat_backend: "local" };
                await updateConfig(newConfig);
            } catch (e) {
                console.error("Failed to update config to local", e);
                setIsRestarting(false);
                toast.error("Could not switch to local inference");
                return;
            }
        }

        // Update path, letting useAutoStart handle the actual server start
        setModelPath(path);
        setIsOpen(false);
    };



    return (
        <div className="relative inline-block text-left" ref={containerRef}>
            <button
                onClick={() => setIsOpen(!isOpen)}
                className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-background/60 hover:bg-background/80 text-sm font-medium transition-colors border border-input/50 backdrop-blur-xl shadow-xs"
            >
                <div className={cn("inline-flex items-center gap-2", isAutoMode && "text-yellow-500 font-bold")}>
                    {isAutoMode ? <Box className="w-4 h-4" /> : <Box className="w-4 h-4 text-primary" />}
                    <span className="max-w-[150px] truncate">
                        {isAutoMode ? "Auto Mode" : (
                            selectedChatProvider !== "local"
                                ? selectedChatModel || (selectedChatProvider.toUpperCase())
                                : (
                                    isOllama && ollamaModels.includes(modelPath)
                                        ? modelPath
                                        : localModels.find(m => m.path === modelPath)?.name.split(/[\\/]/).pop()
                                ) || "Select Model"
                        )}
                    </span>
                    {/* Local / Cloud badge */}
                    {!isAutoMode && (
                        selectedChatProvider !== "local"
                            ? <Cloud className="w-3 h-3 text-blue-500 shrink-0" />
                            : <Monitor className="w-3 h-3 text-emerald-500 shrink-0" />
                    )}
                </div>
                <ChevronDown className={cn("w-3 h-3 transition-transform opacity-50", isOpen && "rotate-180")} />
            </button>

            <AnimatePresence>
                {isOpen && (
                    <motion.div
                        initial={{ opacity: 0, y: -10, x: "-50%", scale: 0.95 }}
                        animate={{ opacity: 1, y: 0, x: "-50%", scale: 1 }}
                        exit={{ opacity: 0, y: -10, x: "-50%", scale: 0.95 }}
                        transition={{ duration: 0.2, ease: "easeOut" }}
                        className="absolute top-full mt-1 left-1/2 w-64 origin-top bg-card/90 backdrop-blur-xl border border-border/50 rounded-xl shadow-xl z-50 overflow-hidden"
                    >
                        <div className="p-1 max-h-[300px] overflow-y-auto scrollbar-hide py-2">
                            {models.length === 0 ? (
                                <div className="px-4 py-3 text-xs text-muted-foreground text-center">No models found</div>
                            ) : (
                                <>
                                    <button
                                        onClick={() => handleSelect("auto", "local")}
                                        className="w-full text-left px-3 py-2 text-sm rounded-lg flex items-center gap-2 hover:bg-accent text-foreground group transition-colors mb-1 border-b border-border/50 pb-2"
                                    >
                                        <div className="p-1 bg-yellow-500/10 rounded flex items-center justify-center">
                                            <Box className="w-3 h-3 text-yellow-500" />
                                        </div>
                                        <span className="truncate flex-1 font-medium text-yellow-600 dark:text-yellow-400">Auto Mode</span>
                                        {isAutoMode && <Check className="w-3.5 h-3.5 shrink-0 text-yellow-500" />}
                                    </button>
                                    {isOllama && (
                                        <div className="mx-2 mb-1 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-2 text-[11px] text-muted-foreground">
                                            <div className="flex items-center justify-between gap-2">
                                                <span>
                                                    {ollamaModelsStatus === 'loading'
                                                        ? 'Reading your Ollama library…'
                                                        : ollamaModelsError
                                                            ? ollamaModelsError
                                                            : ollamaModels.length === 0
                                                                ? 'No Ollama models installed. Run ollama pull <model>.'
                                                                : `${ollamaModels.length} model${ollamaModels.length === 1 ? '' : 's'} installed in Ollama`}
                                                </span>
                                                <button
                                                    type="button"
                                                    onClick={() => void refreshOllamaModels()}
                                                    disabled={ollamaModelsStatus === 'loading'}
                                                    className="shrink-0 rounded p-1 hover:bg-accent disabled:opacity-50"
                                                    aria-label="Refresh installed Ollama models"
                                                >
                                                    <RefreshCw className={cn(
                                                        "h-3.5 w-3.5",
                                                        ollamaModelsStatus === 'loading' && "animate-spin",
                                                    )} />
                                                </button>
                                            </div>
                                        </div>
                                    )}
                                    {models.map((model: any) => {
                                        const filename = model.type === 'local' ? (model.name.split(/[\\/]/).pop() || model.name) : model.name;
                                        const def = RECOMMENDED_MODELS.find(k => k.variants?.some(v => v.filename === filename) || k.id === model.id);
                                        const isRecommended = def?.recommendedForAgent;
                                        const provider = resolveProvider(model.id || '', model.family);

                                        const isActive = model.type === 'local'
                                            ? (model.path === modelPath && selectedChatProvider === "local")
                                            : (selectedChatProvider === provider && selectedChatModel === modelIdForProvider(model.id, provider));

                                        return (
                                            <button
                                                key={model.path}
                                                onClick={() => handleSelect(model.path, model.type)}
                                                className={cn(
                                                    "w-full text-left px-3 py-2 text-sm rounded-lg flex items-center justify-between group transition-colors",
                                                    isActive ? "bg-primary/10 text-primary font-bold" : "hover:bg-accent text-foreground"
                                                )}
                                            >
                                                <div className="flex items-center gap-2 truncate flex-1 mr-2">
                                                    <span className="truncate">{filename}</span>
                                                    {model.type === 'cloud' && <span className="text-[9px] bg-indigo-500/10 text-indigo-500 px-1 rounded border border-indigo-500/20 uppercase font-bold">{model.family}</span>}
                                                    {isRecommended && <Sparkles className="w-3 h-3 text-yellow-500 shrink-0" />}
                                                </div>
                                                {isActive && <Check className="w-3.5 h-3.5 shrink-0" />}
                                            </button>
                                        );
                                    })
                                    }
                                </>
                            )}
                            <div className="border-t border-border/50 my-1 mx-2"></div>
                            <button
                                className="w-full text-left px-3 py-2 text-xs text-muted-foreground hover:text-foreground hover:bg-accent/50 rounded-lg transition-colors flex items-center gap-2"
                                onClick={() => {
                                    setIsOpen(false);
                                    onManageClick();
                                }}
                            >
                                Manage Models...
                            </button>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
