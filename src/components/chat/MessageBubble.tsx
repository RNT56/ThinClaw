import ReactMarkdown from 'react-markdown';
import { Check, Copy, Paperclip, Download, Maximize2, Loader2, Pencil, Sparkles, CheckCircle2, Image as ImageIcon } from 'lucide-react';
import rehypeHighlight from 'rehype-highlight';
import remarkGfm from 'remark-gfm';
import { cn } from '../../lib/utils';
import { Message, commands, WebSearchResult } from '../../lib/bindings';
import { useEffect, useState, isValidElement, useRef } from 'react';
import DOMPurify from 'dompurify';
import { listen } from '@tauri-apps/api/event';
import { readFile } from '@tauri-apps/plugin-fs';
import { toast } from 'sonner';
import { WebSearchBubble, WebStatusState, WebSource } from './WebSearchBubble';
import { StatusIndicator } from './StatusIndicator'; // New Import
import { createPortal } from 'react-dom';

function extractText(node: any): string {
    if (typeof node === 'string' || typeof node === 'number') return String(node);
    if (Array.isArray(node)) return node.map(extractText).join('');
    if (isValidElement(node)) {
        return extractText((node.props as any).children);
    }
    return '';
}

const ImageAttachment = ({ id }: { id: string }) => {
    const [src, setSrc] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(false);
    const [error, setError] = useState(false);
    const [copied, setCopied] = useState(false);
    const [isFullscreen, setIsFullscreen] = useState(false);
    const [overrideId, setOverrideId] = useState<string | null>(null);

    // Pending Generation States
    const [progress, setProgress] = useState(0);
    const [statusText, setStatusText] = useState("Initializing...");
    const [elapsed, setElapsed] = useState(0);
    const startTimeRef = useRef(Date.now());
    const lastUpdateRef = useRef(0);

    // Safety States
    const [isReadyToView, setIsReadyToView] = useState(false);
    const [userRequestedView, setUserRequestedView] = useState(false);

    useEffect(() => {
        if (id === "pending_generation") {
            startTimeRef.current = Date.now();
            let unlistenProgress: Promise<() => void> | undefined;
            let unlistenSuccess: Promise<() => void> | undefined;

            unlistenProgress = listen<string>('image_gen_progress', (event) => {
                const text = event.payload;
                const now = Date.now();

                if (now - lastUpdateRef.current < 50) return;
                lastUpdateRef.current = now;

                const barMatch = text.match(/\|\s*(\d+)\/(\d+)/);
                if (barMatch) {
                    const current = parseInt(barMatch[1]);
                    const total = parseInt(barMatch[2]);
                    if (total > 0) {
                        setProgress((current / total) * 100);
                        setStatusText(`Generating: ${current}/${total}`);
                    }
                } else if (text.toLowerCase().includes("save result")) {
                    setStatusText("Finalizing...");
                }
            });

            const interval = setInterval(() => {
                setElapsed(Math.floor((Date.now() - startTimeRef.current) / 1000));
            }, 1000);

            unlistenSuccess = listen<any>('image_gen_success', (event) => {
                const { original_id, final_id } = event.payload;
                // Transition to real ID ONLY if it matches this pending request
                // Note: "pending_generation" is a generic ID, so we might need a unique ID for the request itself if we support parallel generations.
                // However, for now, we assume single generation flow or that the backend returns the specific UUID we started with? 
                // Actually, 'id' prop passes "pending_generation". 
                // The logical issue: How do we know WHICH "pending_generation" component corresponds to the success event?
                // The backend emits 'original_id'. But if we passed "pending_generation" to the backend, it returns that. 
                // Codebase check: The backend generates a UUID.

                // Wait, in `ImageBubble`, the `id` passed to `ImageAttachment` is usually the database ID or "pending_generation".
                // When we start generation, we create a placeholder message.
                // The `original_id` payload usually matches what?
                // If I look at `image_gen.rs` or `MessageBubble`... 

                // Let's just restore the check that was there before.
                if (id === original_id || id === "pending_generation") {
                    setOverrideId(final_id);
                }
            });

            return () => {
                if (unlistenProgress) unlistenProgress.then(f => f());
                if (unlistenSuccess) unlistenSuccess.then(f => f());
                clearInterval(interval);
            };
        } else {
            // Not pending = Ready to view
            setIsReadyToView(true);
        }
    }, [id]);

    const loadContent = async () => {
        if (id === "pending_generation") return;
        setIsLoading(true);
        try {
            const res = await commands.getImagePath(id);
            if (res.status === "ok") {
                const contents = await readFile(res.data);

                // Determine MIME type from extension
                const ext = res.data.split('.').pop()?.toLowerCase();
                const mimeType = ext === 'jpg' || ext === 'jpeg' ? 'image/jpeg' : 'image/png';

                const blob = new Blob([contents], { type: mimeType });
                const assetUrl = URL.createObjectURL(blob);
                setSrc(assetUrl);
            } else {
                setError(true);
            }
        } catch (e) {
            setError(true);
        } finally {
            setIsLoading(false);
        }
    };

    const handleViewClick = () => {
        setUserRequestedView(true);
        loadContent();
    };

    const handleCopy = async (e: React.MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (!src) return;

        try {
            // Safari/WebKit (Tauri on macOS) requires a direct Promise for the blob in ClipboardItem
            // to avoid losing the "user activation" during the async fetch.
            const item = new ClipboardItem({
                "image/png": fetch(src).then(async (response) => {
                    const blob = await response.blob();
                    return new Blob([blob], { type: "image/png" });
                })
            });

            await navigator.clipboard.write([item]);
            setCopied(true);
            toast.success("Image copied to clipboard");
            setTimeout(() => setCopied(false), 2000);
        } catch (err) {
            console.error('Failed to copy image:', err);
            toast.error("Clipboard Error: Access blocked or unsupported format.");
        }
    };

    const handleDownload = (e: React.MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (!src) return;
        const a = document.createElement('a');
        a.href = src;
        a.download = `generated-${id}.png`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        toast.success("Image saved to Downloads");
    };

    if (overrideId) {
        return <ImageAttachment id={overrideId} />;
    }

    if (id === "pending_generation") {
        return (
            <div className="w-64 h-64 bg-card rounded-xl flex flex-col items-center justify-center gap-4 border border-border/50 relative overflow-hidden shadow-sm">
                <div className="absolute inset-0 bg-muted/10" />
                <div className="relative w-12 h-12 rounded-full bg-primary/10 flex items-center justify-center border border-primary/20 shadow-inner group-hover:scale-110 transition-transform duration-500">
                    <ImageIcon className="w-6 h-6 text-primary animate-pulse" />
                </div>
                <div className="relative flex flex-col items-center gap-1 z-10">
                    <span className="text-sm font-medium text-foreground tracking-tight">{statusText}</span>
                    <span className="text-xs text-muted-foreground font-mono">{elapsed}s elapsed</span>
                </div>
                <div className="absolute bottom-0 left-0 right-0 h-1 bg-muted overflow-hidden">
                    <div
                        className="h-full bg-primary transition-all duration-300 ease-out shadow-[0_0_10px_rgba(var(--primary),0.5)]"
                        style={{ width: `${Math.max(5, progress)}%` }}
                    />
                </div>
            </div>
        );
    }

    // Ready-to-View State (Safety Wall)
    if (isReadyToView && !userRequestedView) {
        return (
            <div className="w-64 h-64 bg-card rounded-xl flex flex-col items-center justify-center gap-4 border border-border/50 relative overflow-hidden shadow-sm animate-in fade-in zoom-in-95 duration-300">
                <div className="absolute inset-0 bg-muted/10" />
                <div className="relative mb-2">
                    <div className="absolute inset-0 bg-green-500/20 blur-2xl rounded-full animate-pulse" />
                    <div className="relative bg-gradient-to-br from-green-400 to-emerald-600 p-4 rounded-2xl shadow-lg ring-1 ring-white/20 animate-in zoom-in-50 duration-500">
                        <Sparkles className="w-8 h-8 text-white" />
                        <div className="absolute -bottom-1 -right-1 bg-white rounded-full p-0.5 shadow-sm">
                            <CheckCircle2 className="w-4 h-4 text-emerald-600" />
                        </div>
                    </div>
                </div>
                <p className="text-sm font-medium text-foreground">Generation Complete</p>
                <button
                    onClick={handleViewClick}
                    className="px-4 py-2 bg-primary text-primary-foreground text-xs font-bold rounded-md shadow hover:bg-primary/90 transition-colors cursor-pointer z-50"
                >
                    View Image
                </button>
            </div>
        );
    }

    if (error) {
        return (
            <div className="w-64 h-64 bg-muted/30 rounded-lg flex flex-col items-center justify-center gap-2 border border-border/50 text-muted-foreground p-4 text-center">
                <span className="text-xs">Image not found</span>
            </div>
        );
    }

    if (!src) {
        return (
            <div className="w-64 h-64 bg-muted/30 rounded-lg flex flex-col items-center justify-center gap-3 border border-border/50 relative overflow-hidden group cursor-pointer hover:bg-muted/50 transition-colors"
                onClick={loadContent}
            >
                {isLoading ? (
                    <Loader2 className="w-8 h-8 text-primary/50 animate-spin" />
                ) : (
                    <>
                        <Download className="w-8 h-8 text-primary/50 group-hover:text-primary transition-colors" />
                        <span className="text-sm font-medium text-muted-foreground/70 group-hover:text-foreground">Click to View Image</span>
                    </>
                )}
            </div>
        );
    }

    return (
        <>
            <div className="relative group inline-block">
                <img
                    src={src}
                    alt="attachment"
                    onError={() => setError(true)}
                    className="max-w-sm rounded-lg border border-border/50 shadow-sm transition-transform cursor-pointer bg-black/5"
                    onClick={() => setIsFullscreen(true)}
                />
                <div className="absolute bottom-2 right-2 flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity duration-200 z-50 pointer-events-auto">
                    <button
                        onClick={handleCopy}
                        className="p-2 bg-black/70 hover:bg-black/90 text-white rounded-md backdrop-blur-md transition-all shadow-md border border-white/20 hover:scale-110 active:scale-95"
                        title="Copy Image"
                    >
                        {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
                    </button>
                    <button
                        onClick={handleDownload}
                        className="p-2 bg-black/70 hover:bg-black/90 text-white rounded-md backdrop-blur-md transition-all shadow-md border border-white/20 hover:scale-110 active:scale-95"
                        title="Download"
                    >
                        <Download className="w-4 h-4" />
                    </button>
                    <button
                        onClick={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            setIsFullscreen(true);
                        }}
                        className="p-2 bg-black/70 hover:bg-black/90 text-white rounded-md backdrop-blur-md transition-all shadow-md border border-white/20 hover:scale-110 active:scale-95"
                        title="Full View"
                    >
                        <Maximize2 className="w-4 h-4" />
                    </button>
                </div>
            </div>
            {isFullscreen && createPortal(
                <div
                    className="fixed inset-0 z-[9999] bg-black/90 backdrop-blur-md flex items-center justify-center p-8 animate-in fade-in duration-200"
                    onClick={() => setIsFullscreen(false)}
                >
                    <div className="relative max-w-full max-h-full" onClick={e => e.stopPropagation()}>
                        <img
                            src={src}
                            alt="Full view"
                            className="max-w-full max-h-[90vh] rounded-lg shadow-2xl object-contain cursor-default"
                        />
                        <button
                            onClick={() => setIsFullscreen(false)}
                            className="absolute -top-12 right-0 p-2 text-white/70 hover:text-white hover:bg-white/10 rounded-full transition-colors"
                        >
                            <Loader2 className="w-6 h-6 rotate-45" />
                        </button>
                    </div>
                </div>,
                document.body
            )}
        </>
    );
}

// Helper to separate thought process from content
function parseThoughts(content: string) {
    const thinkRegex = /<think>([\s\S]*?)<\/think>/g;
    const thoughts: string[] = [];
    const cleanContent = content.replace(thinkRegex, (_, group) => {
        thoughts.push(group.trim());
        return "";
    }).trim();

    return { thoughts, content: cleanContent || content };
}

// Updated Helper to separate content and status tags
function parseContent(content: string) {
    const parts: { type: 'text' | 'status', content?: string, props?: any }[] = [];
    const regex = /<scrappy_status\s+type="([^"]+)"(?:\s+query="([^"]*)")?\s*\/>/g;

    let lastIndex = 0;
    let match;

    while ((match = regex.exec(content)) !== null) {
        if (match.index > lastIndex) {
            parts.push({ type: 'text', content: content.slice(lastIndex, match.index) });
        }
        parts.push({
            type: 'status',
            props: { type: match[1], query: match[2] }
        });
        lastIndex = regex.lastIndex;
    }

    if (lastIndex < content.length) {
        parts.push({ type: 'text', content: content.slice(lastIndex) });
    }

    // Check for [Stopped] at the end of the last part
    const lastPart = parts[parts.length - 1];
    if (lastPart && lastPart.type === 'text' && lastPart.content?.trim().endsWith('[Stopped]')) {
        // modify content to remove [Stopped]
        lastPart.content = lastPart.content.replace(/\[Stopped\]\s*$/, '');
        parts.push({ type: 'status', props: { type: 'stopped' } });
    }

    return parts;
}


function CopyButton({ content, className }: { content: string, className?: string }) {
    const [copied, setCopied] = useState(false);

    const handleCopy = () => {
        navigator.clipboard.writeText(content);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    return (
        <button
            onClick={handleCopy}
            className={cn("p-1.5 rounded-md transition-all duration-200 border bg-background/80 backdrop-blur-sm shadow-sm hover:bg-accent hover:text-accent-foreground", className)}
            title="Copy Text"
        >
            {copied ? <Check className="w-3.5 h-3.5 text-green-500" /> : <Copy className="w-3.5 h-3.5 text-muted-foreground" />}
        </button>
    );
}

type ExtendedMessage = Message & {
    id?: string;
    web_search_results?: WebSearchResult[] | null;
    searchStatus?: WebStatusState;
    searchMessage?: string;
    is_summary?: boolean | null;
    original_messages?: Message[] | null;
};

export function MessageBubble({ message, conversationId, isLastUser, onResend }: { message: ExtendedMessage, conversationId: string | null, isLast?: boolean, isLastUser?: boolean, onResend?: (id: string, content: string) => void }) {
    const isUser = message.role === 'user';
    const { thoughts, content } = !isUser ? parseThoughts(message.content) : { thoughts: [], content: message.content };
    const rawContent = isUser ? message.content : content;
    const sanitizedContent = DOMPurify.sanitize(rawContent);

    const [isEditing, setIsEditing] = useState(false);
    const [editContent, setEditContent] = useState(message.content);
    const [showOriginals, setShowOriginals] = useState(false);

    // Update editContent if message changes (e.g. streaming update, though User messages rarely change)
    useEffect(() => {
        if (isUser && !isEditing) {
            setEditContent(message.content);
        }
    }, [message.content, isEditing, isUser]);

    const [webSources, setWebSources] = useState<WebSource[]>([]);
    const [searchStatus, setSearchStatus] = useState<WebStatusState>('idle');
    const [statusMessage, setStatusMessage] = useState("");

    // Sync state with props
    useEffect(() => {
        if (isUser) {
            setWebSources([]);
            setSearchStatus('idle');
            setStatusMessage("");
            return;
        }

        // If props provide search results, use them
        if (message.web_search_results && message.web_search_results.length > 0) {
            setWebSources(message.web_search_results);
            setSearchStatus('done');
            setStatusMessage("");
        } else if (message.searchStatus) {
            // Live status from activeJob mapping
            setSearchStatus(message.searchStatus);
            setStatusMessage(message.searchMessage || "");
            setWebSources([]);
        } else {
            // Reset for historical messages or chats without search
            setWebSources([]);
            setSearchStatus('idle');
            setStatusMessage("");
        }
    }, [message.web_search_results, message.searchStatus, message.searchMessage, isUser]);

    if (message.is_summary) {
        return (
            <div className="w-full flex flex-col items-center my-6 group animate-in fade-in duration-500">
                <div className="flex items-center gap-2 text-[10px] uppercase tracking-wider text-muted-foreground/70 bg-muted/20 px-4 py-1.5 rounded-full border border-border/30 backdrop-blur-sm transition-all hover:bg-muted/40 hover:text-foreground hover:border-border/50 shadow-sm">
                    <Sparkles className="w-3 h-3 text-amber-500" />
                    <span>{message.content}</span>
                    {message.original_messages && message.original_messages.length > 0 && (
                        <>
                            <div className="w-px h-3 bg-border/50 mx-1" />
                            <button
                                onClick={() => setShowOriginals(!showOriginals)}
                                className="hover:text-primary transition-colors font-semibold"
                            >
                                {showOriginals ? "Hide History" : "View History"}
                            </button>
                        </>
                    )}
                </div>
                {showOriginals && message.original_messages && (
                    <div className="w-full mt-6 flex flex-col gap-4 pl-4 md:pl-8 border-l-2 border-primary/10 relative">
                        <div className="absolute top-0 left-[-1px] w-full h-8 bg-gradient-to-b from-background to-transparent z-10" />
                        <div className="absolute bottom-0 left-[-1px] w-full h-8 bg-gradient-to-t from-background to-transparent z-10" />

                        {message.original_messages.map((m, i) => (
                            <MessageBubble
                                key={`orig-${i}`}
                                message={m as ExtendedMessage}
                                conversationId={conversationId}
                            />
                        ))}

                        <div className="flex justify-center mt-4">
                            <button
                                onClick={() => setShowOriginals(false)}
                                className="text-xs text-muted-foreground/50 hover:text-primary transition-colors flex items-center gap-1"
                            >
                                <CheckCircle2 className="w-3 h-3" /> Close History
                            </button>
                        </div>
                    </div>
                )}
            </div>
        );
    }

    return (
        <div className={cn("flex flex-col w-full animate-in fade-in slide-in-from-bottom-2 duration-300", isUser ? "items-end" : "items-start", isLastUser && "mb-8")}>
            <div className={cn(
                "group relative max-w-[85%] md:max-w-[75%] rounded-2xl px-5 py-4 shadow-sm overflow-visible",
                isUser ? "bg-primary text-primary-foreground rounded-br-sm" : "bg-card border border-border/50 rounded-bl-sm"
            )}>
                {thoughts.length > 0 && !isUser && (
                    <div className="mb-4">
                        {thoughts.map((thought, i) => (
                            <details key={i} className="group/thought text-sm bg-muted/30 border border-border/50 rounded-lg overflow-hidden transition-all duration-200 open:bg-muted/50">
                                <summary className="flex items-center gap-2 px-3 py-2 cursor-pointer select-none text-muted-foreground hover:text-foreground list-none font-medium">
                                    <div className="w-1.5 h-1.5 rounded-full bg-blue-500/50"></div>
                                    <span>Thought Process</span>
                                    <span className="opacity-0 group-open/thought:opacity-100 transition-opacity text-[10px] uppercase tracking-wider ml-auto">Expanded</span>
                                </summary>
                                <div className="px-4 pb-3 pt-1 text-muted-foreground/90 whitespace-pre-wrap leading-relaxed border-t border-border/30 italic text-[0.95em]">
                                    {thought}
                                </div>
                            </details>
                        ))}
                    </div>
                )}

                {!isUser && (
                    <WebSearchBubble
                        status={searchStatus}
                        message={statusMessage}
                        sources={webSources}
                        conversationId={conversationId}
                    />
                )}

                {message.attached_docs && message.attached_docs.length > 0 && (
                    <div className="flex flex-col gap-2 mb-3">
                        {/* Documents */}
                        <div className="flex flex-wrap gap-2">
                            {message.attached_docs.map((doc, i) => {
                                return (
                                    <div key={i} className="flex items-center gap-2 p-2 rounded-lg bg-secondary/50 border border-border/50 transition-colors hover:bg-secondary">
                                        <div className="p-1.5 bg-background rounded-md shadow-sm">
                                            <Paperclip className="w-3.5 h-3.5 text-orange-500" />
                                        </div>
                                        <span className="text-xs font-medium max-w-[200px] truncate" title={doc.name}>
                                            {doc.name}
                                        </span>
                                    </div>
                                );
                            })}
                        </div>
                    </div>
                )}

                {/* Images */}
                {message.images && message.images.length > 0 && (
                    <div className="grid grid-cols-2 gap-2 mb-3">
                        {message.images.map((imgId, i) => (
                            <ImageAttachment key={`${imgId}-${i}`} id={imgId} />
                        ))}
                    </div>
                )}

                {
                    isUser ? (
                        isEditing ? (
                            <div className="flex flex-col gap-2 w-full min-w-[200px]">
                                <textarea
                                    value={editContent}
                                    onChange={(e) => setEditContent(e.target.value)}
                                    className="bg-transparent text-primary-foreground rounded-lg p-2 min-h-[100px] resize-y focus:outline-none border border-white/20 text-sm font-normal"
                                    autoFocus
                                    onKeyDown={(e) => {
                                        if (e.key === 'Enter' && !e.shiftKey) {
                                            e.preventDefault();
                                            setIsEditing(false);
                                            if (message.id) onResend?.(message.id, editContent);
                                        }
                                    }}
                                />
                                <div className="flex justify-end gap-2 mt-2">
                                    <button
                                        onClick={() => { setIsEditing(false); setEditContent(message.content); }}
                                        className="p-1 px-3 rounded bg-black/20 hover:bg-black/30 text-xs text-white/80 transition-colors"
                                    >
                                        Cancel
                                    </button>
                                    <button
                                        onClick={() => { setIsEditing(false); if (message.id) onResend?.(message.id, editContent); }}
                                        className="p-1 px-3 rounded bg-primary-foreground text-primary font-semibold text-xs hover:opacity-90 transition-opacity shadow-sm flex items-center gap-1"
                                    >
                                        <span>Send</span>
                                        <div className="w-3 h-3 text-primary rotate-90">
                                            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round"><path d="M12 19V5" /><path d="M5 12l7-7 7 7" /></svg>
                                        </div>
                                    </button>
                                </div>
                            </div>
                        ) : (
                            <div className="relative group/content">
                                <p className="whitespace-pre-wrap leading-relaxed pb-2">{sanitizedContent}</p>
                            </div>
                        )
                    ) : (
                        <div className="prose prose-sm max-w-none break-words dark:prose-invert
                            text-foreground prose-headings:text-foreground prose-p:text-foreground prose-strong:text-foreground prose-li:text-foreground
                            prose-headings:font-semibold prose-h1:text-xl prose-h2:text-lg prose-h3:text-base prose-h1:mt-6 prose-h2:mt-5
                            prose-p:leading-loose prose-p:my-4
                            prose-li:my-1
                            prose-pre:bg-[hsl(var(--hljs-bg))] prose-pre:border prose-pre:border-border/50 prose-pre:rounded-xl prose-pre:my-4 prose-pre:relative prose-pre:group/code
                            prose-code:bg-[hsl(var(--hljs-bg))] prose-code:px-1.5 prose-code:py-0.5 prose-code:rounded-md prose-code:before:content-none prose-code:after:content-none prose-code:font-mono prose-code:text-[0.9em]
                        ">
                            {parseContent(sanitizedContent as string).map((part, idx) => {
                                if (part.type === 'status') {
                                    return <StatusIndicator key={idx} type={part.props.type} query={part.props.query} />;
                                }
                                if (!part.content) return null;

                                return (
                                    <ReactMarkdown
                                        key={idx}
                                        remarkPlugins={[remarkGfm]}
                                        rehypePlugins={[rehypeHighlight]}
                                        components={{
                                            a: ({ node, ...props }) => (
                                                <a
                                                    {...props}
                                                    target="_blank"
                                                    rel="noopener noreferrer"
                                                    className="text-primary hover:underline cursor-pointer relative z-10"
                                                    onClick={(e) => e.stopPropagation()}
                                                />
                                            ),
                                            pre: ({ node, children, ...props }) => {
                                                let codeText = "";
                                                if (isValidElement(children)) {
                                                    codeText = extractText((children.props as any).children).replace(/\n$/, '');
                                                }

                                                return (
                                                    <pre {...props} className="relative group/code">
                                                        <div className="absolute top-2 right-2 opacity-0 group-hover/code:opacity-100 transition-opacity z-10">
                                                            <CopyButton content={codeText} />
                                                        </div>
                                                        {children}
                                                    </pre>
                                                )
                                            }
                                        }}
                                    >
                                        {part.content}
                                    </ReactMarkdown>
                                );
                            })}
                        </div>
                    )
                }

                {!isUser && (!message.images || message.images.length === 0) && (
                    <div className="absolute -bottom-6 right-0 opacity-0 group-hover:opacity-100 transition-opacity duration-200">
                        <CopyButton content={sanitizedContent} className="border-border/50 shadow-sm" />
                    </div>
                )}

                {isUser && !isEditing && isLastUser && (
                    <div className="absolute -bottom-8 right-0 opacity-0 group-hover:opacity-100 transition-opacity duration-200 z-20">
                        <button
                            onClick={() => { setIsEditing(true); setEditContent(message.content); }}
                            className="bg-card border border-border text-muted-foreground hover:text-foreground p-1.5 rounded-full shadow-sm transition-colors hover:bg-accent"
                            title="Edit Message"
                        >
                            <Pencil className="w-3.5 h-3.5" />
                        </button>
                    </div>
                )}
            </div>
        </div>
    );
}
