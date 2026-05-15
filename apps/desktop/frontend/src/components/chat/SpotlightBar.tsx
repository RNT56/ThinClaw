import { useState, useRef, useEffect, useCallback, isValidElement } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ArrowUp, Command, Copy, Pin, PinOff, Check } from 'lucide-react';
import { cn } from '../../lib/utils';
import { useChat } from '../../hooks/use-chat';
import { commands } from '../../lib/bindings';
import { directCommands } from '../../lib/generated/direct-commands';
import ReactMarkdown from 'react-markdown';
import rehypeHighlight from 'rehype-highlight';
import remarkGfm from 'remark-gfm';
import { toast } from 'sonner';
import { useModelContext } from '../model-context';
import { useConfig } from '../../hooks/use-config';
import { STATUS_TAG_REGEX } from '../../lib/status-tags';

// Extract text from React nodes for copy functionality
function extractText(node: any): string {
    if (typeof node === 'string' || typeof node === 'number') return String(node);
    if (Array.isArray(node)) return node.map(extractText).join('');
    if (isValidElement(node)) {
        return extractText((node.props as any).children);
    }
    return '';
}

// Simple copy button for code blocks
function CodeCopyButton({ content }: { content: string }) {
    const [copied, setCopied] = useState(false);

    const handleCopy = () => {
        navigator.clipboard.writeText(content);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    return (
        <button
            onClick={handleCopy}
            className="p-1.5 rounded-md transition-all duration-200 bg-background/50 backdrop-blur-md hover:bg-accent hover:text-accent-foreground border border-border/30"
            title="Copy Code"
        >
            {copied ? <Check className="w-3 h-3 text-green-500" /> : <Copy className="w-3 h-3 text-muted-foreground" />}
        </button>
    );
}

export function SpotlightBar() {
    const { messages, isStreaming, sendMessage, clearMessages, modelRunning, currentConversationId, directHistoryDeleteConversation } = useChat();
    const { config: userCfg } = useConfig();
    const { currentModelPath, maxContext } = useModelContext();
    const [input, setInput] = useState("");
    const [isPinned, setIsPinned] = useState(false);
    const [promptHistory, setPromptHistory] = useState<string[]>([]);
    const [historyIndex, setHistoryIndex] = useState(-1);
    const [copiedIndex, setCopiedIndex] = useState<number | null>(null);

    // Resize state - values tracked for resize handlers
    const [, setPanelWidth] = useState(800);
    const [, setChatHeight] = useState(400);
    const [isResizingWidth, setIsResizingWidth] = useState(false);
    const [isResizingHeight, setIsResizingHeight] = useState(false);

    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const scrollRef = useRef<HTMLDivElement>(null);
    const lastConversationId = useRef<string | null>(null);
    const blurTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const panelRef = useRef<HTMLDivElement>(null);


    useEffect(() => {
        if (currentConversationId) {
            lastConversationId.current = currentConversationId;
        }
    }, [currentConversationId]);

    useEffect(() => {
        textareaRef.current?.focus();
    }, []);

    useEffect(() => {
        if (scrollRef.current) {
            scrollRef.current.scrollTo({
                top: scrollRef.current.scrollHeight,
                behavior: 'smooth'
            });
        }
    }, [messages]);

    // Auto-resize textarea
    useEffect(() => {
        if (textareaRef.current) {
            textareaRef.current.style.height = 'auto';
            textareaRef.current.style.height = Math.min(textareaRef.current.scrollHeight, 120) + 'px';
        }
    }, [input]);

    // Auto-resize window based on message state
    const messageCount = messages.length;

    // Minimum height: 150px (input bar ~100px + some padding)
    const MIN_WINDOW_HEIGHT = 150;
    // Height per message (enough to display user prompt + response)
    const MSG_HEIGHT_INCREMENT = 100;
    // Maximum auto-grow height before user can resize further
    const MAX_AUTO_HEIGHT = 850;

    // Refs for resize state
    const resizingState = useRef<{
        startMouseX: number;
        startMouseY: number;
        startWidth: number;
        startHeight: number;
        startWinX: number;
        startWinY: number;
        winCenterX: number;
    } | null>(null);

    // Initializer wrapper for resize start
    const handleResizeStart = async (e: React.MouseEvent, type: 'width' | 'height') => {
        e.preventDefault();
        const screenX = e.screenX;
        const screenY = e.screenY;

        try {
            const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
            const win = getCurrentWebviewWindow();
            const size = await win.innerSize();
            const pos = await win.innerPosition();

            const logicalWidth = size.width / window.devicePixelRatio;
            const logicalHeight = size.height / window.devicePixelRatio;
            const logicalX = pos.x / window.devicePixelRatio;
            const logicalY = pos.y / window.devicePixelRatio;

            resizingState.current = {
                startMouseX: screenX,
                startMouseY: screenY,
                startWidth: logicalWidth,
                startHeight: logicalHeight,
                startWinX: logicalX,
                startWinY: logicalY,
                winCenterX: logicalX + (logicalWidth / 2)
            };

            if (type === 'width') setIsResizingWidth(true);
            else setIsResizingHeight(true);

        } catch (error) {
            console.error("Failed to init resize:", error);
        }
    };

    useEffect(() => {
        const resizeWindow = async () => {
            try {
                const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
                const { LogicalSize } = await import('@tauri-apps/api/dpi');
                const win = getCurrentWebviewWindow();
                const currentSize = await win.innerSize();
                const currentLogicalWidth = currentSize.width / window.devicePixelRatio;
                const currentLogicalHeight = currentSize.height / window.devicePixelRatio;

                if (messageCount === 0) {
                    // Minimize to just the input bar if not already there
                    // We remove forced centering to respect user positioning
                    if (Math.abs(currentLogicalHeight - MIN_WINDOW_HEIGHT) > 1) {
                        await win.setSize(new LogicalSize(Math.max(currentLogicalWidth, 600), MIN_WINDOW_HEIGHT));
                    }
                } else {
                    // Ensure minimum of 650px when there are messages
                    const MIN_CHAT_HEIGHT = 650;
                    const calculatedHeight = MIN_WINDOW_HEIGHT + (messageCount * MSG_HEIGHT_INCREMENT);
                    const targetHeight = Math.max(calculatedHeight, MIN_CHAT_HEIGHT);
                    const cappedHeight = Math.min(targetHeight, MAX_AUTO_HEIGHT);

                    // Only grow, never shrink automatically
                    if (cappedHeight > currentLogicalHeight) {
                        await win.setSize(new LogicalSize(Math.max(currentLogicalWidth, 600), cappedHeight));
                    }
                }
            } catch (e) {
                console.error('Failed to auto-resize window:', e);
            }
        };

        resizeWindow();
    }, [messageCount]);

    // Combined resize handler
    useEffect(() => {
        if (!isResizingWidth && !isResizingHeight) {
            resizingState.current = null;
            return;
        }

        const handleMouseMove = async (e: MouseEvent) => {
            if (!resizingState.current) return;
            const state = resizingState.current;

            try {
                const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
                const { LogicalSize, LogicalPosition } = await import('@tauri-apps/api/dpi');
                const win = getCurrentWebviewWindow();

                if (isResizingWidth) {
                    // Symmetrical resizing relative to window center
                    // We MUST use screen coordinates because moving the window changes clientX
                    // creating a feedback loop.

                    // Window center X (Screen/Logical)
                    const centerX = state.winCenterX;

                    // Mouse Screen X
                    const mouseX = e.screenX;

                    // Distance from center
                    const dist = Math.abs(mouseX - centerX);

                    // New width is twice the distance from center
                    const newWidth = Math.max(dist * 2, 600); // Min width 600

                    // New X = Center X - New Width / 2
                    const newX = centerX - (newWidth / 2);

                    await win.setSize(new LogicalSize(newWidth, state.startHeight));
                    await win.setPosition(new LogicalPosition(newX, state.startWinY));
                    setPanelWidth(newWidth);
                }

                if (isResizingHeight) {
                    // Top resizing (Grow Up / Shrink Down)
                    // Dragging UP (negative delta) -> Increase Height, Decrease Y
                    // Dragging DOWN (positive delta) -> Decrease Height, Increase Y

                    const deltaY = e.screenY - state.startMouseY;

                    // New Height = Start Height - deltaY
                    // (Drag Up -> deltaY < 0 -> Height increases)
                    let newHeight = state.startHeight - deltaY;
                    newHeight = Math.max(newHeight, MIN_WINDOW_HEIGHT);
                    newHeight = Math.min(newHeight, window.screen.availHeight * 0.9);

                    // Effective change (clamped)
                    const effectiveChange = newHeight - state.startHeight;

                    // New Y = Start Y - effectiveChange
                    // (Height increased -> Y must decrease to keep bottom fixed)
                    const newY = state.startWinY - effectiveChange;

                    await win.setSize(new LogicalSize(state.startWidth, newHeight));
                    await win.setPosition(new LogicalPosition(state.startWinX, newY));
                    setChatHeight(newHeight);
                }
            } catch (err) {
                console.error("Resize error:", err);
            }
        };

        const handleMouseUp = () => {
            setIsResizingWidth(false);
            setIsResizingHeight(false);
            resizingState.current = null;
        };

        document.addEventListener('mousemove', handleMouseMove);
        document.addEventListener('mouseup', handleMouseUp);

        return () => {
            document.removeEventListener('mousemove', handleMouseMove);
            document.removeEventListener('mouseup', handleMouseUp);
        };
    }, [isResizingWidth, isResizingHeight]);

    const handleSend = async () => {
        if (!input.trim() || isStreaming) return;

        // Add to prompt history
        setPromptHistory(prev => [input, ...prev.slice(0, 49)]);
        setHistoryIndex(-1);

        const isCloud = userCfg?.selected_chat_provider && userCfg.selected_chat_provider !== "local";

        if (!modelRunning && !isCloud) {
            if (currentModelPath === "auto") {
                toast.info("Initializing neural link...");
                try {
                    const modelsRes = await commands.listModels();
                    if (modelsRes.status === "ok" && modelsRes.data.length > 0) {
                        const localModels = modelsRes.data.filter(m => !m.path.startsWith('http'));
                        const best = localModels.length > 0 ? localModels.sort((a, b) => b.size - a.size)[0] : modelsRes.data[0];
                        await directCommands.directRuntimeStartChatServer(best.path, maxContext, null, null, false, false, false);
                    } else {
                        toast.error("No models found. Please download one in settings.");
                        return;
                    }
                } catch (e) {
                    toast.error(`Start failed: ${String(e)}`);
                    return;
                }
            } else if (currentModelPath) {
                toast.info("Waking up LLM...");
                try {
                    await directCommands.directRuntimeStartChatServer(currentModelPath, maxContext, null, null, false, false, false);
                } catch (e) {
                    toast.error(`Wake failed: ${String(e)}`);
                    return;
                }
            } else {
                toast.error("No brain selected.");
                return;
            }
        }

        sendMessage(input);
        setInput("");
    };

    const handleHide = useCallback(async () => {
        // Only delete if session has meaningful content (more than just user message)
        const hasAssistantResponse = messages.some(m => m.role === 'assistant' && m.content.trim().length > 0);

        if (lastConversationId.current && !hasAssistantResponse) {
            const idToDelete = lastConversationId.current;
            lastConversationId.current = null;
            try {
                await directHistoryDeleteConversation(idToDelete);
            } catch (e) {
                console.error("Failed to delete spotlight conversation:", e);
            }
        }

        if (commands.hideSpotlight) {
            commands.hideSpotlight();
        } else {
            (commands as any).hideSpotlight?.();
        }
    }, [directHistoryDeleteConversation, messages]);

    const handleClear = useCallback(async () => {
        if (lastConversationId.current) {
            const idToDelete = lastConversationId.current;
            lastConversationId.current = null;
            try {
                await directHistoryDeleteConversation(idToDelete);
            } catch (e) {
                console.error("Failed to delete spotlight conversation:", e);
            }
        }
        clearMessages();
        toast.info("Chat purged", { duration: 1000 });
    }, [directHistoryDeleteConversation, clearMessages]);

    const handleCopyMessage = useCallback((content: string, index: number) => {
        navigator.clipboard.writeText(content);
        setCopiedIndex(index);
        setTimeout(() => setCopiedIndex(null), 1500);
    }, []);

    // Debounced blur handler
    useEffect(() => {
        const handleBlur = () => {
            if (isPinned || isResizingWidth || isResizingHeight) return;

            // Clear any existing timeout
            if (blurTimeoutRef.current) {
                clearTimeout(blurTimeoutRef.current);
            }

            // Debounce by 150ms to prevent accidental closes
            blurTimeoutRef.current = setTimeout(() => {
                handleHide();
            }, 150);
        };

        const handleFocus = () => {
            // Cancel pending hide if we regain focus
            if (blurTimeoutRef.current) {
                clearTimeout(blurTimeoutRef.current);
                blurTimeoutRef.current = null;
            }
        };

        window.addEventListener('blur', handleBlur);
        window.addEventListener('focus', handleFocus);

        return () => {
            window.removeEventListener('blur', handleBlur);
            window.removeEventListener('focus', handleFocus);
            if (blurTimeoutRef.current) {
                clearTimeout(blurTimeoutRef.current);
            }
        };
    }, [handleHide, isPinned, isResizingWidth, isResizingHeight]);

    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                handleHide();
            }
            if ((e.metaKey || e.ctrlKey) && e.key === 'l') {
                e.preventDefault();
                handleClear();
            }
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [handleHide, handleClear]);

    const handleInputKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            handleSend();
        }
        // Prompt history navigation
        if (e.key === 'ArrowUp' && !input && promptHistory.length > 0) {
            e.preventDefault();
            const newIndex = Math.min(historyIndex + 1, promptHistory.length - 1);
            setHistoryIndex(newIndex);
            setInput(promptHistory[newIndex]);
        }
        if (e.key === 'ArrowDown' && historyIndex >= 0) {
            e.preventDefault();
            const newIndex = historyIndex - 1;
            if (newIndex < 0) {
                setHistoryIndex(-1);
                setInput("");
            } else {
                setHistoryIndex(newIndex);
                setInput(promptHistory[newIndex]);
            }
        }
    };

    const isCloud = userCfg?.selected_chat_provider && userCfg.selected_chat_provider !== "local";
    const isActive = modelRunning || isCloud;

    // Get provider display name
    const getProviderName = () => {
        if (isCloud && userCfg?.selected_chat_provider) {
            const provider = userCfg.selected_chat_provider;
            if (provider.includes('anthropic')) return 'Claude';
            if (provider.includes('openai')) return 'OpenAI';
            if (provider.includes('gemini')) return 'Gemini';
            if (provider.includes('groq')) return 'Groq';
            if (provider.includes('openrouter')) return 'Router';
            return 'Cloud';
        }
        return 'Local';
    };

    // Determine if we should allow resizing (only when there are messages)
    const canResize = messages.length > 0;

    return (
        <div className="fixed inset-0 flex items-end justify-center pointer-events-none select-none">
            <motion.div
                ref={panelRef}
                initial={{ opacity: 0, y: 30 }}
                animate={{ opacity: 1, y: 0 }}
                className={cn(
                    "pointer-events-auto flex flex-col relative w-full",
                    canResize ? "h-full" : "h-auto"
                )}
            >
                {/* Main Panel - Single unified surface */}
                <div
                    className={cn(
                        "relative flex flex-col bg-background/95 border border-border/50 overflow-hidden shadow-2xl rounded-[24px]",
                        canResize ? "flex-1" : "mt-auto mb-8"
                    )}
                    style={{
                        backdropFilter: 'blur(40px) saturate(200%)',
                        WebkitBackdropFilter: 'blur(40px) saturate(200%)',
                    }}
                >
                    {/* Resize handle - Top (height) - only show when there are messages */}
                    {canResize && (
                        <div
                            className="absolute top-0 left-1/2 -translate-x-1/2 w-16 h-3 cursor-ns-resize z-20 flex items-center justify-center group"
                            onMouseDown={(e) => handleResizeStart(e, 'height')}
                        >
                            <div className="w-8 h-1 rounded-full bg-border/30 group-hover:bg-border transition-colors" />
                        </div>
                    )}

                    {/* Drag handle - allows moving the window */}
                    <div
                        className="relative z-10 h-6 w-full cursor-default flex items-center justify-center shrink-0 hover:bg-border/5 transition-colors"
                    >
                        <div className="w-12 h-1 rounded-full bg-border/40 pointer-events-none" />
                    </div>

                    {/* Chat Area */}
                    <AnimatePresence mode="popLayout">
                        {messages.length > 0 && (
                            <motion.div
                                key="chat-area"
                                initial={{ height: 0, opacity: 0 }}
                                animate={{ height: 'auto', opacity: 1 }}
                                exit={{ height: 0, opacity: 0 }}
                                className="relative flex-1 overflow-y-auto spotlight-scroll border-b border-border/30"
                                ref={scrollRef}
                                role="log"
                                aria-live="polite"
                                aria-label="Chat messages"
                            >
                                <div className="px-6 py-6 flex flex-col gap-5">
                                    {messages.map((m, i) => (
                                        <div key={i} className={cn("flex w-full group", m.role === 'user' ? "justify-end" : "justify-start")}>
                                            <div className={cn(
                                                "relative max-w-[95%] px-4 py-3 rounded-[16px] transition-colors",
                                                m.role === 'user'
                                                    ? "bg-primary/20 text-foreground"
                                                    : "bg-muted/50 text-foreground"
                                            )}>
                                                {m.role === 'user' ? (
                                                    <p className="text-[15px] select-text">{m.content}</p>
                                                ) : (
                                                    <div className="prose prose-sm dark:prose-invert select-text max-w-none
                                                        text-foreground prose-headings:text-foreground prose-p:text-foreground prose-strong:text-foreground prose-li:text-foreground
                                                        prose-headings:font-semibold prose-h1:text-lg prose-h2:text-base prose-h3:text-sm
                                                        prose-p:leading-relaxed prose-p:my-2
                                                        prose-li:my-0.5
                                                        prose-pre:bg-[hsl(var(--hljs-bg))] prose-pre:border prose-pre:border-border/50 prose-pre:rounded-xl prose-pre:my-3 prose-pre:relative prose-pre:group/code
                                                        prose-code:bg-[hsl(var(--hljs-bg))] prose-code:px-1.5 prose-code:py-0.5 prose-code:rounded-md prose-code:before:content-none prose-code:after:content-none prose-code:font-mono prose-code:text-[0.9em]
                                                    ">
                                                        <ReactMarkdown
                                                            remarkPlugins={[remarkGfm]}
                                                            rehypePlugins={[rehypeHighlight]}
                                                            components={{
                                                                pre: ({ node, children, ...props }) => {
                                                                    let codeText = "";
                                                                    if (isValidElement(children)) {
                                                                        codeText = extractText((children.props as any).children).replace(/\n$/, '');
                                                                    }

                                                                    return (
                                                                        <pre {...props} className="relative group/code">
                                                                            <div className="absolute top-2 right-2 opacity-0 group-hover/code:opacity-100 transition-opacity z-10">
                                                                                <CodeCopyButton content={codeText} />
                                                                            </div>
                                                                            {children}
                                                                        </pre>
                                                                    );
                                                                }
                                                            }}
                                                        >
                                                            {m.content.replace(STATUS_TAG_REGEX, '').replace(/<think>[\s\S]*?<\/think>/g, '').trim()}
                                                        </ReactMarkdown>
                                                    </div>
                                                )}
                                                {/* Copy button for assistant messages */}
                                                {m.role === 'assistant' && (
                                                    <button
                                                        onClick={() => handleCopyMessage(m.content, i)}
                                                        className="absolute -right-2 top-2 p-1.5 rounded-lg bg-background/80 border border-border/50 opacity-0 group-hover:opacity-100 transition-opacity hover:bg-muted"
                                                        aria-label="Copy message"
                                                    >
                                                        {copiedIndex === i ? (
                                                            <Check className="w-3 h-3 text-green-500" />
                                                        ) : (
                                                            <Copy className="w-3 h-3 text-muted-foreground" />
                                                        )}
                                                    </button>
                                                )}
                                            </div>
                                        </div>
                                    ))}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>

                    {/* Jumping dots loader while waiting for LLM response */}
                    <AnimatePresence>
                        {isStreaming && messages.length > 0 &&
                            messages[messages.length - 1]?.role === 'assistant' &&
                            (messages[messages.length - 1]?.content === "" || !messages[messages.length - 1]?.content) && (
                                <motion.div
                                    initial={{ opacity: 0, height: 0 }}
                                    animate={{ opacity: 1, height: 'auto' }}
                                    exit={{ opacity: 0, height: 0 }}
                                    className="flex justify-start px-6 py-3 border-b border-border/30"
                                >
                                    <div className="px-4 py-3 rounded-[16px] bg-muted/50 flex items-center gap-1.5">
                                        <span className="w-2 h-2 rounded-full bg-primary/60 animate-bounce" style={{ animationDelay: '0ms' }} />
                                        <span className="w-2 h-2 rounded-full bg-primary/60 animate-bounce" style={{ animationDelay: '150ms' }} />
                                        <span className="w-2 h-2 rounded-full bg-primary/60 animate-bounce" style={{ animationDelay: '300ms' }} />
                                    </div>
                                </motion.div>
                            )}
                    </AnimatePresence>


                    <div
                        className="flex items-end gap-3 px-5 py-4 min-h-[64px]"
                    >
                        {/* Status Indicator with Provider Badge */}
                        <div className="flex items-center gap-2 pb-2 flex-shrink-0">
                            <div
                                className={cn(
                                    "w-2 h-2 rounded-full transition-all duration-500",
                                    isActive ? "bg-green-500" : "bg-muted-foreground/30"
                                )}
                                aria-label={isActive ? "Model active" : "Model inactive"}
                            />
                            <span className="text-[10px] uppercase font-bold tracking-wide text-muted-foreground/60">
                                {getProviderName()}
                            </span>
                        </div>

                        <textarea
                            ref={textareaRef}
                            value={input}
                            onChange={(e) => setInput(e.target.value)}
                            onKeyDown={handleInputKeyDown}
                            placeholder="Whisper something..."
                            rows={1}
                            className="flex-1 bg-transparent text-foreground text-[15px] outline-none placeholder:text-muted-foreground/40 resize-none min-h-[24px] max-h-[120px] py-1"
                            aria-label="Spotlight chat input"
                        />

                        <div className="flex items-center gap-2 pb-1">
                            {/* Pin Toggle */}
                            <button
                                onClick={() => setIsPinned(!isPinned)}
                                className={cn(
                                    "p-1.5 rounded-lg transition-colors",
                                    isPinned
                                        ? "bg-primary/20 text-primary"
                                        : "text-muted-foreground/40 hover:text-muted-foreground hover:bg-muted/50"
                                )}
                                aria-label={isPinned ? "Unpin spotlight" : "Pin spotlight"}
                                title={isPinned ? "Click to auto-hide on blur" : "Click to keep visible"}
                            >
                                {isPinned ? <PinOff className="w-4 h-4" /> : <Pin className="w-4 h-4" />}
                            </button>

                            <AnimatePresence>
                                {input.trim() && !isStreaming && (
                                    <motion.button
                                        initial={{ opacity: 0, scale: 0.8 }}
                                        animate={{ opacity: 1, scale: 1 }}
                                        exit={{ opacity: 0, scale: 0.8 }}
                                        onClick={handleSend}
                                        className="w-8 h-8 rounded-lg bg-primary flex items-center justify-center text-primary-foreground hover:bg-primary/90 transition-colors"
                                        aria-label="Send message"
                                    >
                                        <ArrowUp className="w-4 h-4" />
                                    </motion.button>
                                )}
                            </AnimatePresence>
                            {!input.trim() && !isStreaming && (
                                <div className="flex items-center gap-1 text-muted-foreground/30">
                                    <Command className="w-3 h-3" />
                                    <span className="text-[10px] uppercase font-bold">L</span>
                                </div>
                            )}
                        </div>
                    </div>
                </div>

                {/* Resize handles - Left and Right (width) - only show when there are messages */}
                {canResize && (
                    <>
                        <div
                            className="absolute left-0 top-0 bottom-0 w-3 cursor-ew-resize z-20 flex items-center justify-center group hover:bg-border/10 transition-colors"
                            onMouseDown={(e) => handleResizeStart(e, 'width')}
                        >
                            <div className="w-0.5 h-12 rounded-full bg-border/30 group-hover:bg-border transition-colors" />
                        </div>
                        <div
                            className="absolute right-0 top-0 bottom-0 w-3 cursor-ew-resize z-20 flex items-center justify-center group hover:bg-border/10 transition-colors"
                            onMouseDown={(e) => handleResizeStart(e, 'width')}
                        >
                            <div className="w-0.5 h-12 rounded-full bg-border/30 group-hover:bg-border transition-colors" />
                        </div>
                    </>
                )}
            </motion.div>

            <style dangerouslySetInnerHTML={{
                __html: `
                .spotlight-scroll::-webkit-scrollbar { width: 0px; }
                .spotlight-scroll { mask-image: linear-gradient(to bottom, transparent, black 20px); -webkit-mask-image: linear-gradient(to bottom, transparent, black 20px); }
            `}} />
        </div>
    );
}
