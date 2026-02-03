import { useState, useEffect, useRef } from 'react';
import { useModelContext, RECOMMENDED_MODELS } from '../model-context';
import { ChevronDown, Check, Box, Sparkles } from 'lucide-react';
import { cn } from '../../lib/utils';

export function ModelSelector({ onManageClick, isAutoMode, toggleAutoMode }: { onManageClick: () => void, isAutoMode: boolean, toggleAutoMode: (v: boolean) => void }) {
    const { localModels, currentModelPath: modelPath, setModelPath, downloading, setIsRestarting } = useModelContext();
    const [isOpen, setIsOpen] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);

    // Filter out dedicated embedding models AND downloading models AND image gen models
    const models = localModels.filter(m => {
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

    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
                setIsOpen(false);
            }
        };
        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, []);

    const handleSelect = async (path: string) => {
        if (path === "auto") {
            toggleAutoMode(!isAutoMode);
            setIsOpen(false);
            return;
        }

        if (path === modelPath) {
            setIsOpen(false);
            return;
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
                        {(models.find(m => m.path === modelPath)?.name.split(/[\\/]/).pop()) || "Select Model"} {isAutoMode && "(Auto)"}
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
                                    onClick={() => handleSelect("auto")}
                                    className="w-full text-left px-3 py-2 text-sm rounded-lg flex items-center gap-2 hover:bg-accent text-foreground group transition-colors mb-1 border-b border-border/50 pb-2"
                                >
                                    <div className="p-1 bg-yellow-500/10 rounded flex items-center justify-center">
                                        <Box className="w-3 h-3 text-yellow-500" />
                                    </div>
                                    <span className="truncate flex-1 font-medium text-yellow-600 dark:text-yellow-400">Auto Mode</span>
                                    {isAutoMode && <Check className="w-3.5 h-3.5 shrink-0 text-yellow-500" />}
                                </button>
                                {models.map((model) => {
                                    const filename = model.name.split(/[\\/]/).pop() || model.name;
                                    const def = RECOMMENDED_MODELS.find(k => k.variants?.some(v => v.filename === filename));
                                    const isRecommended = def?.recommendedForAgent;

                                    return (
                                        <button
                                            key={model.path}
                                            onClick={() => handleSelect(model.path)}
                                            className={cn(
                                                "w-full text-left px-3 py-2 text-sm rounded-lg flex items-center justify-between group transition-colors",
                                                model.path === modelPath ? "bg-primary/10 text-primary" : "hover:bg-accent text-foreground"
                                            )}
                                        >
                                            <div className="flex items-center gap-2 truncate flex-1 mr-2">
                                                <span className="truncate">{filename}</span>
                                                {isRecommended && <Sparkles className="w-3 h-3 text-yellow-500 shrink-0" />}
                                            </div>
                                            {model.path === modelPath && <Check className="w-3.5 h-3.5 shrink-0" />}
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
