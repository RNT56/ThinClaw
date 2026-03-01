/**
 * useCloudModels — React hook for cloud model discovery.
 *
 * Calls `discover_cloud_models` on mount to fetch models from all providers
 * that have API keys configured. Results are cached on the backend (30 min TTL)
 * and in React state.
 *
 * Usage:
 *   const { models, providers, loading, error, refreshProvider, refreshAll } = useCloudModels();
 */

import { useState, useEffect, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";

// ─── Types matching the Rust Specta-derived types ──────────────────────────

export type ModelCategory = "chat" | "embedding" | "tts" | "stt" | "diffusion" | "other";

export interface ModelPricing {
    inputPerMillion?: number | null;
    outputPerMillion?: number | null;
    perImage?: number | null;
    perMinute?: number | null;
    per1kChars?: number | null;
}

export interface CloudModelEntry {
    id: string;
    displayName: string;
    provider: string;
    providerName: string;
    category: ModelCategory;
    contextWindow: number | null;
    maxOutputTokens: number | null;
    supportsVision: boolean;
    supportsTools: boolean;
    supportsStreaming: boolean;
    deprecated: boolean;
    pricing: ModelPricing | null;
    embeddingDimensions: number | null;
    metadata: Record<string, string>;
}

export interface ProviderDiscoveryResult {
    provider: string;
    models: CloudModelEntry[];
    fromCache: boolean;
    error: string | null;
}

export interface DiscoveryResult {
    providers: ProviderDiscoveryResult[];
    totalModels: number;
    errors: string[];
}

// ─── Hook ──────────────────────────────────────────────────────────────────

export function useCloudModels() {
    const [result, setResult] = useState<DiscoveryResult | null>(null);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const discoverAll = useCallback(async (providers: string[] = []) => {
        setLoading(true);
        setError(null);
        try {
            const data = await invoke<DiscoveryResult>("discover_cloud_models", { providers });
            setResult(data);
            if (data.errors.length > 0) {
                console.warn("[useCloudModels] Discovery errors:", data.errors);
            }
        } catch (e: any) {
            console.error("[useCloudModels] Discovery failed:", e);
            setError(e?.toString?.() ?? "Discovery failed");
        } finally {
            setLoading(false);
        }
    }, []);

    const refreshProvider = useCallback(async (provider: string) => {
        try {
            const providerResult = await invoke<ProviderDiscoveryResult>("refresh_cloud_models", { provider });
            setResult(prev => {
                if (!prev) return prev;
                const updated = prev.providers.map(p =>
                    p.provider === provider ? providerResult : p
                );
                // If provider wasn't in the list, add it
                if (!updated.find(p => p.provider === provider)) {
                    updated.push(providerResult);
                }
                const totalModels = updated.reduce((sum, p) => sum + p.models.length, 0);
                return { ...prev, providers: updated, totalModels };
            });
        } catch (e: any) {
            console.error(`[useCloudModels] Refresh ${provider} failed:`, e);
        }
    }, []);

    // Discover on mount
    useEffect(() => {
        discoverAll();
    }, [discoverAll]);

    // Flatten all models into a single array
    const models = useMemo(() => {
        if (!result) return [];
        return result.providers.flatMap(p => p.models);
    }, [result]);

    // Group by category
    const modelsByCategory = useMemo(() => {
        const groups: Record<ModelCategory, CloudModelEntry[]> = {
            chat: [],
            embedding: [],
            tts: [],
            stt: [],
            diffusion: [],
            other: [],
        };
        for (const m of models) {
            (groups[m.category] ?? groups.other).push(m);
        }
        return groups;
    }, [models]);

    // Providers that returned results
    const providers = useMemo(() => {
        if (!result) return [];
        return result.providers;
    }, [result]);

    return {
        /** All discovered cloud models (flat array). */
        models,
        /** Models grouped by category. */
        modelsByCategory,
        /** Per-provider discovery results. */
        providers,
        /** Total model count. */
        totalModels: result?.totalModels ?? 0,
        /** Whether discovery is in progress. */
        loading,
        /** Error message if discovery completely failed. */
        error,
        /** Refresh a single provider (bypasses cache). */
        refreshProvider,
        /** Re-discover all providers. */
        refreshAll: discoverAll,
    };
}
