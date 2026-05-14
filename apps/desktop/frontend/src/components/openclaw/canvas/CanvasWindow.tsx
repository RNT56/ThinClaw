/**
 * CanvasWindow — renders Canvas/A2UI panels.
 *
 * Supports two modes:
 * 1. **A2UI panels** — native rendering of IronClaw CanvasAction payloads via CanvasPanelRenderer
 * 2. **Legacy content** — iframe-based HTML/URL rendering (backward compat)
 *
 * Panels are draggable, resizable, and support floating/docked/center/modal positions.
 * Multiple panels can be open simultaneously; the focused panel is shown on top.
 */

import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { X, Globe, Maximize2, Minimize2, GripVertical, Layers, PanelRight, PanelBottom, Maximize, Pin } from 'lucide-react';
import { cn } from '../../../lib/utils';
import { dispatchCanvasEvent } from '../../../lib/openclaw';
import { useCanvas, type CanvasPanel } from './CanvasProvider';
import { CanvasPanelRenderer } from './CanvasPanelRenderer';

// ── Position / Size types ───────────────────────────────────────────

interface Position { x: number; y: number; }
interface Size { w: number; h: number; }

const POSITION_DEFAULTS: Record<string, { pos: (ww: number, wh: number, sw: number, sh: number) => Position; size: Size }> = {
    right: {
        pos: (_ww, _wh, sw, _sh) => ({ x: sw - 436, y: 80 }),
        size: { w: 420, h: 600 },
    },
    bottom: {
        pos: (_ww, _wh, _sw, sh) => ({ x: 80, y: sh - 316 }),
        size: { w: 800, h: 300 },
    },
    center: {
        pos: (_ww, _wh, sw, sh) => ({ x: (sw - 560) / 2, y: (sh - 480) / 2 }),
        size: { w: 560, h: 480 },
    },
    floating: {
        pos: () => ({ x: 120, y: 120 }),
        size: { w: 420, h: 500 },
    },
};

// ── Main Component ──────────────────────────────────────────────────

export function CanvasWindow() {
    const {
        panels, focusedPanelId, focusPanel, dismissPanel,
        legacyContent, legacyVisible, setLegacyVisible,
    } = useCanvas();

    const panelArray = useMemo(() => Array.from(panels.values()), [panels]);

    return (
        <>
            {/* A2UI Panels */}
            <AnimatePresence>
                {panelArray.map(panel => (
                    <A2UIPanel
                        key={panel.id}
                        panel={panel}
                        isFocused={focusedPanelId === panel.id}
                        onFocus={() => focusPanel(panel.id)}
                        onDismiss={() => dismissPanel(panel.id)}
                    />
                ))}
            </AnimatePresence>

            {/* Legacy iframe panel */}
            {legacyVisible && legacyContent && (
                <LegacyPanel
                    content={legacyContent}
                    onClose={() => setLegacyVisible(false)}
                />
            )}

            {/* Modal overlay */}
            <AnimatePresence>
                {panelArray.some(p => p.modal) && (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="fixed inset-0 bg-black/60 backdrop-blur-sm z-40"
                        onClick={() => {
                            // Dismiss modal panels on overlay click
                            panelArray.filter(p => p.modal).forEach(p => dismissPanel(p.id));
                        }}
                    />
                )}
            </AnimatePresence>
        </>
    );
}

// ── A2UI Panel (native rendering) ───────────────────────────────────

function A2UIPanel({ panel, isFocused, onFocus, onDismiss }: {
    panel: CanvasPanel;
    isFocused: boolean;
    onFocus: () => void;
    onDismiss: () => void;
}) {
    const defaults = POSITION_DEFAULTS[panel.position] ?? POSITION_DEFAULTS.floating;
    const [pos, setPos] = useState<Position>(() =>
        defaults.pos(0, 0, window.innerWidth, window.innerHeight)
    );
    const [size, setSize] = useState<Size>({ ...defaults.size });
    const [isMaximized, setIsMaximized] = useState(false);

    const dragging = useRef(false);
    const resizing = useRef<string | null>(null);
    const dragOffset = useRef({ x: 0, y: 0 });
    const startPos = useRef({ x: 0, y: 0 });
    const startSize = useRef({ w: 0, h: 0 });

    // Drag/resize handlers
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
                setSize({
                    w: Math.max(280, startSize.current.w + dx),
                    h: Math.max(200, startSize.current.h + dy),
                });
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

    const onDragStart = useCallback((e: React.MouseEvent) => {
        if (isMaximized) return;
        dragging.current = true;
        const el = (e.target as HTMLElement).closest('[data-canvas-panel]') as HTMLElement;
        if (!el) return;
        const rect = el.getBoundingClientRect();
        dragOffset.current = { x: e.clientX - rect.left, y: e.clientY - rect.top };
        document.body.style.cursor = 'grabbing';
        document.body.style.userSelect = 'none';
        e.preventDefault();
        onFocus();
    }, [isMaximized, onFocus]);

    const onResizeStart = useCallback((e: React.MouseEvent) => {
        if (isMaximized) return;
        resizing.current = 'br';
        startPos.current = { x: e.clientX, y: e.clientY };
        startSize.current = { ...size };
        document.body.style.cursor = 'nwse-resize';
        document.body.style.userSelect = 'none';
        e.preventDefault();
        e.stopPropagation();
    }, [isMaximized, size]);

    const positionIcon = panel.position === 'right' ? PanelRight
        : panel.position === 'bottom' ? PanelBottom
            : panel.position === 'center' ? Maximize
                : Pin;

    const windowStyle: React.CSSProperties = isMaximized
        ? { position: 'fixed', top: 0, left: 0, right: 0, bottom: 0, width: '100vw', height: '100vh', borderRadius: 0 }
        : { position: 'fixed', top: pos.y, left: pos.x, width: size.w, height: size.h };

    return (
        <motion.div
            data-canvas-panel
            initial={{ scale: 0.9, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            exit={{ scale: 0.9, opacity: 0 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            style={{
                ...windowStyle,
                zIndex: panel.modal ? 50 : isFocused ? 48 : 45,
            }}
            className={cn(
                'bg-zinc-950/95 backdrop-blur-xl border rounded-lg shadow-2xl flex flex-col overflow-hidden',
                isFocused ? 'border-cyan-500/30' : 'border-white/10',
                panel.modal && 'ring-2 ring-amber-500/30',
            )}
            onClick={onFocus}
        >
            {/* Header */}
            <div
                className={cn(
                    'flex items-center justify-between p-2.5 border-b bg-white/5 cursor-grab active:cursor-grabbing select-none',
                    isFocused ? 'border-cyan-500/20' : 'border-white/10',
                )}
                onMouseDown={onDragStart}
            >
                <div className="flex items-center gap-2 text-sm font-medium text-zinc-200">
                    <GripVertical className="w-3.5 h-3.5 text-zinc-600" />
                    {React.createElement(positionIcon, { className: 'w-3.5 h-3.5 text-cyan-400' })}
                    <span className="text-xs truncate max-w-[180px]">{panel.title}</span>
                    {panel.modal && (
                        <span className="text-[9px] text-amber-400 bg-amber-500/10 px-1.5 py-0.5 rounded font-bold">MODAL</span>
                    )}
                    <span className="text-[9px] text-zinc-600 font-mono">{panel.id}</span>
                </div>
                <div className="flex items-center gap-1">
                    <button
                        onClick={(e) => { e.stopPropagation(); setIsMaximized(!isMaximized); }}
                        className="p-1 hover:bg-white/10 rounded transition-colors text-zinc-500 hover:text-white"
                    >
                        {isMaximized ? <Minimize2 className="w-3 h-3" /> : <Maximize2 className="w-3 h-3" />}
                    </button>
                    <button
                        onClick={(e) => { e.stopPropagation(); onDismiss(); }}
                        className="p-1 hover:bg-red-500/20 rounded transition-colors text-zinc-500 hover:text-red-400"
                    >
                        <X className="w-3 h-3" />
                    </button>
                </div>
            </div>

            {/* Content — native component rendering */}
            <div className="flex-1 overflow-y-auto bg-white/[0.02] scrollbar-thin scrollbar-thumb-white/10">
                <CanvasPanelRenderer
                    components={panel.components}
                    sessionKey={panel.sessionKey}
                    runId={panel.runId}
                />
            </div>

            {/* Footer */}
            <div className="px-3 py-1 border-t border-white/10 bg-black/40 text-[9px] text-zinc-600 flex justify-between uppercase tracking-wider font-mono">
                <span className="flex items-center gap-1.5">
                    <Layers className="w-2.5 h-2.5" />
                    A2UI Panel
                </span>
                <span>{isMaximized ? 'MAXIMIZED' : `${Math.round(size.w)}×${Math.round(size.h)}`}</span>
                <span>{panel.components.length} components</span>
            </div>

            {/* Resize handle */}
            {!isMaximized && (
                <div
                    className="absolute bottom-0 right-0 w-4 h-4 cursor-nwse-resize"
                    onMouseDown={onResizeStart}
                />
            )}
        </motion.div>
    );
}

// ── Legacy Panel (iframe, backward compat) ──────────────────────────

function LegacyPanel({ content, onClose }: {
    content: { type: string; data: string; url?: string; sessionKey?: string; runId?: string };
    onClose: () => void;
}) {
    const iframeRef = useRef<HTMLIFrameElement>(null);
    const [pos, setPos] = useState<Position>({ x: -1, y: -1 });
    const [size] = useState<Size>({ w: 420, h: 600 });
    const [isMaximized, setIsMaximized] = useState(false);
    const dragging = useRef(false);
    const dragOffset = useRef({ x: 0, y: 0 });

    // Iframe message handler
    useEffect(() => {
        const handleMessage = (event: MessageEvent) => {
            if (event.data?.source === 'openclaw-canvas') {
                const { eventType, payload } = event.data;
                if (content.sessionKey) {
                    dispatchCanvasEvent(content.sessionKey, eventType, payload, content.runId)
                        .catch(err => console.error('Failed to dispatch canvas event:', err));
                }
            }
        };
        window.addEventListener('message', handleMessage);
        return () => window.removeEventListener('message', handleMessage);
    }, [content]);

    // Drag handler
    useEffect(() => {
        const handleMouseMove = (e: MouseEvent) => {
            if (dragging.current) {
                setPos({ x: e.clientX - dragOffset.current.x, y: e.clientY - dragOffset.current.y });
            }
        };
        const handleMouseUp = () => {
            dragging.current = false;
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
        const el = (e.target as HTMLElement).closest('[data-legacy-canvas]') as HTMLElement;
        if (!el) return;
        const rect = el.getBoundingClientRect();
        dragOffset.current = { x: e.clientX - rect.left, y: e.clientY - rect.top };
        if (pos.x === -1) setPos({ x: rect.left, y: rect.top });
        document.body.style.cursor = 'grabbing';
        document.body.style.userSelect = 'none';
        e.preventDefault();
    };

    const getSrcDoc = () => {
        if (content.url) return undefined;
        const script = `window.openclaw={emit:(t,p)=>window.parent.postMessage({source:'openclaw-canvas',eventType:t,payload:p},'*')};`;
        if (content.type === 'html') {
            return `<!DOCTYPE html><html><head><script>${script}</script><style>body{margin:0;padding:1rem;color:#e4e4e7;font-family:ui-sans-serif,system-ui,sans-serif;}a{color:#818cf8;}</style></head><body>${content.data}</body></html>`;
        }
        return `<html><head><script>${script}</script></head><body style="background:#000;color:#0f0;"><pre>${content.data}</pre></body></html>`;
    };

    const windowStyle: React.CSSProperties = isMaximized
        ? { position: 'fixed', top: 0, left: 0, right: 0, bottom: 0, width: '100vw', height: '100vh', borderRadius: 0 }
        : pos.x === -1
            ? { position: 'fixed', top: 80, right: 16, width: size.w, height: size.h }
            : { position: 'fixed', top: pos.y, left: pos.x, width: size.w, height: size.h };

    return (
        <motion.div
            data-legacy-canvas
            initial={{ scale: 0.9, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            exit={{ scale: 0.9, opacity: 0 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            style={{ ...windowStyle, zIndex: 44 }}
            className="bg-zinc-950/95 backdrop-blur-xl border border-white/10 rounded-lg shadow-2xl flex flex-col overflow-hidden"
        >
            {/* Header */}
            <div
                className="flex items-center justify-between p-2.5 border-b border-white/10 bg-white/5 cursor-grab active:cursor-grabbing select-none"
                onMouseDown={onDragStart}
            >
                <div className="flex items-center gap-2 text-sm font-medium text-zinc-200">
                    <GripVertical className="w-3.5 h-3.5 text-zinc-600" />
                    <Globe className="w-3.5 h-3.5 text-purple-400" />
                    <span className="text-xs">Legacy Canvas</span>
                </div>
                <div className="flex items-center gap-1">
                    <button
                        onClick={(e) => { e.stopPropagation(); setIsMaximized(!isMaximized); }}
                        className="p-1 hover:bg-white/10 rounded transition-colors text-zinc-500 hover:text-white"
                    >
                        {isMaximized ? <Minimize2 className="w-3 h-3" /> : <Maximize2 className="w-3 h-3" />}
                    </button>
                    <button
                        onClick={(e) => { e.stopPropagation(); onClose(); }}
                        className="p-1 hover:bg-red-500/20 rounded transition-colors text-zinc-500 hover:text-red-400"
                    >
                        <X className="w-3 h-3" />
                    </button>
                </div>
            </div>

            {/* Content */}
            <div className="flex-1 bg-white/[0.02] relative">
                {content.url ? (
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
            <div className="px-3 py-1 border-t border-white/10 bg-black/40 text-[9px] text-zinc-600 flex justify-between uppercase tracking-wider font-mono">
                <span>Legacy v1</span>
                <span>{isMaximized ? 'MAX' : `${size.w}×${size.h}`}</span>
                <span>{content.type}</span>
            </div>
        </motion.div>
    );
}

// Needed for createElement in A2UIPanel
import React from 'react';
