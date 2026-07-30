import { useEffect, useRef, useState } from "react";
import { commands } from "../lib/bindings";
import { useModelContext } from "../components/model-context";
import { useConfig } from "./use-config";
import { toast } from "sonner";
import {
    localChatLaunchKind,
    localChatUsesManagedModelPath,
    LOCAL_CHAT_RUNTIME_RESTART_EVENT,
    startLocalChatRuntime,
} from "../lib/local-runtime-start";
import { isCompatibleManagedModelForCategory } from "../lib/hf-models";

export function useAutoStart() {
    const { config } = useConfig();
    const {
        currentModelPath: modelPath,
        currentEmbeddingModelPath: embeddingPath,
        currentModelTemplate: template,
        maxContext,
        setIsRestarting,
        localModels,
        engineInfo,
        refreshRuntimeSnapshot,
    } = useModelContext();
    const cleanPath = modelPath.trim();
    const cleanEmbeddingPath = embeddingPath.trim();

    // Track the last path we successfully attempted/started so we don't loop
    const lastStartedPath = useRef<string | null>(null);
    const lastStartedEmbeddingPath = useRef<string | null>(null);
    const lastStartedTemplate = useRef<string | null>(null);
    const lastStartedContext = useRef<number | null>(null);
    const lastStartedProvider = useRef<string | null>(null);
    const [forcedRestart, setForcedRestart] = useState(0);

    useEffect(() => {
        const restart = () => {
            lastStartedPath.current = null;
            lastStartedEmbeddingPath.current = null;
            lastStartedTemplate.current = null;
            lastStartedContext.current = null;
            lastStartedProvider.current = null;
            setForcedRestart(value => value + 1);
        };
        window.addEventListener(LOCAL_CHAT_RUNTIME_RESTART_EVENT, restart);
        return () =>
            window.removeEventListener(LOCAL_CHAT_RUNTIME_RESTART_EVENT, restart);
    }, []);

    useEffect(() => {
        if (!cleanPath) return;

        // Skip if using a cloud provider
        if (config?.selected_chat_provider && config.selected_chat_provider !== "local") {
            console.log("[AutoStart] Cloud provider selected, skipping local init.");
            lastStartedProvider.current = config.selected_chat_provider;
            setIsRestarting(false);
            return;
        }

        // Engine information arrives asynchronously. Starting before it is
        // known used to route MLX/vLLM/Ollama through the llama.cpp sidecar.
        if (!engineInfo) return;
        const launchKind = localChatLaunchKind(engineInfo);
        if (launchKind === "unavailable") {
            setIsRestarting(false);
            return;
        }

        // MLX, vLLM, and Ollama are owned by EngineManager. Ollama accepts an
        // external model identifier; bundled engines require an authoritative
        // compatible inventory path.
        if (launchKind === "engine-manager") {
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

                    if (localChatUsesManagedModelPath(engineInfo)) {
                        const selected = localModels.find(model =>
                            model.path === cleanPath
                            && isCompatibleManagedModelForCategory(model, "LLM")
                        );
                        const isValid = Boolean(selected)
                            && await commands.checkModelPath(cleanPath);
                        if (!isValid) {
                            toast.error("Model path invalid", {
                                id: toastId,
                                description:
                                    "Select a compatible chat model from My Models."
                            });
                            return;
                        }
                    }

                    await startLocalChatRuntime({
                        engine: engineInfo,
                        modelPath: cleanPath,
                        contextSize: maxContext,
                    });
                    await refreshRuntimeSnapshot();

                    // Track so we don't restart unnecessarily
                    lastStartedPath.current = cleanPath;
                    lastStartedContext.current = maxContext;
                    lastStartedProvider.current = engineInfo.id;

                    toast.success(`${engineInfo.display_name} ready`, {
                        id: toastId,
                        description: `Model loaded with ${maxContext} context tokens`
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

                const selectedModel = localModels.find(model =>
                    model.path === cleanPath
                    && isCompatibleManagedModelForCategory(model, "LLM")
                );
                const isValid = Boolean(selectedModel)
                    && await commands.checkModelPath(cleanPath);
                if (!isValid) {
                    setIsRestarting(false);
                    console.warn("[AutoStart] Invalid model path:", cleanPath);
                    toast.error("Model path invalid", {
                        description: "Select a compatible chat model from My Models.",
                        id: "model-path-error"
                    });
                    return;
                }

                const modelName = cleanPath.split(/[/\\]/).pop() ?? cleanPath;
                const toastId = toast.loading(`Waking up ${modelName}...`, {
                    description: `Context: ${maxContext} tokens`
                });

                await startLocalChatRuntime({
                    engine: engineInfo,
                    modelPath: cleanPath,
                    contextSize: maxContext,
                    template,
                    mmproj: selectedModel?.companion_path ?? null,
                    mlock: config?.mlock ?? false,
                    quantizeKv: config?.quantize_kv ?? false,
                });
                await refreshRuntimeSnapshot();

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
    }, [
        cleanPath,
        cleanEmbeddingPath,
        template,
        maxContext,
        localModels,
        config?.selected_chat_provider,
        config?.mlock,
        config?.quantize_kv,
        engineInfo,
        refreshRuntimeSnapshot,
        forcedRestart,
    ]);
}
