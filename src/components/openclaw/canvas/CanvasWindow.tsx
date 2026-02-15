
import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
// @ts-ignore
import { listen } from '@tauri-apps/api/event';
import { X, Globe } from 'lucide-react';
import { dispatchCanvasEvent } from '../../../lib/openclaw';

interface CanvasContent {
    type: 'html' | 'json';
    data: string;
    url?: string;
    sessionKey?: string;
    runId?: string;
}

export function CanvasWindow() {
    const [isVisible, setIsVisible] = useState(false);
    const [content, setContent] = useState<CanvasContent | null>(null);
    const iframeRef = useRef<HTMLIFrameElement>(null);

    // Listen for events from Backend
    useEffect(() => {
        let unlisteners: Array<() => void> = [];

        const setupListeners = async () => {
            // Legacy push support
            const u1 = await listen('openclaw-canvas-push', (event: any) => {
                setContent({ type: 'html', data: event.payload as string });
                setIsVisible(true);
            });
            unlisteners.push(() => u1());

            const u2 = await listen('openclaw-canvas-navigate', (event: any) => {
                setContent(prev => prev ? { ...prev, url: event.payload as string } : { type: 'html', data: '', url: event.payload as string });
                setIsVisible(true);
            });
            unlisteners.push(() => u2());

            // New CanvasUpdate event via openclaw-event pipeline
            const u3 = await listen('openclaw-event', (event: any) => {
                const payload = event.payload;
                if (payload && payload.kind === 'CanvasUpdate') {
                    setContent({
                        type: (payload.content_type as any) || 'html',
                        data: payload.content || '',
                        url: payload.url,
                        sessionKey: payload.session_key,
                        runId: payload.run_id
                    });
                    setIsVisible(true);
                }
            });
            unlisteners.push(() => u3());
        };

        setupListeners();

        return () => {
            unlisteners.forEach(u => u());
        };
    }, []);

    // Listen for events from Iframe (the "Shim")
    useEffect(() => {
        const handleMessage = (event: MessageEvent) => {
            // Safety check: ensure message is from our iframe?
            // For now, check structure
            if (event.data && event.data.source === 'openclaw-canvas') {
                const { eventType, payload } = event.data;
                if (content?.sessionKey) {
                    dispatchCanvasEvent(content.sessionKey, eventType, payload, content.runId)
                        .catch(err => console.error("Failed to dispatch canvas event:", err));
                } else {
                    console.warn("Canvas event dispatched but no active session key found.");
                }
            }
        };

        window.addEventListener('message', handleMessage);
        return () => window.removeEventListener('message', handleMessage);
    }, [content]);

    if (!isVisible) return null;

    // Helper to inject script
    const getSrcDoc = () => {
        if (content?.url) return undefined;

        const script = `
            window.openclaw = {
                emit: (eventType, payload) => {
                    window.parent.postMessage({ 
                        source: 'openclaw-canvas',
                        eventType, 
                        payload 
                    }, '*');
                }
            };
        `;

        if (content?.type === 'html') {
            return `
                <!DOCTYPE html>
                <html>
                <head>
                    <script>${script}</script>
                    <style>
                        body { margin: 0; padding: 1rem; color: #e4e4e7; font-family: ui-sans-serif, system-ui, sans-serif, "Apple Color Emoji", "Segoe UI Emoji"; }
                        a { color: #818cf8; }
                    </style>
                </head>
                <body>
                    ${content.data}
                </body>
                </html>
            `;
        }

        // Fallback for JSON or other types -> render pre
        return `
            <html><head><script>${script}</script></head>
            <body style="background:#000;color:#0f0;"><pre>${content?.data}</pre></body></html>
        `;
    };

    return (
        <AnimatePresence>
            {isVisible && (
                <motion.div
                    initial={{ x: 400, opacity: 0 }}
                    animate={{ x: 0, opacity: 1 }}
                    exit={{ x: 400, opacity: 0 }}
                    transition={{ type: "spring", stiffness: 300, damping: 30 }}
                    className="fixed top-20 right-4 w-[400px] h-[600px] bg-zinc-950/90 backdrop-blur-md border border-white/10 rounded-lg shadow-2xl z-50 flex flex-col overflow-hidden"
                >
                    {/* Header */}
                    <div className="flex items-center justify-between p-3 border-b border-white/10 bg-white/5">
                        <div className="flex items-center gap-2 text-sm font-medium text-zinc-200">
                            <Globe className="w-4 h-4 text-cyan-400" />
                            <span>Canvas</span>
                            {content?.sessionKey && <span className="text-xs text-zinc-500 ml-2">Session: {content.sessionKey.substring(0, 8)}...</span>}
                        </div>
                        <button
                            onClick={() => setIsVisible(false)}
                            className="p-1 hover:bg-white/10 rounded-md transition-colors text-zinc-400 hover:text-white"
                        >
                            <X className="w-4 h-4" />
                        </button>
                    </div>

                    {/* Content */}
                    <div className="flex-1 bg-white/5 relative">
                        {content?.url ? (
                            <iframe
                                ref={iframeRef}
                                src={content.url}
                                className="w-full h-full border-none"
                                title="Canvas URL"
                                sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
                            />
                        ) : (
                            <iframe
                                ref={iframeRef}
                                srcDoc={getSrcDoc()}
                                className="w-full h-full border-none"
                                title="Canvas Content"
                                sandbox="allow-scripts allow-forms allow-popups"
                            />
                        )}
                    </div>

                    {/* Footer / Status */}
                    <div className="p-2 border-t border-white/10 bg-black/40 text-[10px] text-zinc-600 flex justify-between uppercase tracking-wider font-mono">
                        <span>A2UI Protocol v2</span>
                        <span>{content?.type || 'IDLE'}</span>
                    </div>
                </motion.div>
            )}
        </AnimatePresence>
    );
}
