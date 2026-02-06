import { useState, useEffect, useRef } from 'react';
import { useModelContext, RECOMMENDED_MODELS } from '../model-context';
import { ChevronDown, Check, Box, Sparkles } from 'lucide-react';
import { commands } from '../../lib/bindings';
import { cn } from '../../lib/utils';

export function ModelSelector({ onManageClick, isAutoMode, toggleAutoMode }: { onManageClick: () => void, isAutoMode: boolean, toggleAutoMode: (v: boolean) => void }) {
    const { localModels, currentModelPath: modelPath, setModelPath, downloading, setIsRestarting } = useModelContext();
    const [isOpen, setIsOpen] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);
    const [status, setStatus] = useState<any>(null);
    const [config, setConfig] = useState<any>(null);

    useEffect(() => {
        const loadStatus = async () => {
            try {
                const [s, cfg] = await Promise.all([
                    commands.getClawdbotStatus(),
                    commands.getUserConfig()
                ]);
                if (s.status === 'ok') setStatus(s.data);
                setConfig(cfg);
            } catch (e) {
                console.error("Failed to load status in ModelSelector", e);
            }
        };
        loadStatus();
    }, [isOpen]);

    // Filter out dedicated embedding models AND downloading models AND image gen models
    const filteredLocal = localModels.filter(m => {
        // Exclude if currently downloading
        if (downloading[m.name]) return false;

        const filename = m.name.split(/[\\/]/).pop() || m.name;
        const known = RECOMMENDED_MODELS.find(k => k.variants?.some(v => v.filename === filename));

        // If known, strictly check tags
        if (known) {
            const isExcluded = known.tags?.some(t => ['Embedding', 'STT', 'Image Gen'].includes(t));
            if (isExcluded) return false;
            return true;
        }

        // Fallback for non-curated or local models
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
    const cloudModels = RECOMMENDED_MODELS.filter(m => {
        if ((m as any).category !== "Cloud") return false;

        const id = m.id.toLowerCase();
        const family = m.family.toLowerCase();

        // Check if disabled in config
        if (config?.disabled_providers?.includes(family)) return false;

        // Determine provider by ID prefix first (robust for aggregators)
        if (id.startsWith("openrouter-")) return !!(status?.has_openrouter_key || status?.hasOpenrouterKey);
        if (id.startsWith("groq-")) return !!(status?.has_groq_key || status?.hasGroqKey);
        if (id.startsWith("anthropic-")) return !!(status?.has_anthropic_key || status?.hasAnthropicKey);
        if (id.startsWith("openai-")) return !!(status?.has_openai_key || status?.hasOpenaiKey);
        if (id.startsWith("google-") || id.startsWith("gemini-")) return !!(status?.has_gemini_key || status?.hasGeminiKey);

        // Fallback to family-based check
        if (family === "anthropic") return !!(status?.has_anthropic_key || status?.hasAnthropicKey);
        if (family === "openai") return !!(status?.has_openai_key || status?.hasOpenaiKey);
        if (family === "gemini") return !!(status?.has_gemini_key || status?.hasGeminiKey);
        if (family === "groq") return !!(status?.has_groq_key || status?.hasGroqKey);
        if (family === "openrouter") return !!(status?.has_openrouter_key || status?.hasOpenrouterKey);

        return false;
    });

    const models = [
        ...filteredLocal.map(m => ({ ...m, type: 'local' as const })),
        ...cloudModels.map(m => ({
            path: m.id,
            name: m.name,
            type: 'cloud' as const,
            family: m.family,
            id: m.id
        }))
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
                const modelDef = cloudModels.find(m => m.id === path);
                if (!modelDef) return;

                const id = modelDef.id.toLowerCase();
                const brain = id.startsWith("openrouter-") ? "openrouter" :
                    id.startsWith("groq-") ? "groq" :
                        id.startsWith("anthropic-") ? "anthropic" :
                            id.startsWith("openai-") ? "openai" :
                                id.startsWith("google-") ? "gemini" :
                                    modelDef.family.toLowerCase();
                const modelId = modelDef.id.split('-').slice(1).join('-');

                const newConfig = {
                    ...config,
                    selected_chat_provider: brain,
                };

                await commands.updateUserConfig(newConfig);
                if ((commands as any).saveSelectedCloudModel) {
                    await (commands as any).saveSelectedCloudModel(modelId);
                }

                // Refresh state to reflect new cloud model
                const [s, cfg] = await Promise.all([
                    commands.getClawdbotStatus(),
                    commands.getUserConfig()
                ]);
                if (s.status === 'ok') setStatus(s.data);
                setConfig(cfg);

                setIsOpen(false);
                // We don't setModelPath for cloud models yet as it's handled by provider routing in backend
                // but we might want to update local UI state if needed.
                return;
            } catch (e) {
                console.error(e);
            }
        }

        if (path === modelPath && config?.selected_chat_provider === "local") {
            setIsOpen(false);
            return;
        }

        // If current provider is not local, switch it back to local
        if (config?.selected_chat_provider && config.selected_chat_provider !== "local") {
            try {
                const newConfig = { ...config, selected_chat_provider: "local" };
                await commands.updateUserConfig(newConfig);
                // Refresh both to be safe
                const [s, cfg] = await Promise.all([
                    commands.getClawdbotStatus(),
                    commands.getUserConfig()
                ]);
                if (s.status === 'ok') setStatus(s.data);
                setConfig(cfg);
            } catch (e) {
                console.error("Failed to update config to local", e);
            }
        }

        // Trigger immediate UI block
        setIsRestarting(true);
        // Update path, letting useAutoStart handle the actual server start
        setModelPath(path);
        setIsOpen(false);
    };



    return (
        <div className="relative inline-block text-left" ref={containerRef}>
            <button
                onClick={() => setIsOpen(!isOpen)}
                className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-secondary/50 hover:bg-secondary text-sm font-medium transition-colors border border-border/50 backdrop-blur-sm"
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
                </div>
                <ChevronDown className={cn("w-3 h-3 transition-transform opacity-50", isOpen && "rotate-180")} />
            </button>

            {isOpen && (
                <div className="absolute top-full mt-2 left-1/2 -translate-x-1/2 w-64 origin-top bg-card/90 backdrop-blur-xl border border-border/50 rounded-xl shadow-xl z-50 overflow-hidden animate-in fade-in zoom-in-95 duration-100">
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
                                    const provider = model.id?.startsWith("openrouter-") ? "openrouter" :
                                        model.id?.startsWith("groq-") ? "groq" :
                                            model.id?.startsWith("anthropic-") ? "anthropic" :
                                                model.id?.startsWith("openai-") ? "openai" :
                                                    model.id?.startsWith("google-") ? "gemini" :
                                                        model.family?.toLowerCase();

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
                </div>
            )}
        </div>
    );
}
