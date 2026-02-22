import { useState, useRef, useCallback, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { toast } from 'sonner';
import {
    ImageIcon, Sliders, Palette,
    Download, Copy, Expand, X
} from 'lucide-react';
import { ImagineModeIcon, ImagineSendIcon, ImagineMainIcon } from '../icons/ModeIcons';
import { cn } from '../../lib/utils';
import { STYLE_LIBRARY, findStyle } from '../../lib/style-library';
import { imagineListImages, GeneratedImage } from '../../lib/imagine';
import { convertFileSrc } from '@tauri-apps/api/core';
import { useModelContext } from '../model-context';
import { ModelVariant } from '../../lib/model-library';
import { downloadImageToDisk } from '../../lib/fs-utils';
import * as openclaw from '../../lib/openclaw';

interface ImagineGenerationProps {
    onGenerate?: (prompt: string, options: GenerationOptions) => Promise<void>;
    isGenerating?: boolean;
    progress?: string | null;
    lastGeneratedImage?: string | null;
}

interface GenerationOptions {
    provider: 'local' | 'nano-banana' | 'nano-banana-pro';
    aspectRatio: string;
    resolution: string;
    styleId?: string;
    sourceImages?: string[];
    steps?: number;
}

const ASPECT_RATIOS = [
    { id: '1:1', label: 'Square', width: 1, height: 1 },
    { id: '16:9', label: 'Landscape', width: 16, height: 9 },
    { id: '9:16', label: 'Portrait', width: 9, height: 16 },
    { id: '4:3', label: 'Standard', width: 4, height: 3 },
    { id: '3:2', label: 'Photo', width: 3, height: 2 },
    { id: '21:9', label: 'Ultrawide', width: 21, height: 9 },
];

const RESOLUTIONS = [
    { id: '512', label: '512px', desc: 'Fast' },
    { id: '1K', label: '1K', desc: 'Standard' },
    { id: '2K', label: '2K', desc: 'High' },
];

export function ImagineGeneration({
    onGenerate,
    isGenerating = false,
    progress,
    lastGeneratedImage
}: ImagineGenerationProps) {
    const { currentImageGenModelPath, models } = useModelContext();

    // Find active model and variant for constraints
    const activeModel = models.find(m => m.category === 'Diffusion' && m.variants.some(v => currentImageGenModelPath.includes(v.filename)));
    const activeVariant: ModelVariant | undefined = activeModel?.variants.find(v => currentImageGenModelPath.includes(v.filename));

    const [prompt, setPrompt] = useState('');
    const [showSettings, setShowSettings] = useState(false);
    const [showStylePicker, setShowStylePicker] = useState(false);
    const [provider, setProvider] = useState<'local' | 'nano-banana' | 'nano-banana-pro'>(activeModel ? 'local' : 'nano-banana');
    const [aspectRatio, setAspectRatio] = useState('1:1');
    const [resolution, setResolution] = useState('512');
    const [selectedStyleId, setSelectedStyleId] = useState<string | null>(null);
    const [sourceImages, setSourceImages] = useState<string[]>([]);
    const [steps, setSteps] = useState(28); // Default to a safe SD value
    const [recentImages, setRecentImages] = useState<GeneratedImage[]>([]);
    const [previewImage, setPreviewImage] = useState<string | null>(null);
    const [isExpanded, setIsExpanded] = useState(false);

    const settingsRef = useRef<HTMLDivElement>(null);
    const stylePickerRef = useRef<HTMLDivElement>(null);
    const settingsButtonRef = useRef<HTMLButtonElement>(null);
    const styleButtonRef = useRef<HTMLButtonElement>(null);

    const minSteps = activeVariant?.min_steps ?? 1;
    const maxSteps = activeVariant?.max_steps ?? 50;

    // Reset steps when model changes
    useEffect(() => {
        if (activeVariant?.default_steps) {
            setSteps(activeVariant.default_steps);
        }
    }, [currentImageGenModelPath, activeVariant?.default_steps]);

    useEffect(() => {
        setPreviewImage(null);
    }, [lastGeneratedImage]);

    const displayImage = previewImage || lastGeneratedImage;
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const fileInputRef = useRef<HTMLInputElement>(null);

    // Helpers
    const handleDownload = async () => {
        if (!displayImage) return;
        try {
            const filename = `generated-${Date.now()}.png`;
            await downloadImageToDisk(displayImage, filename);
            toast.success("Image downloaded");
        } catch (err) {
            console.error('Failed to download:', err);
            toast.error("Failed to download image");
        }
    };

    const handleCopy = async () => {
        if (!displayImage) return;
        try {
            // Create ClipboardItem with a Promise to preserve the user gesture
            // This is required for Safari/WebKit which invalidates the gesture if you await before writing
            const item = new ClipboardItem({
                'image/png': (async () => {
                    // 1. Fetch
                    const response = await fetch(displayImage);
                    const blob = await response.blob();

                    // 2. Sanitize via Bitmap (faster than Image element)
                    const img = await createImageBitmap(blob);
                    const canvas = document.createElement('canvas');
                    canvas.width = img.width;
                    canvas.height = img.height;
                    const ctx = canvas.getContext('2d');
                    if (!ctx) throw new Error('Failed to get canvas context');

                    ctx.drawImage(img, 0, 0);

                    // 3. Convert to clean PNG Blob
                    return new Promise<Blob>((resolve, reject) => {
                        canvas.toBlob(b => {
                            if (b) resolve(b);
                            else reject(new Error('Canvas to Blob failed'));
                        }, 'image/png');
                    });
                })()
            });

            await navigator.clipboard.write([item]);
            toast.success("Copied to clipboard");
        } catch (err) {
            console.error('Failed to copy:', err);
            toast.error(`Failed to copy: ${err instanceof Error ? err.message : String(err)}`);
        }
    };

    // Debug log for isGenerating prop
    useEffect(() => {
    }, [isGenerating]);

    // Load recent images on mount
    useEffect(() => {
        loadRecentImages();
    }, []);

    const loadRecentImages = async () => {
        try {
            const images = await imagineListImages(10);
            setRecentImages(images);
        } catch (error) {
            console.error('Failed to load recent images:', error);
        }
    };

    // Click outside handler
    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            const target = event.target as Node;

            // Handle Settings Menu
            if (showSettings &&
                settingsRef.current &&
                !settingsRef.current.contains(target) &&
                settingsButtonRef.current &&
                !settingsButtonRef.current.contains(target)
            ) {
                setShowSettings(false);
            }

            // Handle Style Picker
            if (showStylePicker &&
                stylePickerRef.current &&
                !stylePickerRef.current.contains(target) &&
                styleButtonRef.current &&
                !styleButtonRef.current.contains(target)
            ) {
                setShowStylePicker(false);
            }
        };

        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, [showSettings, showStylePicker]);

    const selectedStyle = selectedStyleId ? findStyle(selectedStyleId) : null;

    const [hasGeminiKey, setHasGeminiKey] = useState<boolean>(false);

    // Fetch openclaw status to check for keys
    useEffect(() => {
        const checkStatus = async () => {
            try {
                const status = await openclaw.getOpenClawStatus();
                setHasGeminiKey(status.has_gemini_key);
            } catch (e) {
                console.error("Failed to check status:", e);
            }
        };
        checkStatus();
    }, []);

    const handleGenerate = useCallback(async () => {
        if (!prompt.trim() || isGenerating) return;

        // Check for Gemini Key if using Cloud Models
        if ((provider === 'nano-banana' || provider === 'nano-banana-pro') && !hasGeminiKey) {
            toast.error("Google Gemini API Key Required", {
                description: "Cloud generation requires a Gemini API key.",
                action: {
                    label: "Add Key",
                    onClick: () => window.dispatchEvent(new CustomEvent('open-settings', { detail: 'secrets' }))
                }
            });
            return;
        }

        // Clear the prompt and images after sending
        const currentPrompt = prompt;
        const currentImages = sourceImages;
        setPrompt('');
        setSourceImages([]);

        await onGenerate?.(currentPrompt, {
            provider,
            aspectRatio,
            resolution,
            styleId: selectedStyleId || undefined,
            sourceImages: currentImages.length > 0 ? currentImages : undefined,
            steps: provider === 'local' ? steps : undefined
        });

        // Refresh recent images after generation
        await loadRecentImages();
    }, [prompt, provider, aspectRatio, resolution, selectedStyleId, sourceImages, steps, isGenerating, onGenerate, hasGeminiKey]);

    const handleKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            handleGenerate();
        }
    };

    const handleImageDrop = useCallback((e: React.DragEvent) => {
        e.preventDefault();
        const files = Array.from(e.dataTransfer.files);
        const imageFiles = files.filter(f => f.type.startsWith('image/')).slice(0, 14 - sourceImages.length);

        if (imageFiles.length === 0) return;

        imageFiles.forEach(file => {
            const reader = new FileReader();
            reader.onload = (event) => {
                const dataUrl = event.target?.result as string;
                setSourceImages(prev => [...prev, dataUrl].slice(0, 14));
                if (provider === 'local') {
                    setProvider('nano-banana');
                    toast.info("Switched to Cloud for img2img support");
                }
            };
            reader.readAsDataURL(file);
        });
    }, [provider, sourceImages.length]);

    return (
        <div className="flex-1 flex flex-col h-full relative overflow-hidden">
            {/* Main Canvas Area */}
            <div
                className="flex-1 flex items-center justify-center p-4 md:p-8 relative min-h-0 overflow-hidden"
                onDragOver={(e) => e.preventDefault()}
                onDrop={handleImageDrop}
            >
                <AnimatePresence mode="wait">
                    {isGenerating ? (
                        <motion.div
                            key="loading"
                            initial={{ opacity: 0, scale: 0.95 }}
                            animate={{ opacity: 1, scale: 1 }}
                            exit={{ opacity: 0, scale: 0.95 }}
                            className="flex flex-col items-center justify-center gap-8 w-full max-w-md"
                        >
                            <motion.div
                                className="relative w-32 h-32 rounded-3xl bg-card border border-border/50 flex items-center justify-center shadow-2xl"
                                animate={{
                                    boxShadow: [
                                        '0 0 20px hsl(var(--primary)/0.1)',
                                        '0 0 40px hsl(var(--primary)/0.2)',
                                        '0 0 20px hsl(var(--primary)/0.1)'
                                    ]
                                }}
                                transition={{ duration: 4, repeat: Infinity, ease: "easeInOut" }}
                            >
                                <div className="absolute inset-0 bg-gradient-to-tr from-primary/5 via-transparent to-primary/5 animate-pulse" />
                                <ImagineMainIcon size={80} isActive={true} className="relative z-10" />
                            </motion.div>

                            <div className="w-full space-y-4">
                                <div className="flex justify-between items-end mb-1">
                                    <div className="flex flex-col gap-1">
                                        <span className="text-xs font-bold uppercase tracking-widest text-primary/70">
                                            {typeof progress === 'object' && progress !== null ? (progress as any).stage : "Processing"}
                                        </span>
                                        <p className="text-lg font-semibold text-foreground">
                                            {typeof progress === 'object' && progress !== null
                                                ? (typeof (progress as any).text === 'string' ? (progress as any).text : JSON.stringify((progress as any).text))
                                                : (typeof progress === 'string' ? progress : "Creating your vision...")}
                                        </p>
                                    </div>
                                    <span className="text-sm font-mono text-muted-foreground">
                                        {typeof progress === 'object' && progress !== null ? `${Math.round((progress as any).progress * 100)}%` : ""}
                                    </span>
                                </div>

                                <div className="h-3 w-full bg-muted/30 rounded-full border border-border/20 overflow-hidden relative backdrop-blur-sm">
                                    <motion.div
                                        className="h-full bg-gradient-to-r from-primary via-primary/80 to-primary relative"
                                        initial={{ width: 0 }}
                                        animate={{
                                            width: typeof progress === 'object' && progress !== null
                                                ? `${Math.max(5, (progress as any).progress * 100)}%`
                                                : "10%"
                                        }}
                                        transition={{ type: "spring", bounce: 0, duration: 0.5 }}
                                    >
                                        <div className="absolute inset-0 bg-[linear-gradient(45deg,rgba(255,255,255,0.15)_25%,transparent_25%,transparent_50%,rgba(255,255,255,0.15)_50%,rgba(255,255,255,0.15)_75%,transparent_75%,transparent)] bg-[length:24px_24px] animate-[progress-bar-stripes_1s_linear_infinity]" />
                                    </motion.div>
                                </div>

                                <div className="flex justify-center gap-6 text-[10px] font-bold uppercase tracking-tighter text-muted-foreground/40">
                                    <span className={cn((progress as any)?.stage === "Initializing" && "text-primary/60")}>Init</span>
                                    <span className={cn((progress as any)?.stage === "Loading Weights" && "text-primary/60")}>Weights</span>
                                    <span className={cn((progress as any)?.stage === "Engine Setup" && "text-primary/60")}>Engine</span>
                                    <span className={cn((progress as any)?.stage === "Generating" && "text-primary/60")}>Sample</span>
                                    <span className={cn((progress as any)?.stage === "Saving" && "text-primary/60")}>Finalize</span>
                                </div>
                            </div>
                        </motion.div>
                    ) : displayImage ? (
                        <motion.div
                            key={displayImage}
                            initial={{ opacity: 0, y: 20 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: -20 }}
                            className="relative group h-full flex items-center justify-center p-4"
                        >
                            <img
                                src={displayImage}
                                alt="Generated"
                                className="max-w-full max-h-full w-auto h-auto rounded-2xl shadow-2xl shadow-black/20 object-contain"
                            />
                            {/* Action buttons overlay */}
                            <motion.div
                                className="absolute bottom-10 left-1/2 -translate-x-1/2 flex gap-2 opacity-0 group-hover:opacity-100 transition-opacity"
                                initial={{ y: 10 }}
                                whileHover={{ y: 0 }}
                            >
                                <button
                                    className="p-2 rounded-lg bg-black/60 backdrop-blur text-white hover:bg-black/80 transition-colors"
                                    onClick={handleDownload}
                                    title="Download"
                                >
                                    <Download className="w-4 h-4" />
                                </button>
                                <button
                                    className="p-2 rounded-lg bg-black/60 backdrop-blur text-white hover:bg-black/80 transition-colors"
                                    onClick={handleCopy}
                                    title="Copy to Clipboard"
                                >
                                    <Copy className="w-4 h-4" />
                                </button>
                                <button
                                    className="p-2 rounded-lg bg-black/60 backdrop-blur text-white hover:bg-black/80 transition-colors"
                                    onClick={() => setIsExpanded(true)}
                                    title="Expand"
                                >
                                    <Expand className="w-4 h-4" />
                                </button>
                                {/* <button className="p-2 rounded-lg bg-pink-500/80 backdrop-blur text-white hover:bg-pink-500 transition-colors">
                                    <RefreshCw className="w-4 h-4" />
                                </button> */}
                            </motion.div>
                        </motion.div>
                    ) : (
                        <motion.div
                            key="empty"
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex flex-col items-center gap-4 text-center max-w-md"
                        >
                            <motion.div
                                className="w-24 h-24 rounded-2xl bg-gradient-to-br from-primary/10 to-accent/10 flex items-center justify-center"
                                animate={{
                                    y: [0, -4, 0],
                                }}
                                transition={{ duration: 3, repeat: Infinity, ease: "easeInOut" }}
                            >
                                <ImagineMainIcon size={48} isActive={false} />
                            </motion.div>
                            <div>
                                <h3 className="text-lg font-semibold text-foreground mb-1">Bring your imagination to life</h3>
                                <p className="text-sm text-muted-foreground">
                                    Describe what you want to create, or drop an image to edit
                                </p>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>

            {/* Bottom Controls Area */}
            <div className="relative w-full flex flex-col items-center pb-8 px-4 gap-4 z-30">
                {/* Recent Generations Strip */}
                <AnimatePresence>
                    {recentImages.length > 0 && (
                        <motion.div
                            initial={{ opacity: 0, y: 20 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: 20 }}
                            className="flex gap-2 p-2 rounded-2xl bg-black/40 backdrop-blur-md border border-white/10 overflow-x-auto max-w-full"
                        >
                            {recentImages.map((img) => (
                                <button
                                    key={img.id}
                                    onClick={() => {
                                        setPreviewImage(convertFileSrc(img.filePath));
                                        setPrompt(img.prompt);
                                        if (img.styleId) setSelectedStyleId(img.styleId);
                                        if (img.aspectRatio) setAspectRatio(img.aspectRatio);
                                        if (img.resolution) setResolution(img.resolution);
                                        if (img.provider === 'local' || img.provider === 'nano-banana' || img.provider === 'nano-banana-pro') {
                                            setProvider(img.provider);
                                        }
                                    }}
                                    className="relative group shrink-0 w-16 h-16 rounded-lg overflow-hidden border border-white/10 hover:border-primary/50 transition-colors"
                                >
                                    <img
                                        src={convertFileSrc(img.filePath)}
                                        alt={img.prompt}
                                        className="w-full h-full object-cover"
                                    />
                                    {img.isFavorite && (
                                        <div className="absolute top-0.5 right-0.5 w-2 h-2 rounded-full bg-yellow-500" />
                                    )}
                                </button>
                            ))}
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* Input Area controls: Style Badge and Image Previews */}
                <div className="relative w-full max-w-2xl flex flex-col items-start gap-2 px-1">
                    {/* Style badge */}
                    <AnimatePresence>
                        {selectedStyle && (
                            <motion.div
                                initial={{ opacity: 0, y: 10, scale: 0.9 }}
                                animate={{ opacity: 1, y: 0, scale: 1 }}
                                exit={{ opacity: 0, y: 10, scale: 0.9 }}
                                className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-primary/10 border border-primary/20 text-primary text-xs z-20 shadow-sm"
                            >
                                <Palette className="w-3 h-3" />
                                <span className="font-medium">{selectedStyle.label}</span>
                                <button
                                    onClick={() => setSelectedStyleId(null)}
                                    className="ml-1 p-0.5 hover:bg-primary/20 rounded-full transition-colors"
                                >
                                    <X className="w-3 h-3" />
                                </button>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>

                {/* Main Input Bar and Attachments */}
                <div className="relative w-full max-w-2xl pointer-events-auto">
                    {/* Source Image Attachment Preview */}
                    <AnimatePresence>
                        {sourceImages.length > 0 && (
                            <motion.div
                                initial={{ opacity: 0, y: 10, scale: 0.95 }}
                                animate={{ opacity: 1, y: 0, scale: 1 }}
                                exit={{ opacity: 0, y: 10, scale: 0.95 }}
                                className="flex gap-2 mb-3 overflow-x-auto pb-1 px-1 scrollbar-hide z-20"
                            >
                                {sourceImages.map((src, idx) => (
                                    <div key={idx} className="group relative flex items-center gap-2 p-1.5 pr-2.5 rounded-xl border border-border/40 bg-card/60 backdrop-blur-md shadow-lg transition-all duration-300 select-none shrink-0">
                                        <div className="w-8 h-8 rounded-lg overflow-hidden border border-primary/20">
                                            <img
                                                src={src}
                                                alt={`Source ${idx + 1}`}
                                                className="w-full h-full object-cover"
                                            />
                                        </div>
                                        <div className="flex flex-col">
                                            <span className="text-[10px] font-semibold text-foreground/90 leading-tight">Image {idx + 1}</span>
                                            <span className="text-[9px] font-bold text-primary uppercase tracking-wide leading-none">img2img</span>
                                        </div>
                                        <button
                                            onClick={() => setSourceImages(prev => prev.filter((_, i) => i !== idx))}
                                            className="ml-1 p-0.5 hover:bg-destructive/10 text-muted-foreground hover:text-destructive rounded-full transition-colors opacity-0 group-hover:opacity-100"
                                        >
                                            <X className="w-3 h-3" />
                                        </button>
                                    </div>
                                ))}
                                {sourceImages.length < 14 && (
                                    <button
                                        onClick={() => fileInputRef.current?.click()}
                                        className="flex items-center justify-center w-12 h-11 rounded-xl border-2 border-dashed border-border/40 bg-card/40 hover:bg-accent/40 text-muted-foreground hover:text-primary transition-all shrink-0"
                                    >
                                        <ImageIcon className="w-4 h-4" />
                                    </button>
                                )}
                            </motion.div>
                        )}
                    </AnimatePresence>

                    <div className={cn(
                        "flex items-end gap-2 p-3 rounded-2xl border transition-all duration-300",
                        "bg-card/80 backdrop-blur-xl shadow-2xl shadow-black/10",
                        "border-border/50 focus-within:border-primary/50 focus-within:ring-2 focus-within:ring-primary/20"
                    )}>
                        {/* Left actions */}
                        <div className="flex items-center gap-1">
                            <input
                                ref={fileInputRef}
                                type="file"
                                accept="image/*"
                                multiple
                                className="hidden"
                                onChange={(e) => {
                                    const files = Array.from(e.target.files || []);
                                    const imageFiles = files.filter(f => f.type.startsWith('image/')).slice(0, 14 - sourceImages.length);

                                    imageFiles.forEach(file => {
                                        const reader = new FileReader();
                                        reader.onload = (event) => {
                                            const dataUrl = event.target?.result as string;
                                            setSourceImages(prev => [...prev, dataUrl].slice(0, 14));
                                            if (provider === 'local') {
                                                setProvider('nano-banana');
                                                toast.info("Switched to Cloud for img2img support");
                                            }
                                        };
                                        reader.readAsDataURL(file);
                                    });
                                }}
                            />
                            <button
                                onClick={() => fileInputRef.current?.click()}
                                className={cn(
                                    "p-2 rounded-lg transition-colors",
                                    sourceImages.length > 0 ? "text-primary bg-primary/10" : "text-muted-foreground hover:text-foreground hover:bg-accent"
                                )}
                                title="Add source image"
                            >
                                <ImageIcon className="w-5 h-5" />
                            </button>
                            <button
                                ref={styleButtonRef}
                                onClick={() => {
                                    setShowStylePicker(!showStylePicker);
                                    if (!showStylePicker) setShowSettings(false); // Close other
                                }}
                                className={cn(
                                    "p-2 rounded-lg transition-colors",
                                    selectedStyleId
                                        ? "text-primary bg-primary/10"
                                        : "text-muted-foreground hover:text-foreground hover:bg-accent"
                                )}
                                title="Select style"
                            >
                                <Palette className="w-5 h-5" />
                            </button>
                            <button
                                ref={settingsButtonRef}
                                onClick={() => {
                                    setShowSettings(!showSettings);
                                    if (!showSettings) setShowStylePicker(false); // Close other
                                }}
                                className={cn(
                                    "p-2 rounded-lg transition-colors",
                                    showSettings
                                        ? "text-foreground bg-accent"
                                        : "text-muted-foreground hover:text-foreground hover:bg-accent"
                                )}
                                title="Generation settings"
                            >
                                <Sliders className="w-5 h-5" />
                            </button>
                        </div>

                        {/* Textarea */}
                        <textarea
                            ref={textareaRef}
                            value={prompt}
                            onChange={(e) => setPrompt(e.target.value)}
                            onKeyDown={handleKeyDown}
                            placeholder={sourceImages.length > 0 ? "Describe how to edit these images..." : "Describe your imagination..."}
                            rows={1}
                            className="flex-1 resize-none bg-transparent border-0 outline-none text-foreground placeholder:text-muted-foreground min-h-[40px] max-h-[120px] py-2"
                            style={{ height: 'auto' }}
                            onInput={(e) => {
                                const target = e.target as HTMLTextAreaElement;
                                target.style.height = 'auto';
                                target.style.height = Math.min(target.scrollHeight, 120) + 'px';
                            }}
                        />

                        {/* Generate button */}
                        <button
                            onClick={handleGenerate}
                            disabled={!prompt.trim() || isGenerating}
                            className={cn(
                                "p-3 rounded-xl transition-all duration-300",
                                "bg-gradient-to-r from-primary to-accent",
                                "text-primary-foreground shadow-lg shadow-primary/20",
                                "hover:shadow-xl hover:shadow-primary/30 hover:scale-105",
                                "disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:scale-100"
                            )}
                        >
                            {isGenerating ? (
                                <ImagineModeIcon size={20} isActive={true} />
                            ) : (
                                <ImagineSendIcon size={20} isActive={prompt.length > 0} />
                            )}
                        </button>
                    </div>

                    {/* Settings Panel */}
                    <AnimatePresence>
                        {showSettings && (
                            <motion.div
                                ref={settingsRef}
                                initial={{ opacity: 0, y: 10, height: 0 }}
                                animate={{ opacity: 1, y: 0, height: 'auto' }}
                                exit={{ opacity: 0, y: 10, height: 0 }}
                                className="absolute bottom-full left-0 right-0 mb-2 p-4 rounded-xl bg-card/95 backdrop-blur-xl border border-border/50 shadow-xl overflow-hidden"
                            >
                                <div className="space-y-4">
                                    {/* Provider */}
                                    <div>
                                        <label className="text-[10px] uppercase font-bold text-muted-foreground/70 tracking-wider mb-2 block">
                                            Provider
                                        </label>
                                        <div className="grid grid-cols-3 gap-2">
                                            <button
                                                onClick={() => setProvider('local')}
                                                className={cn(
                                                    "px-3 py-2 rounded-lg text-sm transition-all",
                                                    provider === 'local'
                                                        ? "bg-primary/10 text-primary ring-1 ring-primary/30"
                                                        : "bg-muted/50 text-muted-foreground hover:text-foreground"
                                                )}
                                            >
                                                <div className="font-medium">Local</div>
                                                <div className="text-[10px] opacity-70">On-device</div>
                                            </button>
                                            <button
                                                onClick={() => setProvider('nano-banana')}
                                                className={cn(
                                                    "px-3 py-2 rounded-lg text-sm transition-all",
                                                    provider === 'nano-banana'
                                                        ? "bg-primary/10 text-primary ring-1 ring-primary/30"
                                                        : "bg-muted/50 text-muted-foreground hover:text-foreground"
                                                )}
                                            >
                                                <div className="font-medium">Cloud</div>
                                                <div className="text-[10px] opacity-70">Nano Banana</div>
                                            </button>
                                            <button
                                                onClick={() => setProvider('nano-banana-pro')}
                                                className={cn(
                                                    "px-3 py-2 rounded-lg text-sm transition-all",
                                                    provider === 'nano-banana-pro'
                                                        ? "bg-primary/10 text-primary ring-1 ring-primary/30"
                                                        : "bg-muted/50 text-muted-foreground hover:text-foreground"
                                                )}
                                            >
                                                <div className="font-medium">Pro</div>
                                                <div className="text-[10px] opacity-70">Nano Banana Pro</div>
                                            </button>
                                        </div>
                                    </div>

                                    {/* Aspect Ratio */}
                                    <div>
                                        <label className="text-[10px] uppercase font-bold text-muted-foreground/70 tracking-wider mb-2 block">
                                            Aspect Ratio
                                        </label>
                                        <div className="flex flex-wrap gap-1">
                                            {ASPECT_RATIOS.map((ar) => (
                                                <button
                                                    key={ar.id}
                                                    onClick={() => setAspectRatio(ar.id)}
                                                    className={cn(
                                                        "px-2.5 py-1.5 rounded-md text-xs transition-all",
                                                        aspectRatio === ar.id
                                                            ? "bg-primary/10 text-primary ring-1 ring-primary/30"
                                                            : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
                                                    )}
                                                >
                                                    {ar.label}
                                                </button>
                                            ))}
                                        </div>
                                    </div>

                                    {/* Resolution (Local & Nano Pro) */}
                                    {(provider === 'local' || provider === 'nano-banana-pro') && (
                                        <div>
                                            <label className="text-[10px] uppercase font-bold text-muted-foreground/70 tracking-wider mb-2 block">
                                                Resolution
                                            </label>
                                            <div className="flex gap-2">
                                                {RESOLUTIONS.map((res) => (
                                                    <button
                                                        key={res.id}
                                                        onClick={() => setResolution(res.id)}
                                                        className={cn(
                                                            "flex-1 px-3 py-1.5 rounded-md text-xs transition-all",
                                                            resolution === res.id
                                                                ? "bg-primary/10 text-primary ring-1 ring-primary/30"
                                                                : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
                                                        )}
                                                    >
                                                        <span className="font-medium">{res.label}</span>
                                                        <span className="text-[10px] opacity-70 ml-1">{res.desc}</span>
                                                    </button>
                                                ))}
                                            </div>
                                        </div>
                                    )}

                                    {/* Diffusion Steps (Local only) */}
                                    {provider === 'local' && (
                                        <div className="space-y-3 pt-2 border-t border-border/30">
                                            <div className="flex justify-between items-center text-[10px] uppercase font-bold text-muted-foreground/70 tracking-wider">
                                                <span>Inference Steps</span>
                                                <span className="bg-primary/10 text-primary px-1.5 py-0.5 rounded font-mono font-bold tracking-normal">{steps}</span>
                                            </div>
                                            <div className="relative flex items-center gap-3">
                                                <input
                                                    type="range"
                                                    min={minSteps}
                                                    max={maxSteps}
                                                    step="1"
                                                    value={steps}
                                                    onChange={(e) => setSteps(parseInt(e.target.value))}
                                                    className="w-full h-1.5 bg-muted rounded-lg appearance-none cursor-pointer accent-primary"
                                                />
                                            </div>
                                            <div className="flex justify-between text-[8px] text-muted-foreground/40 font-bold uppercase tracking-widest">
                                                <span>Faster</span>
                                                <span>Better Quality</span>
                                            </div>
                                        </div>
                                    )}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>

                    {/* Style Picker */}
                    <AnimatePresence>
                        {showStylePicker && (
                            <motion.div
                                ref={stylePickerRef}
                                initial={{ opacity: 0, y: 10 }}
                                animate={{ opacity: 1, y: 0 }}
                                exit={{ opacity: 0, y: 10 }}
                                className="absolute bottom-full left-0 right-0 mb-2 p-4 rounded-xl bg-card/95 backdrop-blur-xl border border-border/50 shadow-xl max-h-64 overflow-y-auto"
                            >
                                <div className="text-[10px] uppercase font-bold text-muted-foreground/70 tracking-wider mb-3">
                                    Style Presets
                                </div>
                                <div className="grid grid-cols-3 gap-2">
                                    {STYLE_LIBRARY.map((style) => (
                                        <button
                                            key={style.id}
                                            onClick={() => {
                                                setSelectedStyleId(style.id);
                                                setShowStylePicker(false);
                                            }}
                                            className={cn(
                                                "px-3 py-2 rounded-lg text-left transition-all",
                                                selectedStyleId === style.id
                                                    ? "bg-primary/10 text-primary ring-1 ring-primary/30"
                                                    : "bg-muted/30 text-muted-foreground hover:text-foreground hover:bg-muted/50"
                                            )}
                                        >
                                            <div className="text-xs font-medium truncate">{style.label}</div>
                                            <div className="text-[10px] opacity-70 truncate">{style.description}</div>
                                        </button>
                                    ))}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>
            </div>

            {/* Expanded Modal */}
            <AnimatePresence>
                {isExpanded && displayImage && (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="fixed inset-0 z-50 bg-black/95 flex items-center justify-center p-8 backdrop-blur-sm"
                        onClick={() => setIsExpanded(false)}
                    >
                        <motion.div
                            initial={{ scale: 0.9, opacity: 0 }}
                            animate={{ scale: 1, opacity: 1 }}
                            exit={{ scale: 0.9, opacity: 0 }}
                            className="relative w-full h-full flex items-center justify-center"
                            onClick={(e) => e.stopPropagation()}
                        >
                            <img
                                src={displayImage}
                                alt="Expanded"
                                className="max-w-full max-h-[90vh] object-contain rounded-lg shadow-2xl"
                            />
                            <button
                                onClick={() => setIsExpanded(false)}
                                className="absolute top-4 right-4 p-2 rounded-full bg-white/10 text-white hover:bg-white/20 transition-colors z-50"
                            >
                                <X className="w-8 h-8" />
                            </button>

                            {/* Modal Actions */}
                            <div className="absolute bottom-8 left-1/2 -translate-x-1/2 flex gap-3 z-50">
                                <button
                                    className="flex items-center gap-2 px-6 py-3 rounded-full bg-black/60 backdrop-blur text-white hover:bg-black/80 transition-colors border border-white/10"
                                    onClick={handleDownload}
                                >
                                    <Download className="w-5 h-5" />
                                    <span className="text-base font-medium">Download</span>
                                </button>
                                <button
                                    className="flex items-center gap-2 px-6 py-3 rounded-full bg-black/60 backdrop-blur text-white hover:bg-black/80 transition-colors border border-white/10"
                                    onClick={handleCopy}
                                >
                                    <Copy className="w-5 h-5" />
                                    <span className="text-base font-medium">Copy</span>
                                </button>
                            </div>
                        </motion.div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
