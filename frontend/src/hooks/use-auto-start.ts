import { useEffect, useRef } from "react";
import { commands } from "../lib/bindings";
import { useModelContext } from "../components/model-context";
import { useConfig } from "./use-config";
import { toast } from "sonner";

export function useAutoStart() {
    const { config } = useConfig();
    const { currentModelPath: modelPath, currentEmbeddingModelPath: embeddingPath, currentModelTemplate: template, maxContext, setIsRestarting, models, modelsDir } = useModelContext();
    const cleanPath = modelPath.trim();
    const cleanEmbeddingPath = embeddingPath.trim();
    // Track the last path we successfully attempted/started so we don't loop
    const lastStartedPath = useRef<string | null>(null);
    const lastStartedEmbeddingPath = useRef<string | null>(null);
    const lastStartedTemplate = useRef<string | null>(null);
    const lastStartedContext = useRef<number | null>(null);
    const lastStartedProvider = useRef<string | null>(null);

    useEffect(() => {
        if (!cleanPath) return;

        // Skip if using a cloud provider
        if (config?.selected_chat_provider && config.selected_chat_provider !== "local") {
            console.log("[AutoStart] Cloud provider selected, skipping local init.");
            lastStartedProvider.current = config.selected_chat_provider;
            setIsRestarting(false);
            return;
        }

        // If we already started this exact local configuration, just ensure we're not stuck in restarting state
        if (lastStartedPath.current === cleanPath &&
            lastStartedEmbeddingPath.current === cleanEmbeddingPath &&
            lastStartedTemplate.current === template &&
            lastStartedContext.current === maxContext &&
            lastStartedProvider.current === "local"
        ) {
            setIsRestarting(false);
            return;
        }

        const init = async () => {
            console.log("[AutoStart] Initializing Local AI:", cleanPath);

            try {
                // Ensure UI is blocked during init
                setIsRestarting(true);

                // Check if valid
                const isValid = await commands.checkModelPath(cleanPath);
                if (!isValid) {
                    setIsRestarting(false);
                    console.warn("[AutoStart] Invalid model path:", cleanPath);
                    toast.error("Model path invalid", {
                        description: "Check your model path in Settings.",
                        id: "model-path-error"
                    });
                    return;
                }

                await commands.getSidecarStatus();
                const toastId = toast.loading(`Waking up ${cleanPath.split(/[\\/]/).pop()}...`, {
                    description: `Context: ${maxContext} tokens`
                });

                // Determine mmproj path
                let mmprojPath: string | null = null;
                const modelDef = models.find(m => m.variants.some(v => cleanPath.endsWith(v.filename)));
                if (modelDef && modelDef.mmproj) {
                    const slash = cleanPath.lastIndexOf('/');
                    const backslash = cleanPath.lastIndexOf('\\');
                    const separatorIndex = Math.max(slash, backslash);
                    if (separatorIndex !== -1) {
                        const dir = cleanPath.substring(0, separatorIndex);
                        mmprojPath = `${dir}/${modelDef.mmproj.filename}`;
                    } else if (modelsDir) {
                        mmprojPath = `${modelsDir}/${modelDef.mmproj.filename}`;
                    }
                }

                await commands.startChatServer(cleanPath, maxContext, template, mmprojPath, false, false, false);

                // Track success
                lastStartedPath.current = cleanPath;
                lastStartedEmbeddingPath.current = cleanEmbeddingPath;
                lastStartedTemplate.current = template;
                lastStartedContext.current = maxContext;
                lastStartedProvider.current = "local";

                toast.success("AI Ready to chat", {
                    id: toastId,
                    description: `Server online with ${maxContext} ctx.`
                });

            } catch (e) {
                console.error("[AutoStart] Failed:", e);
                toast.error("Server Start Failed", { description: String(e) });
            } finally {
                // Always unlock UI after attempt
                setIsRestarting(false);
            }
        };

        const timer = setTimeout(init, 500); // 500ms feel faster
        return () => {
            clearTimeout(timer);
        };
    }, [cleanPath, cleanEmbeddingPath, template, maxContext, models, modelsDir, config?.selected_chat_provider]);
}
