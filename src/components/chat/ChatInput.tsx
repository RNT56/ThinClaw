import { memo, useRef, useEffect } from 'react';
import { cn } from '../../lib/utils';
import { Paperclip, X, Sparkles, Globe, Mic, Square, Palette, Send, Server, Terminal, Layers, ChevronRight, ImageIcon } from 'lucide-react';
import { toast } from 'sonner';
import { motion, AnimatePresence } from 'framer-motion';

// ... This would rely on many props. Let's define the interface.

export interface ChatInputProps {
    input: string;
    setInput: (value: string) => void;
    textareaRef: React.RefObject<HTMLTextAreaElement | null>;
    isStreaming: boolean;
    isRestarting: boolean;
    modelRunning: boolean;
    isImageMode: boolean;
    isWebSearchEnabled: boolean;
    isRecording: boolean;
    canSee: boolean;
    isRagCapable: boolean;
    isCloudProvider: boolean;
    autoMode: boolean;
    attachedImages: { id: string; path: string }[];
    ingestedFiles: { id: string; name: string }[];

    // Handlers
    handleSend: () => void;
    handleGenerateImage: () => void;
    handleCancelGeneration: () => void;
    handleImageUpload: () => void;
    handleFileUpload: () => void;
    handleMicClick: () => void;

    // Feature Toggles
    imageRunning: boolean;
    setIsImageMode: (val: boolean) => void;
    setIsWebSearchEnabled: (val: boolean) => void;
    setShowImageSettings: (val: boolean) => void;
    showImageSettings: boolean;

    // Callbacks
    startServer?: () => Promise<void>;

    // Slash Command / Mention State
    slashQuery: string | null;
    setSlashQuery: (val: string | null) => void;
    mentionQuery: string | null;
    setMentionQuery: (val: string | null) => void;

    // Settings
    cfgScale: number;
    setCfgScale: (val: number) => void;
    imageSteps: number;
    setImageSteps: (val: number) => void;

    // Suggestion Lists
    filteredDocs: { id: string; name: string }[];
    slashSuggestions: { id: string; label: string; desc: string; type: string }[];

    // Active Suggestions state
    selectedIndex: number;
    setSelectedIndex: (fn: (prev: number) => number) => void;
    slashSelectedIndex: number;
    setSlashSelectedIndex: (fn: (prev: number) => number) => void;
    handleSlashCommandExecute: (s: any) => void;
    setIngestedFiles: (fn: (prev: { id: string; name: string }[]) => { id: string; name: string }[]) => void;

    // Style
    activeStyleId: string | null;
    setActiveStyleId: (id: string | null) => void;
    findStyle: (id: string) => { label: string } | undefined;
}

export const ChatInput = memo(function ChatInput({
    input, setInput, textareaRef, isStreaming, isRestarting, modelRunning, isImageMode, isWebSearchEnabled, isRecording,
    canSee, isRagCapable, isCloudProvider, autoMode, attachedImages, ingestedFiles,
    handleSend, handleGenerateImage, handleCancelGeneration, handleImageUpload, handleFileUpload, handleMicClick,
    setIsImageMode, setIsWebSearchEnabled, setShowImageSettings, showImageSettings, startServer,
    slashQuery, setSlashQuery, mentionQuery, setMentionQuery, cfgScale, setCfgScale, imageSteps, setImageSteps,
    filteredDocs, slashSuggestions, selectedIndex, setSelectedIndex, slashSelectedIndex, setSlashSelectedIndex, handleSlashCommandExecute, setIngestedFiles,
    activeStyleId, setActiveStyleId, findStyle, imageRunning
}: ChatInputProps) {

    const slashCommandContainerRef = useRef<HTMLDivElement>(null);

    // Initial focus
    useEffect(() => {
        if (!isStreaming && !isRestarting) {
            textareaRef.current?.focus();
        }
    }, [isStreaming, isRestarting]);

    return (
        <div className="relative flex items-end gap-2 bg-background/60 backdrop-blur-xl border border-input/50 p-2 rounded-2xl shadow-lg transition-all">
            {(canSee || isRagCapable || isImageMode) && !isWebSearchEnabled && (
                <div className="relative group flex flex-col justify-end">
                    <div className="absolute bottom-full left-0 mb-0 flex flex-col gap-1 p-1 bg-background/80 backdrop-blur-md border rounded-xl shadow-xl opacity-0 translate-y-2 invisible group-hover:visible group-hover:opacity-100 group-hover:translate-y-0 transition-all duration-200 ease-out z-20 min-w-[120px]">
                        {(canSee || isImageMode) && (
                            <button onClick={handleImageUpload} className="flex items-center gap-2 p-2 hover:bg-accent rounded-lg text-xs font-medium transition-colors">
                                <div className="p-1.5 bg-blue-500/10 rounded-md"><ImageIcon className="w-4 h-4 text-blue-500" /></div>
                                <span>Image</span>
                            </button>
                        )}
                        {!isImageMode && (
                            <button onClick={handleFileUpload} disabled={!isRagCapable} className="flex items-center gap-2 p-2 hover:bg-accent rounded-lg text-xs font-medium transition-colors disabled:opacity-50">
                                <div className="p-1.5 bg-orange-500/10 rounded-md"><Paperclip className="w-4 h-4 text-orange-500" /></div>
                                <span>Document</span>
                            </button>
                        )}
                    </div>
                    <div className="p-2 text-muted-foreground hover:text-foreground hover:bg-background/50 rounded-lg transition-colors cursor-pointer">
                        <Paperclip className="w-5 h-5" />
                    </div>
                </div>
            )}
            <div className="flex-1 relative flex flex-col">
                {activeStyleId && (
                    <div className="absolute -top-10 left-0 flex items-center gap-1.5 bg-primary/10 border border-primary/30 text-primary px-2 py-1 rounded-full text-[10px] font-bold uppercase tracking-wider animate-in slide-in-from-bottom-2">
                        <Sparkles className="w-3 h-3" />
                        <span>Style: {findStyle(activeStyleId)?.label}</span>
                        <button onClick={() => setActiveStyleId(null)} className="ml-1 hover:text-primary">
                            <X className="w-3 h-3" />
                        </button>
                    </div>
                )}
                <textarea
                    ref={textareaRef}
                    value={input}
                    onChange={(e) => {
                        const newVal = e.target.value;
                        // Style Command Detection
                        if (newVal.startsWith("/style_")) {
                            const match = newVal.match(/^\/style_([a-zA-Z0-9-]+)(\s+)?(.*)/);
                            if (match) {
                                const styleId = match[1];
                                const remainder = match[3] || "";
                                const styleDef = findStyle(styleId);
                                if (styleDef) {
                                    setIsImageMode(true);
                                    setActiveStyleId(styleId);
                                    setInput(remainder);
                                    toast.success(`Style Locked: ${styleDef.label}`, {
                                        icon: "🎨"
                                    });
                                    return;
                                }
                            }
                        }
                        setInput(newVal);

                        // Check for @ mention at cursor position
                        const cursor = e.target.selectionStart;
                        const textBeforeCursor = newVal.slice(0, cursor);
                        const lastAt = textBeforeCursor.lastIndexOf('@');

                        if (lastAt !== -1) {
                            const query = textBeforeCursor.slice(lastAt + 1);
                            // If query contains space, invalidate unless it is very short (e.g. "my file") but usually handles filenames
                            if (!query.includes(' ') && query.length < 20) {
                                setMentionQuery(query);
                                setSelectedIndex(() => 0);
                                return;
                            }
                        }
                        setMentionQuery(null);

                        // Slash Command Discovery
                        if (newVal.startsWith("/")) {
                            setSlashQuery(newVal);
                            setSlashSelectedIndex(() => 0);
                        } else {
                            setSlashQuery(null);
                        }
                    }}
                    onKeyDown={(e) => {
                        // Handle Mentions
                        if (mentionQuery !== null && filteredDocs.length > 0) {
                            if (e.key === 'ArrowUp') { e.preventDefault(); setSelectedIndex(prev => Math.max(0, prev - 1)); return; }
                            if (e.key === 'ArrowDown') { e.preventDefault(); setSelectedIndex(prev => Math.min(filteredDocs.length - 1, prev + 1)); return; }
                            if (e.key === 'Enter' || e.key === 'Tab') {
                                e.preventDefault();
                                const doc = filteredDocs[selectedIndex];
                                setIngestedFiles(prev => [...prev, { id: doc.id, name: doc.name }]);
                                const cursor = textareaRef.current?.selectionStart || 0;
                                const textBefore = input.slice(0, cursor);
                                const lastAt = textBefore.lastIndexOf('@');
                                if (lastAt !== -1) {
                                    const prefix = textBefore.slice(0, lastAt);
                                    setInput(prefix + input.slice(cursor));
                                }
                                setMentionQuery(null);
                                return;
                            }
                            if (e.key === 'Escape') { setMentionQuery(null); return; }
                        }

                        // Handle Slash Commands
                        if (slashQuery !== null && slashSuggestions.length > 0) {
                            if (e.key === 'ArrowUp') { e.preventDefault(); setSlashSelectedIndex(prev => Math.max(0, prev - 1)); return; }
                            if (e.key === 'ArrowDown') { e.preventDefault(); setSlashSelectedIndex(prev => Math.min(slashSuggestions.length - 1, prev + 1)); return; }
                            if (e.key === 'Enter' || e.key === 'Tab') {
                                e.preventDefault();
                                handleSlashCommandExecute(slashSuggestions[slashSelectedIndex]);
                                return;
                            }
                            if (e.key === 'Escape') { setSlashQuery(null); return; }
                        }

                        if (e.key === 'Enter' && !e.shiftKey) {
                            e.preventDefault();

                            // Don't send if button is disabled (blocks Enter while warming up/restarting)
                            const canSend = !isRestarting && ((input.trim() || attachedImages.length > 0 || ingestedFiles.length > 0 || isStreaming) && (isCloudProvider || modelRunning || isImageMode || isStreaming));
                            if (!canSend) return;

                            if (isImageMode) {
                                setSlashQuery(null);
                                setMentionQuery(null);
                                handleGenerateImage();
                            } else {
                                setSlashQuery(null);
                                setMentionQuery(null);
                                handleSend();
                            }
                        }
                    }}
                    placeholder={
                        isRestarting ? "Warming up model..." : (
                            isCloudProvider
                                ? "Type a message..."
                                : (!modelRunning ? "Starting model..." : (isImageMode ? "Describe the image you want to generate..." : (canSee ? "Type a message..." : (isRagCapable ? "Type a message..." : "Select a Vision model or start Embedder..."))))
                        )
                    }
                    className="flex-1 bg-transparent border-0 focus:ring-0 focus:outline-none resize-none p-2 max-h-32 min-h-[44px]"
                    rows={1}
                    style={{ height: 'auto', minHeight: '44px' }}
                />
            </div>

            {!autoMode && (
                <div className="flex items-center">
                    {isImageMode && (
                        <button
                            onClick={() => setShowImageSettings(!showImageSettings)}
                            className={cn(
                                "px-2 py-1 mr-2 text-[10px] font-black uppercase tracking-widest transition-all duration-300 rounded-md border",
                                showImageSettings ? "bg-primary/10 border-primary/30 text-primary" : "bg-muted/30 border-border/50 text-muted-foreground hover:text-foreground hover:border-border"
                            )}
                        >
                            Settings
                        </button>
                    )}
                    {!isImageMode && (
                        <button onClick={() => setIsWebSearchEnabled(!isWebSearchEnabled)} className={cn("p-2 rounded-xl transition-all duration-300 mr-1", isWebSearchEnabled ? "bg-blue-500 text-white shadow-md shadow-blue-500/20" : "text-muted-foreground hover:bg-muted hover:text-foreground")}
                            title={isWebSearchEnabled ? "Disable Web Search" : "Enable Web Search"}
                        >
                            <Globe className={cn("w-5 h-5", isWebSearchEnabled && "stroke-[2.5]")} />
                        </button>
                    )}
                </div>
            )}

            <button onClick={handleMicClick} className={cn("p-2 rounded-xl transition-all duration-300 mr-1", isRecording ? "bg-red-500 text-white animate-stop-pulse" : "text-muted-foreground hover:bg-muted hover:text-foreground")}
                title={isRecording ? "Stop Recording" : "Voice Input"}
            >
                {isRecording ? <Square className="w-5 h-5 fill-current" /> : <Mic className="w-5 h-5" />}
            </button>

            {!autoMode && !isWebSearchEnabled && (
                <button onClick={() => { setIsImageMode(!isImageMode); }} disabled={isRecording} className={cn("p-2 rounded-xl transition-all duration-300 mr-1", isImageMode ? "bg-primary text-primary-foreground shadow-md shadow-primary/20" : (imageRunning ? "text-primary hover:bg-primary/10" : "text-muted-foreground hover:bg-muted"))}
                    title={isImageMode ? "Cancel Image Mode" : "Switch to Image Generator"}
                >
                    <Palette className={cn("w-5 h-5", isImageMode && "fill-current")} />
                </button>
            )}

            <button
                onClick={() => {
                    if (isStreaming) { handleCancelGeneration(); return; }
                    setSlashQuery(null);
                    setMentionQuery(null);
                    if (isImageMode) {
                        handleGenerateImage();
                    } else {
                        handleSend();
                    }
                }}
                disabled={isRestarting || (!input.trim() && attachedImages.length === 0 && ingestedFiles.length === 0 && !isStreaming) || (!isCloudProvider && !modelRunning && !isImageMode && !isStreaming)}
                className={cn(
                    "p-2 rounded-xl transition-colors disabled:opacity-50",
                    isStreaming ? "bg-destructive text-destructive-foreground hover:bg-destructive/90 animate-stop-pulse shadow-md shadow-red-500/20" :
                        ((input.trim() || attachedImages.length > 0 || ingestedFiles.length > 0) ?
                            (isImageMode ? "bg-primary hover:bg-primary/90 text-primary-foreground" :
                                ((isCloudProvider || modelRunning) ? "bg-primary text-primary-foreground hover:bg-primary/90" : "bg-muted text-muted-foreground"))
                            : "text-muted-foreground hover:bg-muted")
                )}
            >
                {isStreaming ? <Square className="w-5 h-5 fill-current" /> : (isImageMode ? <Palette className="w-5 h-5" /> : <Send className="w-5 h-5" />)}
            </button>

            {!modelRunning && !isImageMode && !isCloudProvider && (
                <button
                    onClick={async () => {
                        if (startServer) {
                            toast.loading("Starting Chat Server...");
                            try {
                                await startServer();
                                toast.dismiss();
                                toast.success("Server Started");
                            } catch (e) { toast.error("Start failed"); }
                        }
                    }}
                    className="p-2 rounded-xl transition-all duration-300 mr-1 text-muted-foreground hover:bg-muted hover:text-foreground"
                    title="Start Server Manually"
                >
                    <Server className="w-5 h-5" />
                </button>
            )}

            {/* Image Generation Settings Popover */}
            <AnimatePresence>
                {showImageSettings && isImageMode && (
                    <motion.div
                        initial={{ opacity: 0, y: 10, scale: 0.95 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 10, scale: 0.95 }}
                        className="absolute bottom-full left-0 right-0 mb-2 p-4 bg-background/95 backdrop-blur-xl border border-border/50 rounded-2xl shadow-2xl z-50 flex flex-col gap-4 origin-bottom"
                    >
                        <div className="flex items-center justify-between border-b border-border/50 pb-2">
                            <span className="text-xs font-black uppercase tracking-widest text-muted-foreground flex items-center gap-2">
                                <Palette className="w-3.5 h-3.5" /> Engine Parameters
                            </span>
                            <button onClick={() => setShowImageSettings(false)} className="text-muted-foreground hover:text-foreground transition-colors">
                                <X className="w-4 h-4" />
                            </button>
                        </div>

                        <div className="grid grid-cols-2 gap-6">
                            <div className="flex flex-col gap-3">
                                <div className="flex justify-between text-[10px] items-center">
                                    <span className="font-bold text-muted-foreground uppercase opacity-70">Guidance Scale</span>
                                    <span className="bg-primary/10 text-primary px-1.5 py-0.5 rounded font-mono font-bold">{cfgScale.toFixed(1)}</span>
                                </div>
                                <input
                                    type="range"
                                    min="1" max="20" step="0.5"
                                    value={cfgScale}
                                    onChange={(e) => setCfgScale(parseFloat(e.target.value))}
                                    className="w-full h-1.5 bg-muted rounded-lg appearance-none cursor-pointer accent-primary"
                                />
                                <p className="text-[9px] text-muted-foreground leading-tight italic">Higher values follow prompt more closely but can cause artifacts.</p>
                            </div>

                            <div className="flex flex-col gap-3">
                                <div className="flex justify-between text-[10px] items-center">
                                    <span className="font-bold text-muted-foreground uppercase opacity-70">Inference Steps</span>
                                    <span className="bg-primary/10 text-primary px-1.5 py-0.5 rounded font-mono font-bold">{imageSteps}</span>
                                </div>
                                <input
                                    type="range"
                                    min="1" max="50" step="1"
                                    value={imageSteps}
                                    onChange={(e) => setImageSteps(parseInt(e.target.value))}
                                    className="w-full h-1.5 bg-muted rounded-lg appearance-none cursor-pointer accent-primary"
                                />
                                <p className="text-[9px] text-muted-foreground leading-tight italic">More steps = better quality but takes longer to generate.</p>
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Mentions Popover */}
            {mentionQuery !== null && filteredDocs.length > 0 && (
                <div className="absolute bottom-full left-0 mb-1 w-80 bg-popover/95 backdrop-blur-xl border border-border/50 rounded-xl shadow-2xl overflow-hidden origin-bottom animate-in fade-in slide-in-from-bottom-2 zoom-in-95 duration-150 ease-out z-50">
                    <div className="px-3 py-1.5 bg-muted/50 text-[10px] font-semibold text-muted-foreground uppercase tracking-wider border-b border-border/50 flex items-center gap-2">
                        <Layers className="w-3 h-3" /> Suggested Documents
                    </div>
                    <div className="max-h-56 overflow-y-auto p-1 scrollbar-thin scrollbar-thumb-border scrollbar-track-transparent">
                        {filteredDocs.map((doc, i) => (
                            <button
                                key={doc.id}
                                className={cn(
                                    "w-full text-left px-3 py-2.5 text-sm rounded-xl flex items-center gap-3 transition-all duration-200",
                                    i === selectedIndex
                                        ? "bg-primary/10 text-primary font-medium translate-x-1"
                                        : "hover:bg-muted/50 text-foreground"
                                )}
                                onClick={() => {
                                    setIngestedFiles(prev => [...prev, { id: doc.id, name: doc.name }]);
                                    const cursor = document.querySelector('textarea')?.selectionStart || 0;
                                    const textBefore = input.slice(0, cursor);
                                    const textAfter = input.slice(cursor);
                                    const lastAt = textBefore.lastIndexOf('@');
                                    if (lastAt !== -1) {
                                        const prefix = textBefore.slice(0, lastAt);
                                        setInput(prefix + textAfter);
                                    }
                                    setMentionQuery(null);
                                }}
                            >
                                <div className={cn(
                                    "p-1.5 rounded-lg",
                                    i === selectedIndex ? "bg-primary/20" : "bg-muted"
                                )}>
                                    <Paperclip className={cn("w-3.5 h-3.5", i === selectedIndex ? "text-primary" : "text-muted-foreground")} />
                                </div>
                                <span className="truncate">{doc.name}</span>
                            </button>
                        ))}
                    </div>
                </div>
            )}

            {/* Slash Commands Popover */}
            <AnimatePresence>
                {slashQuery !== null && (
                    <motion.div
                        initial={{ opacity: 0, y: 10, scale: 0.95 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 10, scale: 0.95 }}
                        className="absolute bottom-full left-0 mb-2 w-72 bg-popover/95 backdrop-blur-xl border border-border/50 rounded-2xl shadow-2xl overflow-hidden z-50 origin-bottom"
                    >
                        <div className="px-3 py-2 bg-muted/30 text-[10px] font-black text-muted-foreground uppercase tracking-tighter border-b border-border/50 flex items-center justify-between">
                            <div className="flex items-center gap-1.5">
                                <Terminal className="w-3 h-3" />
                                <span>Commands</span>
                            </div>
                            <kbd className="px-1.5 py-0.5 rounded bg-muted/50 border border-border/50">TAB</kbd>
                        </div>
                        <div
                            ref={slashCommandContainerRef}
                            className="max-h-64 overflow-y-auto p-1.5 custom-scrollbar"
                        >
                            {(() => {
                                if (slashSuggestions.length === 0) return <div className="p-3 text-xs text-muted-foreground text-center italic">No matches found</div>;

                                return slashSuggestions.map((s, i) => (
                                    <button
                                        key={s.id}
                                        className={cn(
                                            "w-full text-left px-3 py-2.5 text-sm rounded-xl flex items-center justify-between group transition-all duration-200 outline-none",
                                            i === slashSelectedIndex
                                                ? "bg-accent text-foreground font-semibold shadow-sm ring-1 ring-primary/20 translate-x-1"
                                                : "hover:bg-muted text-foreground"
                                        )}
                                        onClick={() => handleSlashCommandExecute(s)}
                                    >
                                        <div className="flex items-center gap-3">
                                            <div className={cn(
                                                "w-6 h-6 rounded-lg flex items-center justify-center transition-colors",
                                                i === slashSelectedIndex ? "bg-primary/20 text-primary" : "bg-muted"
                                            )}>
                                                {s.type === 'command' ? <Terminal className="w-3.5 h-3.5" /> : <Palette className="w-3.5 h-3.5" />}
                                            </div>
                                            <div className="flex flex-col">
                                                <span className="font-semibold tracking-tight leading-none mb-0.5">{s.label}</span>
                                                <span className={cn(
                                                    "text-[10px]",
                                                    i === slashSelectedIndex ? "text-primary/70" : "text-muted-foreground"
                                                )}>
                                                    {s.desc}
                                                </span>
                                            </div>
                                        </div>
                                        {i === slashSelectedIndex && (
                                            <ChevronRight className="w-4 h-4 text-primary" />
                                        )}
                                    </button>
                                ));
                            })()}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
});
