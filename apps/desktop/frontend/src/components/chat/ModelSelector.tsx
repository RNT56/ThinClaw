import { useState, useEffect, useRef, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { useModelContext, RECOMMENDED_MODELS } from '../model-context';
import { ChevronDown, Check, Box, Sparkles, Cloud, Monitor } from 'lucide-react';
import { commands } from '../../lib/bindings';
import { cn } from '../../lib/utils';
import { useConfig } from '../../hooks/use-config';
import { useCloudModels } from '../../hooks/use-cloud-models';

export function ModelSelector({ onManageClick, isAutoMode, toggleAutoMode }: { onManageClick: () => void, isAutoMode: boolean, toggleAutoMode: (v: boolean) => void }) {
    const { localModels, currentModelPath: modelPath, setModelPath, downloading, setIsRestarting } = useModelContext();
    const [isOpen, setIsOpen] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);
    const [status, setStatus] = useState<any>(null);
    const { config, updateConfig } = useConfig();

    useEffect(() => {
        const loadStatus = async () => {
            try {
                const s = await commands.thinclawGetStatus();
                if (s.status === 'ok') setStatus(s.data);
            } catch (e) {
                console.error("Failed to load status in ModelSelector", e);
            }
        };
        loadStatus();
    }, [isOpen]);

    // Filter out dedicated embedding/STT/diffusion/TTS models from the chat selector.
    // Uses BOTH path-based category detection (models/{Category}/...) and filename heuristics.
    const filteredLocal = localModels.filter(m => {
        // Exclude if currently downloading
        if (downloading[m.name]) return false;

        // --- Path-based category exclusion (most reliable) ---
        // Models downloaded via HF discovery go into category subdirectories:
        //   models/Embedding/..., models/STT/..., models/Diffusion/..., models/TTS/...
        // The `name` field is the relative path from models/ (e.g. "STT/mlx-community_whisper-large-v3-turbo")
        const namePath = m.name.replace(/\\/g, '/');
        if (namePath.startsWith('Embedding/') || namePath.startsWith('embedding/')) return false;
        if (namePath.startsWith('STT/') || namePath.startsWith('stt/')) return false;
        if (namePath.startsWith('Diffusion/') || namePath.startsWith('diffusion/')) return false;
        if (namePath.startsWith('TTS/') || namePath.startsWith('tts/')) return false;

        // Also check the absolute path for category folders
        const pathLower = m.path.replace(/\\/g, '/').toLowerCase();
        if (pathLower.includes('/models/embedding/')) return false;
        if (pathLower.includes('/models/stt/')) return false;
        if (pathLower.includes('/models/diffusion/')) return false;
        if (pathLower.includes('/models/tts/')) return false;

        const filename = m.name.split(/[\\/]/).pop() || m.name;
        const known = RECOMMENDED_MODELS.find(k => k.variants?.some(v => v.filename === filename));

        // If known, strictly check tags
        if (known) {
            const isExcluded = known.tags?.some(t => ['Embedding', 'STT', 'Image Gen'].includes(t));
            if (isExcluded) return false;
            return true;
        }

        // Fallback for non-curated or local models (filename heuristics)
        const nameLower = filename.toLowerCase();

        // Exclude embeddings
        if (nameLower.includes('embed') || nameLower.includes('nomic-') || nameLower.includes('bge-')) return false;

        // Exclude Image Gen
        if (nameLower.includes('diffusion') || nameLower.includes('flux') || nameLower.includes('sd-') || nameLower.includes('sdxl') || nameLower.includes('stable-diffusion') || nameLower.includes('v1-5')) return false;

        // Exclude STT / Whisper / TTS
        if (nameLower.includes('whisper') || nameLower.includes('ggml') || nameLower.includes('stt') || nameLower.includes('tts')) return false;

        // Exclude specific components
        if (nameLower.includes('vae') || nameLower.includes('clip') || nameLower.includes('t5xxl') || nameLower.includes('mmproj')) return false;

        return true;
    });
    // Unified provider lookup used across cloud model filtering, selection, and badge rendering
    const PROVIDER_LOOKUP: [string, string][] = [
        ["openrouter-", "openrouter"], ["groq-", "groq"],
        ["anthropic-", "anthropic"], ["openai-", "openai"],
        ["google-", "gemini"], ["gemini-", "gemini"],
        ["mistral-", "mistral"], ["codestral-", "mistral"],
        ["xai-", "xai"], ["together-", "together"],
        ["venice-", "venice"], ["cohere-", "cohere"],
        ["moonshot-", "moonshot"], ["minimax-", "minimax"],
        ["nvidia-", "nvidia"], ["xiaomi-", "xiaomi"],
    ];

    const resolveProvider = (id: string, fallbackFamily?: string): string => {
        const lower = id.toLowerCase();
        const match = PROVIDER_LOOKUP.find(([p]) => lower.startsWith(p));
        return match ? match[1] : (fallbackFamily?.toLowerCase() ?? "");
    };

    const hasKeyForProvider = (provider: string): boolean => {
        if (!status) return false;
        const s = status as any;
        // Core 5 providers have dedicated status keys
        const coreKeys: Record<string, boolean> = {
            anthropic: !!(s.has_anthropic_key || s.hasAnthropicKey),
            openai: !!(s.has_openai_key || s.hasOpenaiKey),
            gemini: !!(s.has_gemini_key || s.hasGeminiKey),
            groq: !!(s.has_groq_key || s.hasGroqKey),
            openrouter: !!(s.has_openrouter_key || s.hasOpenrouterKey),
        };
        if (provider in coreKeys) return coreKeys[provider];
        // Implicit providers
        const camel = provider.charAt(0).toUpperCase() + provider.slice(1);
        return !!(s[`has_${provider}_key`] || s[`has${camel}Key`]);
    };

    const cloudModels = RECOMMENDED_MODELS.filter(m => {
        if ((m as any).category !== "Cloud") return false;
        const provider = resolveProvider(m.id, m.family);
        if (config?.disabled_providers?.includes(provider)) return false;
        return hasKeyForProvider(provider);
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
        ...filteredLocal.map(m => ({ ...m, type: 'local' as const })),
        ...allCloudModels,
    ];

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
                const modelId = modelDef.id.split('-').slice(1).join('-');

                // Propagate the discovered model's context window to the backend.
                // `_contextWindow` is set for cloud-discovered models; null for hardcoded fallback entries.
                const contextSize = (modelDef as any)._contextWindow as number | null;

                const newConfig = {
                    ...config,
                    selected_chat_provider: brain,
                    selected_model_context_size: contextSize ?? undefined,
                };

                await updateConfig(newConfig);
                if (commands.thinclawSaveSelectedCloudModel) {
                    await commands.thinclawSaveSelectedCloudModel(modelId);
                }

                // Refresh status to reflect new cloud model
                const s = await commands.thinclawGetStatus();
                if (s.status === 'ok') setStatus(s.data);

                setIsOpen(false);
                // We don't setModelPath for cloud models yet as it's handled by provider routing in backend
                // but we might want to update local UI state if needed.
                return;
            } catch (e) {
                console.error(e);
            }
        }

        if (path === modelPath && config?.selected_chat_provider === "local") {
            setIsRestarting(false);
            setIsOpen(false);
            return;
        }

        // Trigger immediate UI block
        setIsRestarting(true);

        // If switching from cloud to local but path is same, we need to force a trigger.
        // We'll update the config first.
        if (type === 'local' && config?.selected_chat_provider !== "local") {
            try {
                const newConfig = { ...config, selected_chat_provider: "local" };
                await updateConfig(newConfig);
                // Refresh status to ensure consistency
                const s = await commands.thinclawGetStatus();
                if (s.status === 'ok') setStatus(s.data);
            } catch (e) {
                console.error("Failed to update config to local", e);
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
                className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-background/60 hover:bg-background/80 text-sm font-medium transition-colors border border-input/50 backdrop-blur-xl shadow-sm"
            >
                <div className={cn("inline-flex items-center gap-2", isAutoMode && "text-yellow-500 font-bold")}>
                    {isAutoMode ? <Box className="w-4 h-4" /> : <Box className="w-4 h-4 text-primary" />}
                    <span className="max-w-[150px] truncate">
                        {isAutoMode ? "Auto Mode" : (
                            config?.selected_chat_provider && config.selected_chat_provider !== "local"
                                ? status?.selected_cloud_model || (config.selected_chat_provider.toUpperCase())
                                : (localModels.find(m => m.path === modelPath)?.name.split(/[\\/]/).pop()) || "Select Model"
                        )}
                    </span>
                    {/* Local / Cloud badge */}
                    {!isAutoMode && (
                        config?.selected_chat_provider && config.selected_chat_provider !== "local"
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
                                    {models.map((model: any) => {
                                        const filename = model.type === 'local' ? (model.name.split(/[\\/]/).pop() || model.name) : model.name;
                                        const def = RECOMMENDED_MODELS.find(k => k.variants?.some(v => v.filename === filename) || k.id === model.id);
                                        const isRecommended = def?.recommendedForAgent;
                                        const provider = resolveProvider(model.id || '', model.family);

                                        const isActive = model.type === 'local'
                                            ? (model.path === modelPath && config?.selected_chat_provider === "local")
                                            : (config?.selected_chat_provider === provider && status?.selected_cloud_model === model.id?.split('-').slice(1).join('-'));

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
