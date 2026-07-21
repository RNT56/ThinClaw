export const PERFORMANCE_BUDGETS = Object.freeze({
    backendReadyMs: 8_000,
    rendererReadyMs: 2_500,
    streamEventFlushMs: 16,
    frontendChunkBytes: 500 * 1024,
});

export interface RendererPerformanceSnapshot {
    renderer_ready_ms: number;
    renderer_budget_ms: number;
    renderer_budget_exceeded: boolean;
}

declare global {
    interface Window {
        __THINCLAW_PERFORMANCE__?: RendererPerformanceSnapshot;
    }
}

export function rendererPerformanceSnapshot(elapsedMs: number): RendererPerformanceSnapshot {
    const rounded = Math.max(0, Math.round(elapsedMs));
    return {
        renderer_ready_ms: rounded,
        renderer_budget_ms: PERFORMANCE_BUDGETS.rendererReadyMs,
        renderer_budget_exceeded: rounded > PERFORMANCE_BUDGETS.rendererReadyMs,
    };
}

export function recordRendererReady(): RendererPerformanceSnapshot {
    if (window.__THINCLAW_PERFORMANCE__) return window.__THINCLAW_PERFORMANCE__;

    const snapshot = rendererPerformanceSnapshot(performance.now());
    window.__THINCLAW_PERFORMANCE__ = snapshot;
    const log = snapshot.renderer_budget_exceeded ? console.warn : console.info;
    log('[performance] renderer ready', snapshot);
    return snapshot;
}
