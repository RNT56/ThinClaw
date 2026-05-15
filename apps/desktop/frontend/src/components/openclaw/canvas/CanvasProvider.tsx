/**
 * CanvasProvider — manages all active Canvas/A2UI panels.
 *
 * Listens for CanvasAction events (via `openclaw-event` with `content_type: "canvas_action"`)
 * and legacy canvas push/navigate events. Maintains a map of active panels by ID.
 *
 * Used by CanvasWindow (multi-panel renderer) and CanvasToolbar (panel badge/popover).
 */

import { createContext, useContext, useState, useEffect, useCallback, useRef, type ReactNode } from 'react';
// @ts-ignore
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import type {
    CanvasAction, CanvasActionShow, UiComponent,
    PanelPosition, NotifyLevel
} from '../../../lib/openclaw';
import { dismissCanvasPanel, getCanvasPanel, listCanvasPanels } from '../../../lib/openclaw';

// ── Panel State ─────────────────────────────────────────────────────

export interface CanvasPanel {
    id: string;
    title: string;
    components: UiComponent[];
    position: PanelPosition;
    modal: boolean;
    sessionKey?: string;
    runId?: string;
    createdAt: number;
    updatedAt: number;
}

interface CanvasContextType {
    /** All active panels keyed by panel_id */
    panels: Map<string, CanvasPanel>;
    /** Currently focused panel ID */
    focusedPanelId: string | null;
    /** Legacy HTML content (backward compat) */
    legacyContent: { type: 'html' | 'json'; data: string; url?: string; sessionKey?: string; runId?: string } | null;
    /** Whether any panels or legacy content is visible */
    hasContent: boolean;
    /** Total panel count */
    panelCount: number;
    /** Backend canvas command availability */
    availability: 'checking' | 'ready' | 'unavailable';
    availabilityReason: string | null;
    refreshPanels: () => Promise<void>;
    /** Focus a specific panel */
    focusPanel: (id: string) => void;
    /** Dismiss a panel */
    dismissPanel: (id: string) => void;
    /** Dismiss all panels */
    dismissAll: () => void;
    /** Set legacy content visibility */
    setLegacyVisible: (v: boolean) => void;
    legacyVisible: boolean;
}

const CanvasContext = createContext<CanvasContextType | null>(null);

export function useCanvas() {
    const ctx = useContext(CanvasContext);
    if (!ctx) throw new Error('useCanvas must be used within CanvasProvider');
    return ctx;
}

export function CanvasProviderWrapper({ children }: { children: ReactNode }) {
    const [panels, setPanels] = useState<Map<string, CanvasPanel>>(new Map());
    const [focusedPanelId, setFocusedPanelId] = useState<string | null>(null);
    const [legacyContent, setLegacyContent] = useState<CanvasContextType['legacyContent']>(null);
    const [legacyVisible, setLegacyVisible] = useState(false);
    const [availability, setAvailability] = useState<CanvasContextType['availability']>('checking');
    const [availabilityReason, setAvailabilityReason] = useState<string | null>(null);
    const toastTimers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

    // ── Handle CanvasAction payloads ──────────────────────────────────
    const handleCanvasAction = useCallback((action: CanvasAction, sessionKey?: string, runId?: string) => {
        const now = Date.now();

        switch (action.action) {
            case 'show': {
                const show = action as CanvasActionShow;
                setPanels(prev => {
                    const next = new Map(prev);
                    next.set(show.panel_id, {
                        id: show.panel_id,
                        title: show.title,
                        components: show.components,
                        position: show.position ?? 'right',
                        modal: show.modal ?? false,
                        sessionKey,
                        runId,
                        createdAt: now,
                        updatedAt: now,
                    });
                    return next;
                });
                setFocusedPanelId(show.panel_id);
                break;
            }
            case 'update': {
                setPanels(prev => {
                    const next = new Map(prev);
                    const existing = next.get(action.panel_id);
                    if (existing) {
                        next.set(action.panel_id, {
                            ...existing,
                            components: action.components,
                            updatedAt: now,
                        });
                    }
                    return next;
                });
                break;
            }
            case 'dismiss': {
                setPanels(prev => {
                    const next = new Map(prev);
                    next.delete(action.panel_id);
                    return next;
                });
                if (focusedPanelId === action.panel_id) {
                    setFocusedPanelId(null);
                }
                break;
            }
            case 'notify': {
                const level = (action.level ?? 'info') as NotifyLevel;
                const msg = action.message;
                const duration = (action.duration_secs ?? 5) * 1000;
                switch (level) {
                    case 'success':
                        toast.success(msg, { duration });
                        break;
                    case 'warning':
                        toast.warning(msg, { duration });
                        break;
                    case 'error':
                        toast.error(msg, { duration });
                        break;
                    default:
                        toast.info(msg, { duration });
                }
                break;
            }
        }
    }, [focusedPanelId]);

    const refreshPanels = useCallback(async () => {
        try {
            const listing = await listCanvasPanels();
            const loaded = await Promise.all(
                (listing.panels || []).map(summary => getCanvasPanel(summary.panel_id))
            );
            const now = Date.now();
            const next = new Map<string, CanvasPanel>();
            for (const panel of loaded) {
                if (!panel) continue;
                const metadata = (panel.metadata && typeof panel.metadata === 'object' ? panel.metadata : {}) as Record<string, any>;
                next.set(panel.panel_id, {
                    id: panel.panel_id,
                    title: panel.title,
                    components: Array.isArray(panel.components) ? panel.components as UiComponent[] : [],
                    position: (metadata.position || 'right') as PanelPosition,
                    modal: Boolean(metadata.modal),
                    sessionKey: metadata.session_key,
                    runId: metadata.run_id,
                    createdAt: now,
                    updatedAt: now,
                });
            }
            setPanels(next);
            setFocusedPanelId(prev => prev && next.has(prev) ? prev : next.keys().next().value || null);
            setAvailability('ready');
            setAvailabilityReason(null);
        } catch (e) {
            setAvailability('unavailable');
            setAvailabilityReason(String(e));
        }
    }, []);

    // ── Event listeners ──────────────────────────────────────────────
    useEffect(() => {
        const unlisteners: Array<() => void> = [];

        const setup = async () => {
            // Listen for legacy canvas push
            const u1 = await listen('openclaw-canvas-push', (event: any) => {
                setLegacyContent({ type: 'html', data: event.payload as string });
                setLegacyVisible(true);
            });
            unlisteners.push(() => u1());

            // Listen for legacy canvas navigate
            const u2 = await listen('openclaw-canvas-navigate', (event: any) => {
                setLegacyContent(prev => prev
                    ? { ...prev, url: event.payload as string }
                    : { type: 'html', data: '', url: event.payload as string }
                );
                setLegacyVisible(true);
            });
            unlisteners.push(() => u2());

            // Listen for openclaw-event — handle both CanvasUpdate (legacy) and canvas_action (A2UI)
            const u3 = await listen('openclaw-event', (event: any) => {
                const payload = event.payload;
                if (!payload) return;

                if (payload.kind === 'CanvasUpdate') {
                    if (payload.content_type === 'canvas_action') {
                        // New A2UI CanvasAction — parse and dispatch
                        try {
                            const action: CanvasAction = JSON.parse(payload.content);
                            handleCanvasAction(action, payload.session_key, payload.run_id);
                        } catch (e) {
                            console.error('[Canvas] Failed to parse CanvasAction:', e);
                        }
                    } else {
                        // Legacy HTML/JSON content
                        setLegacyContent({
                            type: (payload.content_type as any) || 'html',
                            data: payload.content || '',
                            url: payload.url,
                            sessionKey: payload.session_key,
                            runId: payload.run_id,
                        });
                        setLegacyVisible(true);
                    }
                }
            });
            unlisteners.push(() => u3());
        };

        setup();
        refreshPanels();
        return () => {
            unlisteners.forEach(u => u());
            toastTimers.current.forEach(t => clearTimeout(t));
        };
    }, [handleCanvasAction, refreshPanels]);

    const focusPanel = useCallback((id: string) => setFocusedPanelId(id), []);
    const dismissPanel = useCallback((id: string) => {
        dismissCanvasPanel(id).catch(() => { });
        setPanels(prev => {
            const next = new Map(prev);
            next.delete(id);
            return next;
        });
        if (focusedPanelId === id) setFocusedPanelId(null);
    }, [focusedPanelId]);
    const dismissAll = useCallback(() => {
        panels.forEach((_panel, id) => dismissCanvasPanel(id).catch(() => { }));
        setPanels(new Map());
        setFocusedPanelId(null);
        setLegacyContent(null);
        setLegacyVisible(false);
    }, [panels]);

    const panelCount = panels.size + (legacyVisible && legacyContent ? 1 : 0);
    const hasContent = panelCount > 0;

    return (
        <CanvasContext.Provider value={{
            panels, focusedPanelId, legacyContent, hasContent, panelCount,
            availability, availabilityReason, refreshPanels,
            focusPanel, dismissPanel, dismissAll, setLegacyVisible, legacyVisible,
        }}>
            {children}
        </CanvasContext.Provider>
    );
}
