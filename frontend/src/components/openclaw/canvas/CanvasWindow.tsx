
import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
// @ts-ignore
import { listen } from '@tauri-apps/api/event';
import { X, Globe, Maximize2, Minimize2, GripVertical } from 'lucide-react';
import { dispatchCanvasEvent } from '../../../lib/openclaw';

interface CanvasContent {
    type: 'html' | 'json';
    data: string;
    url?: string;
    sessionKey?: string;
    runId?: string;
}

interface Position { x: number; y: number; }
interface Size { w: number; h: number; }

export function CanvasWindow() {
    const [isVisible, setIsVisible] = useState(false);
    const [content, setContent] = useState<CanvasContent | null>(null);
    const [isMaximized, setIsMaximized] = useState(false);
    const iframeRef = useRef<HTMLIFrameElement>(null);

    // Position & Size state
    const [pos, setPos] = useState<Position>({ x: -1, y: -1 }); // -1 means "use default"
    const [size, setSize] = useState<Size>({ w: 420, h: 600 });

    // Drag state
    const dragging = useRef(false);
    const resizing = useRef<'br' | 'bl' | 'tr' | 'tl' | null>(null);
    const dragOffset = useRef({ x: 0, y: 0 });
    const startPos = useRef({ x: 0, y: 0 });
    const startSize = useRef({ w: 0, h: 0 });

    // Listen for events from Backend
    useEffect(() => {
        let unlisteners: Array<() => void> = [];

        const setupListeners = async () => {
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
        return () => { unlisteners.forEach(u => u()); };
    }, []);

    // Listen for events from Iframe
    useEffect(() => {
        const handleMessage = (event: MessageEvent) => {
            if (event.data && event.data.source === 'openclaw-canvas') {
                const { eventType, payload } = event.data;
                if (content?.sessionKey) {
                    dispatchCanvasEvent(content.sessionKey, eventType, payload, content.runId)
                        .catch(err => console.error("Failed to dispatch canvas event:", err));
                }
            }
        };
        window.addEventListener('message', handleMessage);
        return () => window.removeEventListener('message', handleMessage);
    }, [content]);

    // Drag/resize mouse handlers
    useEffect(() => {
        const handleMouseMove = (e: MouseEvent) => {
            if (dragging.current) {
                setPos({
                    x: e.clientX - dragOffset.current.x,
                    y: e.clientY - dragOffset.current.y,
                });
            } else if (resizing.current) {
                const dx = e.clientX - startPos.current.x;
                const dy = e.clientY - startPos.current.y;
                const corner = resizing.current;

                let newW = startSize.current.w;
                let newH = startSize.current.h;

                if (corner === 'br') {
                    newW = Math.max(300, startSize.current.w + dx);
                    newH = Math.max(200, startSize.current.h + dy);
                } else if (corner === 'bl') {
                    newW = Math.max(300, startSize.current.w - dx);
                    newH = Math.max(200, startSize.current.h + dy);
                    setPos(prev => ({ x: startPos.current.x + dx, y: prev.y }));
                } else if (corner === 'tr') {
                    newW = Math.max(300, startSize.current.w + dx);
                    newH = Math.max(200, startSize.current.h - dy);
                    setPos(prev => ({ x: prev.x, y: startPos.current.y + dy }));
                } else if (corner === 'tl') {
                    newW = Math.max(300, startSize.current.w - dx);
                    newH = Math.max(200, startSize.current.h - dy);
                    setPos({ x: startPos.current.x + dx, y: startPos.current.y + dy });
                }

                setSize({ w: newW, h: newH });
            }
        };

        const handleMouseUp = () => {
            dragging.current = false;
            resizing.current = null;
            document.body.style.cursor = '';
            document.body.style.userSelect = '';
        };

        window.addEventListener('mousemove', handleMouseMove);
        window.addEventListener('mouseup', handleMouseUp);
        return () => {
            window.removeEventListener('mousemove', handleMouseMove);
            window.removeEventListener('mouseup', handleMouseUp);
        };
    }, []);

    const onDragStart = (e: React.MouseEvent) => {
        if (isMaximized) return;
        dragging.current = true;
        const el = (e.target as HTMLElement).closest('[data-canvas-window]') as HTMLElement;
        if (!el) return;
        const rect = el.getBoundingClientRect();
        dragOffset.current = { x: e.clientX - rect.left, y: e.clientY - rect.top };
        if (pos.x === -1) {
            setPos({ x: rect.left, y: rect.top });
        }
        document.body.style.cursor = 'grabbing';
        document.body.style.userSelect = 'none';
        e.preventDefault();
    };

    const onResizeStart = (e: React.MouseEvent, corner: 'br' | 'bl' | 'tr' | 'tl') => {
        if (isMaximized) return;
        resizing.current = corner;
        startPos.current = { x: e.clientX, y: e.clientY };
        startSize.current = { ...size };
        document.body.style.cursor = corner === 'br' || corner === 'tl' ? 'nwse-resize' : 'nesw-resize';
        document.body.style.userSelect = 'none';
        e.preventDefault();
        e.stopPropagation();
    };

    if (!isVisible) return null;

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

        return `
            <html><head><script>${script}</script></head>
            <body style="background:#000;color:#0f0;"><pre>${content?.data}</pre></body></html>
        `;
    };

    const windowStyle: React.CSSProperties = isMaximized
        ? { position: 'fixed', top: 0, left: 0, right: 0, bottom: 0, width: '100vw', height: '100vh', borderRadius: 0 }
        : pos.x === -1
            ? { position: 'fixed', top: 80, right: 16, width: size.w, height: size.h }
            : { position: 'fixed', top: pos.y, left: pos.x, width: size.w, height: size.h };

    return (
        <AnimatePresence>
            {isVisible && (
                <motion.div
                    data-canvas-window
                    initial={{ scale: 0.9, opacity: 0 }}
                    animate={{ scale: 1, opacity: 1 }}
                    exit={{ scale: 0.9, opacity: 0 }}
                    transition={{ type: "spring", stiffness: 400, damping: 30 }}
                    style={windowStyle}
                    className="bg-zinc-950/95 backdrop-blur-xl border border-white/10 rounded-lg shadow-2xl z-50 flex flex-col overflow-hidden"
                >
                    {/* Header (draggable) */}
                    <div
                        className="flex items-center justify-between p-2.5 border-b border-white/10 bg-white/5 cursor-grab active:cursor-grabbing select-none"
                        onMouseDown={onDragStart}
                    >
                        <div className="flex items-center gap-2 text-sm font-medium text-zinc-200">
                            <GripVertical className="w-3.5 h-3.5 text-zinc-600" />
                            <Globe className="w-3.5 h-3.5 text-cyan-400" />
                            <span className="text-xs">Canvas</span>
                            {content?.sessionKey && (
                                <span className="text-[10px] text-zinc-600 ml-1">
                                    {content.sessionKey.substring(0, 8)}…
                                </span>
                            )}
                        </div>
                        <div className="flex items-center gap-1">
                            <button
                                onClick={(e) => { e.stopPropagation(); setIsMaximized(!isMaximized); }}
                                className="p-1 hover:bg-white/10 rounded transition-colors text-zinc-500 hover:text-white"
                            >
                                {isMaximized ? <Minimize2 className="w-3 h-3" /> : <Maximize2 className="w-3 h-3" />}
                            </button>
                            <button
                                onClick={(e) => { e.stopPropagation(); setIsVisible(false); }}
                                className="p-1 hover:bg-red-500/20 rounded transition-colors text-zinc-500 hover:text-red-400"
                            >
                                <X className="w-3 h-3" />
                            </button>
                        </div>
                    </div>

                    {/* Content */}
                    <div className="flex-1 bg-white/[0.02] relative">
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

                    {/* Footer */}
                    <div className="p-1.5 border-t border-white/10 bg-black/40 text-[9px] text-zinc-600 flex justify-between uppercase tracking-wider font-mono px-3">
                        <span>A2UI Protocol v2</span>
                        <span>{isMaximized ? 'MAXIMIZED' : `${Math.round(size.w)}×${Math.round(size.h)}`}</span>
                        <span>{content?.type || 'IDLE'}</span>
                    </div>

                    {/* Resize handles (only when not maximized) */}
                    {!isMaximized && (
                        <>
                            <div className="absolute bottom-0 right-0 w-4 h-4 cursor-nwse-resize" onMouseDown={e => onResizeStart(e, 'br')} />
                            <div className="absolute bottom-0 left-0 w-4 h-4 cursor-nesw-resize" onMouseDown={e => onResizeStart(e, 'bl')} />
                            <div className="absolute top-0 right-0 w-4 h-4 cursor-nesw-resize" onMouseDown={e => onResizeStart(e, 'tr')} />
                            <div className="absolute top-0 left-0 w-4 h-4 cursor-nwse-resize" onMouseDown={e => onResizeStart(e, 'tl')} />
                        </>
                    )}
                </motion.div>
            )}
        </AnimatePresence>
    );
}
