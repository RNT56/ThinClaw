/**
 * CanvasToolbar — floating badge + popover listing active Canvas panels.
 *
 * Shows a small badge in the bottom-right corner when panels are active.
 * Click to show a popover listing all panels with focus/dismiss controls.
 */

import { useState, useRef, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Layers, X, Eye, Trash2, ChevronUp, AlertTriangle, RefreshCw } from 'lucide-react';
import { cn } from '../../../lib/utils';
import { useCanvas } from './CanvasProvider';

export function CanvasToolbar({ showAvailability = false }: { showAvailability?: boolean }) {
    const { panels, panelCount, focusedPanelId, focusPanel, dismissPanel, dismissAll, legacyVisible, legacyContent, setLegacyVisible, availability, availabilityReason, refreshPanels } = useCanvas();
    const [isOpen, setIsOpen] = useState(false);
    const popoverRef = useRef<HTMLDivElement>(null);

    // Close popover when clicking outside
    useEffect(() => {
        const handleClick = (e: MouseEvent) => {
            if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
                setIsOpen(false);
            }
        };
        if (isOpen) {
            document.addEventListener('mousedown', handleClick);
            return () => document.removeEventListener('mousedown', handleClick);
        }
    }, [isOpen]);

    if (panelCount === 0) {
        if (!showAvailability) return null;
        const unavailable = availability === 'unavailable';
        return (
            <div ref={popoverRef} className="fixed bottom-4 right-4 z-[60]">
                <motion.button
                    initial={{ scale: 0 }}
                    animate={{ scale: 1 }}
                    onClick={() => refreshPanels()}
                    className={cn(
                        'flex items-center gap-2 px-3 py-2 rounded-full shadow-lg border transition-all',
                        unavailable
                            ? 'bg-amber-950/80 border-amber-500/20 text-amber-300'
                            : 'bg-zinc-900/70 backdrop-blur-xl border-white/10 text-zinc-400 hover:border-cyan-500/30 hover:text-cyan-300',
                    )}
                    title={unavailable ? `Canvas/A2UI unavailable: ${availabilityReason || 'unknown'}` : 'Canvas/A2UI ready'}
                >
                    {availability === 'checking' ? (
                        <RefreshCw className="w-4 h-4 animate-spin" />
                    ) : unavailable ? (
                        <AlertTriangle className="w-4 h-4" />
                    ) : (
                        <Layers className="w-4 h-4" />
                    )}
                    <span className="text-xs font-semibold">0</span>
                </motion.button>
            </div>
        );
    }

    return (
        <div ref={popoverRef} className="fixed bottom-4 right-4 z-[60]">
            <AnimatePresence>
                {isOpen && (
                    <motion.div
                        initial={{ opacity: 0, y: 8, scale: 0.95 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 8, scale: 0.95 }}
                        transition={{ type: 'spring', stiffness: 400, damping: 30 }}
                        className="absolute bottom-12 right-0 w-72 bg-zinc-950/95 backdrop-blur-xl border border-white/10 rounded-xl shadow-2xl overflow-hidden"
                    >
                        {/* Header */}
                        <div className="flex items-center justify-between px-3 py-2 border-b border-white/10 bg-white/5">
                            <div className="flex items-center gap-2">
                                <Layers className="w-3.5 h-3.5 text-cyan-400" />
                                <span className="text-xs font-semibold text-zinc-200">Canvas Panels</span>
                                <span className="text-[10px] text-zinc-500 bg-white/5 px-1.5 py-0.5 rounded">
                                    {panelCount}
                                </span>
                            </div>
                            <button
                                onClick={() => { dismissAll(); setIsOpen(false); }}
                                className="text-[10px] text-red-400 hover:text-red-300 transition-colors flex items-center gap-1"
                            >
                                <Trash2 className="w-3 h-3" />
                                Close All
                            </button>
                        </div>

                        {/* Panel list */}
                        <div className="max-h-64 overflow-y-auto py-1">
                            {/* A2UI Panels */}
                            {Array.from(panels.values()).map(panel => (
                                <div
                                    key={panel.id}
                                    className={cn(
                                        'flex items-center gap-2 px-3 py-2 hover:bg-white/5 transition-colors cursor-pointer group',
                                        focusedPanelId === panel.id && 'bg-indigo-500/10 border-l-2 border-indigo-500'
                                    )}
                                    onClick={() => focusPanel(panel.id)}
                                >
                                    {/* Indicator */}
                                    <div className={cn(
                                        'w-1.5 h-1.5 rounded-full',
                                        panel.modal ? 'bg-amber-400' : 'bg-cyan-400'
                                    )} />

                                    {/* Info */}
                                    <div className="flex-1 min-w-0">
                                        <div className="text-xs font-medium text-zinc-200 truncate">{panel.title}</div>
                                        <div className="flex items-center gap-1.5 mt-0.5">
                                            <span className="text-[9px] text-zinc-600 font-mono">{panel.id}</span>
                                            <span className="text-[9px] text-zinc-600">·</span>
                                            <span className="text-[9px] text-zinc-600">{panel.position}</span>
                                            <span className="text-[9px] text-zinc-600">·</span>
                                            <span className="text-[9px] text-zinc-600">{panel.components.length} items</span>
                                        </div>
                                    </div>

                                    {/* Actions */}
                                    <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                                        <button
                                            onClick={(e) => { e.stopPropagation(); focusPanel(panel.id); }}
                                            className="p-1 hover:bg-white/10 rounded transition-colors text-zinc-500 hover:text-cyan-400"
                                            title="Focus"
                                        >
                                            <Eye className="w-3 h-3" />
                                        </button>
                                        <button
                                            onClick={(e) => { e.stopPropagation(); dismissPanel(panel.id); }}
                                            className="p-1 hover:bg-red-500/10 rounded transition-colors text-zinc-500 hover:text-red-400"
                                            title="Dismiss"
                                        >
                                            <X className="w-3 h-3" />
                                        </button>
                                    </div>
                                </div>
                            ))}

                            {/* Legacy content */}
                            {legacyVisible && legacyContent && (
                                <div
                                    className="flex items-center gap-2 px-3 py-2 hover:bg-white/5 transition-colors cursor-pointer group"
                                    onClick={() => setLegacyVisible(true)}
                                >
                                    <div className="w-1.5 h-1.5 rounded-full bg-purple-400" />
                                    <div className="flex-1 min-w-0">
                                        <div className="text-xs font-medium text-zinc-200">Legacy Canvas</div>
                                        <div className="text-[9px] text-zinc-600">
                                            {legacyContent.url ? 'URL view' : legacyContent.type}
                                        </div>
                                    </div>
                                    <button
                                        onClick={(e) => { e.stopPropagation(); setLegacyVisible(false); }}
                                        className="p-1 hover:bg-red-500/10 rounded transition-colors text-zinc-500 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-opacity"
                                    >
                                        <X className="w-3 h-3" />
                                    </button>
                                </div>
                            )}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Badge button */}
            <motion.button
                initial={{ scale: 0 }}
                animate={{ scale: 1 }}
                className={cn(
                    'flex items-center gap-2 px-3 py-2 rounded-full shadow-lg border transition-all',
                    'bg-zinc-900/90 backdrop-blur-xl border-white/10 hover:border-cyan-500/30',
                    isOpen && 'border-cyan-500/40 bg-zinc-900'
                )}
                onClick={() => setIsOpen(!isOpen)}
            >
                <Layers className="w-4 h-4 text-cyan-400" />
                <span className="text-xs font-semibold text-zinc-200">{panelCount}</span>
                <ChevronUp className={cn(
                    'w-3 h-3 text-zinc-500 transition-transform',
                    isOpen && 'rotate-180'
                )} />
            </motion.button>
        </div>
    );
}
