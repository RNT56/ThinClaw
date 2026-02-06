import { useEffect, useRef } from "react";
import { commands } from "../lib/bindings";
import { useModelContext } from "../components/model-context";
import { toast } from "sonner";

export function useAutoStart() {
    const { currentModelPath: modelPath, currentEmbeddingModelPath: embeddingPath, currentModelTemplate: template, maxContext, setIsRestarting, models, modelsDir } = useModelContext();
    const cleanPath = modelPath.trim();
    const cleanEmbeddingPath = embeddingPath.trim();
    // Track the last path we successfully attempted/started so we don't loop
    const lastStartedPath = useRef<string | null>(null);
    const lastStartedEmbeddingPath = useRef<string | null>(null);
    const lastStartedTemplate = useRef<string | null>(null);
    const lastStartedContext = useRef<number | null>(null);

    useEffect(() => {
        if (!cleanPath) return;

        // If we already started this exact configuration, skip
        if (lastStartedPath.current === cleanPath &&
            lastStartedEmbeddingPath.current === cleanEmbeddingPath &&
            lastStartedTemplate.current === template &&
            lastStartedContext.current === maxContext
        ) return;

        const init = async () => {
            console.log("[AutoStart] Triggered for context:", maxContext);
            lastStartedPath.current = cleanPath;
            lastStartedEmbeddingPath.current = cleanEmbeddingPath;
            lastStartedTemplate.current = template;
            lastStartedContext.current = maxContext;

            try {
                setIsRestarting(true);
                // Check if valid
                const isValid = await commands.checkModelPath(cleanPath);
                if (!isValid) {
                    setIsRestarting(false);
                    console.warn("[AutoStart] Invalid model path:", cleanPath);
                    toast.error("Model path invalid", {
                        description: "Please select a valid model in Settings."
                    });
                    return;
                }

                await commands.getSidecarStatus();

                const toastId = toast.loading(`Starting Local AI (${maxContext} tokens)...`);

                // Determine mmproj path
                let mmprojPath: string | null = null;
                const modelDef = models.find(m => m.variants.some(v => cleanPath.endsWith(v.filename)));
                if (modelDef && modelDef.mmproj) {
                    // Try to infer directory from the model path
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

                // Use user-defined context size (removing 32k enforcement to prevent OOM on low-spec machines)
                try {
                    await commands.startChatServer(cleanPath, maxContext, template, mmprojPath, false, false, false);
                    toast.success("AI Servers Ready", {
                        id: toastId,
                        description: `Chat: ${cleanPath.split('/').pop()}\nCtx: ${maxContext}`
                    });
                } catch (e) {
                    throw e; // Let the outer catch handle it
                }

                // Allow events to settle
                setTimeout(() => setIsRestarting(false), 2000);

            } catch (e) {
                setIsRestarting(false);
                console.error("[AutoStart] Failed:", e);
                // We DON'T reset lastStartedPath here anymore to prevent infinite retry loops on a bad model/path
                toast.error("Server Start Failed", {
                    description: String(e),
                });
            }
        };

        // Debounce slightly to avoid rapid switching churn and allow app hydration
        const timer = setTimeout(init, 1500);
        return () => clearTimeout(timer);
    }, [cleanPath, cleanEmbeddingPath, template, maxContext, models, modelsDir]);
}
