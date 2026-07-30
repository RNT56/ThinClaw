import { useCallback, useEffect, useRef, useState } from "react";
import { bridgeErrorMessage } from "../lib/command-errors";
import { directCommands } from "../lib/generated/direct-commands";
import {
    loadInstalledOllamaModels,
    type OllamaModelCommandClient,
} from "../lib/ollama-models";

export type OllamaModelsStatus = "idle" | "loading" | "ready" | "error";

// The generated bindings are refreshed from the Rust command registry during
// release preparation. Keeping this one cast local makes the temporary
// pre-generation type gap explicit without weakening the rest of the command
// surface.
const ollamaCommandClient = directCommands as typeof directCommands
    & OllamaModelCommandClient;

export function useOllamaModels(enabled: boolean) {
    const [models, setModels] = useState<string[]>([]);
    const [status, setStatus] = useState<OllamaModelsStatus>("idle");
    const [error, setError] = useState<string | null>(null);
    const generationRef = useRef(0);

    const refresh = useCallback(async (): Promise<string[] | null> => {
        const generation = ++generationRef.current;
        setStatus("loading");
        setError(null);
        try {
            const installed = await loadInstalledOllamaModels(ollamaCommandClient);
            if (generation !== generationRef.current) return null;
            setModels(installed);
            setStatus("ready");
            return installed;
        } catch (reason) {
            if (generation !== generationRef.current) return null;
            setModels([]);
            setStatus("error");
            setError(bridgeErrorMessage(reason));
            return null;
        }
    }, []);

    useEffect(() => {
        if (!enabled) {
            ++generationRef.current;
            setModels([]);
            setStatus("idle");
            setError(null);
            return;
        }
        void refresh();
        return () => {
            ++generationRef.current;
        };
    }, [enabled, refresh]);

    return { models, status, error, refresh };
}
