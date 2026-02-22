
import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Globe, Search, ExternalLink, Sparkles } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { listen } from '@tauri-apps/api/event';

export type WebSource = {
    title: string;
    link: string;
    snippet: string;
}

export type WebStatusState = 'idle' | 'searching' | 'scraping' | 'analyzing' | 'summarizing' | 'generating' | 'done' | 'error' | 'rag_searching' | 'rag_reading';

type ScrapingProgress = {
    url: string;
    status: string;
    title?: string;
    content_preview?: string;
}

function ScrapingStreamWindow({ progress }: { progress: ScrapingProgress | null }) {
    if (!progress || !progress.content_preview) return null;

    return (
        <motion.div
            initial={{ opacity: 0, height: 0, marginTop: 0 }}
            animate={{ opacity: 1, height: 'auto', marginTop: 8 }}
            exit={{ opacity: 0, height: 0, marginTop: 0 }}
            className="w-full max-w-md overflow-hidden rounded-xl border border-primary/10 bg-black/40 backdrop-blur-sm"
        >
            <div className="flex items-center justify-between px-3 py-1.5 border-b border-white/5 bg-white/5">
                <div className="flex items-center gap-2">
                    <div className="w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse shadow-[0_0_8px_rgba(34,197,94,0.5)]" />
                    <span className="text-[10px] font-medium text-green-400/90 uppercase tracking-wider">
                        Live Read
                    </span>
                </div>
                <span className="text-[9px] text-muted-foreground truncate max-w-[150px]">
                    {progress.title}
                </span>
            </div>

            <div className="relative h-24 overflow-hidden p-3 font-mono text-[10px] leading-relaxed text-muted-foreground/80">
                <div className="absolute inset-0 bg-gradient-to-b from-black/10 via-transparent to-black/80 z-10 pointer-events-none" />

                <motion.div
                    key={progress.url}
                    initial={{ y: 0 }}
                    animate={{ y: "-100%" }}
                    transition={{
                        duration: 15,
                        ease: "linear",
                    }}
                >
                    {progress.content_preview}
                </motion.div>
            </div>
        </motion.div>
    );
}

interface WebSearchBubbleProps {
    status: WebStatusState;
    message: string;
    sources: WebSource[];
}

function SourceCard({ src, index }: { src: WebSource, index: number }) {
    const [isHovered, setIsHovered] = useState(false);

    let domain = "";
    try {
        if (src.link) {
            domain = new URL(src.link).hostname.replace(/^www\./, '');
        }
    } catch (e) {
        domain = "";
    }
    // If no domain, use a generic "Web" label or the title if short, but "Source" is safe
    const displayDomain = domain || "Source";

    // Only try to get favicon if we have a domain
    const favicon = domain
        ? `https://www.google.com/s2/favicons?domain=${domain}&sz=32`
        : ''; // Empty string will trigger onError in img

    const handleOpenLink = async (e: React.MouseEvent) => {
        e.preventDefault();
        try {
            if (src.link) {
                await openUrl(src.link);
            }
        } catch (error) {
            console.error("Failed to open link:", error);
        }
    };

    return (
        <div
            className="relative group print:hidden" // Added print:hidden just in case
            onMouseEnter={() => setIsHovered(true)}
            onMouseLeave={() => setIsHovered(false)}
        >
            <motion.div
                onClick={handleOpenLink}
                initial={{ opacity: 0, scale: 0.9 }}
                animate={{ opacity: 1, scale: 1 }}
                transition={{ delay: index * 0.05 }}
                className="flex items-center gap-3 px-3 py-2.5 rounded-xl bg-card border border-border/60 hover:border-primary/40 hover:bg-accent/10 hover:shadow-sm transition-all w-full sm:w-[260px] cursor-pointer"
            >
                <div className="relative shrink-0 w-8 h-8 rounded-lg bg-background/80 flex items-center justify-center p-1.5 border border-border/40 shadow-sm transition-transform group-hover:scale-105">
                    <img
                        src={favicon}
                        alt=""
                        className="w-full h-full object-contain rounded-sm"
                        onError={(e) => {
                            e.currentTarget.style.display = 'none';
                        }}
                    />
                    {/* Fallback Globe behind image (visible if img fails/loads transparent) */}
                    <div className="absolute inset-0 flex items-center justify-center -z-10">
                        <Globe className="w-4 h-4 text-muted-foreground/30" />
                    </div>
                </div>

                <div className="flex flex-col min-w-0 flex-1">
                    <span className="text-xs font-semibold truncate text-foreground/90 group-hover:text-primary transition-colors">
                        {src.title}
                    </span>
                    <span className="text-[10px] text-muted-foreground/70 truncate leading-none mt-0.5">
                        {displayDomain}
                    </span>
                </div>
            </motion.div >

            {/* Preview Hover Card */}
            <AnimatePresence>
                {
                    isHovered && (
                        <motion.div
                            initial={{ opacity: 0, y: -8, scale: 0.96 }}
                            animate={{ opacity: 1, y: 0, scale: 1 }}
                            exit={{ opacity: 0, y: -4, scale: 0.96 }}
                            transition={{ duration: 0.15, ease: "easeOut" }}
                            className="absolute top-[calc(100%+8px)] left-1/2 -translate-x-1/2 z-50 w-[300px] p-4 rounded-xl bg-popover/95 backdrop-blur-xl border border-border shadow-xl ring-1 ring-black/5"
                        >
                            {/* Header */}
                            <div className="flex items-center gap-2 mb-3 pb-2 border-b border-border/40">
                                <img src={favicon} alt="" className="w-3.5 h-3.5 rounded-sm opacity-80" />
                                <span className="text-[10px] uppercase tracking-wider font-semibold text-muted-foreground truncate">{displayDomain}</span>
                            </div>

                            {/* Content */}
                            <div className="space-y-2">
                                <h4 className="font-semibold text-sm text-foreground leading-snug">
                                    {src.title}
                                </h4>
                                <p className="text-xs text-muted-foreground leading-relaxed line-clamp-4 bg-muted/30 p-2 rounded-md border border-border/20">
                                    {src.snippet}
                                </p>
                            </div>

                            {/* Footer/CTA */}
                            <div
                                className="mt-3 pt-1 flex items-center justify-between text-[10px] text-muted-foreground/50 hover:text-primary cursor-pointer transition-colors"
                                onClick={(e) => {
                                    e.stopPropagation();
                                    handleOpenLink(e);
                                }}
                            >
                                <span>Source Preview</span>
                                <span className="flex items-center gap-1 font-medium">
                                    Visit Website <ExternalLink className="w-3 h-3" />
                                </span>
                            </div>

                            {/* Arrow */}
                            <div className="absolute -top-2 left-1/2 -translate-x-1/2 w-4 h-4 rotate-45 bg-popover border-l border-t border-border z-0"></div>
                        </motion.div>
                    )
                }
            </AnimatePresence >
        </div >
    )
}

interface WebSearchBubbleProps {
    status: WebStatusState;
    message: string;
    sources: WebSource[];
    conversationId: string | null;
}

// ... SourceCard component ...

export function WebSearchBubble({ status, message, sources, conversationId }: WebSearchBubbleProps) {
    const [scrapingProgress, setScrapingProgress] = useState<ScrapingProgress | null>(null);

    useEffect(() => {
        const unlisten = listen<ScrapingProgress & { id?: string }>('scraping_progress', (event) => {
            if (event.payload.id === conversationId) {
                setScrapingProgress(event.payload);
            }
        });
        return () => {
            unlisten.then(f => f());
        };
    }, [conversationId]);

    // Reset progress when status changes away from scraping
    useEffect(() => {
        if (status !== 'scraping') {
            setScrapingProgress(null);
        }
    }, [status]);

    // If idle and no sources, don't render anything
    if (status === 'idle' && (!sources || sources.length === 0)) return null;

    const isSearching = status === 'searching' || status === 'scraping' || status === 'analyzing' || status === 'summarizing' || status === 'generating' || status === 'rag_searching' || status === 'rag_reading';
    const isDone = status === 'done' || (status === 'idle' && sources && sources.length > 0);
    const isError = status === 'error';
    const isRag = status === 'rag_searching' || status === 'rag_reading';

    // Dynamic message
    let displayMessage = message || "Searching...";
    if (status === 'scraping' && scrapingProgress) {
        if (scrapingProgress.status === 'visiting') {
            try {
                const hostname = new URL(scrapingProgress.url).hostname.replace(/^www\./, '');
                displayMessage = `Visiting ${hostname}...`;
            } catch (e) {
                displayMessage = "Visiting link...";
            }
        } else if (scrapingProgress.status === 'scraped') {
            displayMessage = `Reading ${scrapingProgress.title || 'content'}...`;
        }
    } else if (status === 'generating') {
        displayMessage = "Formulating response...";
    }

    return (
        <div className="mb-6">
            <AnimatePresence mode="wait">
                {isSearching && (
                    <motion.div
                        key="searching-container"
                        className="flex flex-col items-center md:items-start"
                    >
                        <motion.div
                            key="searching-pill"
                            initial={{ opacity: 0, scale: 0.9, y: 5 }}
                            animate={{ opacity: 1, scale: 1, y: 0 }}
                            exit={{ opacity: 0, scale: 0.9, height: 0 }}
                            className="inline-flex items-center gap-3 px-4 py-2 bg-secondary/50 backdrop-blur-md border border-primary/20 rounded-full shadow-sm max-w-full md:max-w-md z-10"
                        >
                            <div className="relative flex items-center justify-center w-6 h-6 rounded-full bg-primary/10 text-primary">
                                {status === 'searching' && <Globe className="w-4 h-4 animate-pulse" />}
                                {(status === 'scraping' || status === 'analyzing') && <Search className="w-4 h-4 animate-bounce" />}
                                {status === 'summarizing' && <div className="w-4 h-4 bg-primary/20 rounded animate-pulse" />}
                                {status === 'generating' && <Sparkles className="w-4 h-4 animate-pulse" />}
                                {status === 'rag_searching' && <Search className="w-4 h-4 animate-pulse" />}
                                {status === 'rag_reading' && <div className="w-4 h-4 rounded-sm border-2 border-primary animate-pulse flex items-center justify-center text-[10px] font-bold">F</div>}

                                {/* Spinner Ring */}
                                <div className="absolute inset-0 rounded-full border-2 border-primary border-t-transparent animate-spin" />
                            </div>

                            <div className="flex flex-col min-w-0">
                                <div className="flex items-center gap-2">
                                    <span className="text-xs font-semibold text-primary uppercase tracking-wider">
                                        {status === 'scraping' ? 'Reading Content' :
                                            status === 'analyzing' ? 'Analyzing Data' :
                                                status === 'summarizing' ? 'Summarizing Findings' :
                                                    status === 'generating' ? 'Generating Answer' :
                                                        isRag ? 'Browsing Documents' : 'Browsing the Web'}
                                    </span>
                                    {sources && sources.length > 0 && (
                                        <span className="text-[10px] font-medium bg-primary/10 text-primary px-1.5 py-0.5 rounded-full animate-in fade-in zoom-in duration-300">
                                            {sources.length}
                                        </span>
                                    )}
                                </div>
                                <span className="text-xs text-muted-foreground truncate max-w-[200px] md:max-w-[300px]">
                                    {displayMessage}
                                </span>
                            </div>

                            {/* Mini Wave Visualizer */}
                            <div className="flex gap-0.5 items-end h-3 w-8 opacity-50 ml-2">
                                {[1, 2, 3].map(i => (
                                    <motion.div
                                        key={i}
                                        animate={{ height: [4, 12, 4] }}
                                        transition={{ repeat: Infinity, duration: 1, delay: i * 0.15 }}
                                        className="w-1 bg-primary rounded-full"
                                    />
                                ))}
                            </div>
                        </motion.div>

                        {/* Live Scraping Window */}
                        {status === 'scraping' && (
                            <ScrapingStreamWindow progress={scrapingProgress} />
                        )}
                    </motion.div>
                )}

                {isError && (
                    <motion.div
                        key="error-pill"
                        initial={{ opacity: 0, y: 5 }}
                        animate={{ opacity: 1, y: 0 }}
                        className="inline-flex items-center gap-2 px-3 py-2 bg-destructive/10 text-destructive border border-destructive/20 rounded-lg text-xs font-medium"
                    >
                        <div className="w-4 h-4 rounded-full bg-destructive text-destructive-foreground flex items-center justify-center font-bold">!</div>
                        {message || "Search failed"}
                    </motion.div>
                )}

                {isDone && (
                    <motion.div
                        key="sources-grid"
                        initial={{ opacity: 0, y: 10 }}
                        animate={{ opacity: 1, y: 0 }}
                        transition={{ duration: 0.4, type: "spring" }}
                        className="w-full"
                    >
                        <div className="flex items-center gap-2 mb-3">
                            <div className="p-1.5 bg-primary/10 rounded-md">
                                <Globe className="w-3.5 h-3.5 text-primary" />
                            </div>
                            <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Sources</span>
                            <div className="h-px flex-1 bg-border/50" />
                            <span className="text-[10px] text-muted-foreground bg-secondary/50 px-2 py-0.5 rounded-full">
                                {sources.length} found
                            </span>
                        </div>

                        <div className="flex flex-wrap gap-3">
                            {(sources || []).map((src, i) => (
                                <SourceCard key={i} src={src} index={i} />
                            ))}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
