import { useEffect, useRef } from "react";
import { commands } from "../lib/bindings";
import { useModelContext } from "../components/model-context";
import { useConfig } from "./use-config";
import { toast } from "sonner";

export function useAutoStart() {
    const { config } = useConfig();
    const {
        currentModelPath: modelPath,
        currentEmbeddingModelPath: embeddingPath,
        currentModelTemplate: template,
        maxContext,
        setIsRestarting,
        models,
        modelsDir,
        engineInfo,
    } = useModelContext();
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

        // For non-llamacpp engines (MLX, vLLM, Ollama): start via EngineManager
        // instead of the llama-server sidecar.  The EngineManager spawns the
        // engine's own server (e.g. mlx_lm.server) and returns the port/token
        // that resolve_provider() will use in chat.rs.
        if (engineInfo && !engineInfo.single_file_model) {
            // Deduplicate: only restart if path or context changed
            if (
                lastStartedPath.current === cleanPath &&
                lastStartedContext.current === maxContext &&
                lastStartedProvider.current === engineInfo.id
            ) {
                setIsRestarting(false);
                return;
            }

            const initEngine = async () => {
                const modelName = cleanPath.split(/[/\\]/).pop() ?? cleanPath;
                const toastId = toast.loading(`Starting ${engineInfo.display_name} with ${modelName}...`, {
                    description: `Context: ${maxContext} tokens`
                });
                try {
                    setIsRestarting(true);

                    // Validate model directory exists
                    const isValid = await commands.checkModelPath(cleanPath);
                    if (!isValid) {
                        toast.error("Model path invalid", {
                            id: toastId,
                            description: "Check your model path in Settings."
                        });
                        setIsRestarting(false);
                        return;
                    }

                    // Start the engine server (mlx_lm.server / vllm serve / etc.)
                    const result = await commands.startEngine(cleanPath, maxContext);
                    if (result.status === "error") {
                        throw new Error(result.error);
                    }

                    // Track so we don't restart unnecessarily
                    lastStartedPath.current = cleanPath;
                    lastStartedContext.current = maxContext;
                    lastStartedProvider.current = engineInfo.id;

                    toast.success(`${engineInfo.display_name} ready`, {
                        id: toastId,
                        description: `Model loaded on port ${result.data.port}`
                    });
                } catch (e) {
                    console.error(`[AutoStart] ${engineInfo.id} engine start failed:`, e);
                    toast.error(`${engineInfo.display_name} start failed`, {
                        id: toastId,
                        description: String(e)
                    });
                } finally {
                    setIsRestarting(false);
                }
            };

            const timer = setTimeout(initEngine, 500);
            return () => { clearTimeout(timer); };
        }

        // If we already started this exact local configuration, don't restart
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
            console.log("[AutoStart] Initializing Local AI (llama.cpp):", cleanPath);

            try {
                setIsRestarting(true);

                // Validate the path exists
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
                const modelName = cleanPath.split(/[/\\]/).pop() ?? cleanPath;
                const toastId = toast.loading(`Waking up ${modelName}...`, {
                    description: `Context: ${maxContext} tokens`
                });

                // Resolve mmproj path for vision models
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

                // Track successful start
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
                setIsRestarting(false);
            }
        };

        const timer = setTimeout(init, 500);
        return () => { clearTimeout(timer); };
    }, [cleanPath, cleanEmbeddingPath, template, maxContext, models, modelsDir, config?.selected_chat_provider, engineInfo]);
}
