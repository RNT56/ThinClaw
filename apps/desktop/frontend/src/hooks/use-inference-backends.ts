/**
 * useInferenceBackends — Lightweight hook to query which backend is active
 * per modality (chat, tts, stt, embedding, diffusion).
 *
 * Used by ChatInput (STT badge), MessageBubble (TTS badge), etc.
 *
 * Unlike InferenceModeTab which fetches the full list, this hook exposes
 * just the active backend info per modality for display purposes.
 */

import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface BackendInfo {
    id: string;
    displayName: string;
    isLocal: boolean;
    modelId: string | null;
    available: boolean;
}

export type Modality = "chat" | "embedding" | "tts" | "stt" | "diffusion";

interface ModalityBackends {
    modality: Modality;
    active: BackendInfo | null;
    available: BackendInfo[];
}

export function useInferenceBackends() {
    const [backends, setBackends] = useState<Record<Modality, BackendInfo | null>>({
        chat: null,
        embedding: null,
        tts: null,
        stt: null,
        diffusion: null,
    });
    const [loaded, setLoaded] = useState(false);

    const load = useCallback(async () => {
        try {
            const data = await invoke<ModalityBackends[]>("get_inference_backends");
            const map: Record<string, BackendInfo | null> = {};
            for (const mb of data) {
                map[mb.modality] = mb.active;
            }
            setBackends(map as Record<Modality, BackendInfo | null>);
            setLoaded(true);
        } catch (e) {
            // InferenceRouter may not be initialized yet — that's fine
            console.debug("[useInferenceBackends] Not available yet:", e);
        }
    }, []);

    useEffect(() => {
        load();
    }, [load]);

    return {
        /** Active backend for each modality (null = not configured). */
        backends,
        /** Whether the data has been loaded at least once. */
        loaded,
        /** Re-fetch backend status. */
        refresh: load,

        // Convenience getters
        /** Active STT backend info. */
        stt: backends.stt,
        /** Active TTS backend info. */
        tts: backends.tts,
        /** Active chat backend info. */
        chat: backends.chat,
    };
}
