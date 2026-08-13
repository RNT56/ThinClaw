import type { EngineInfo } from "./bindings";
import { directCommands } from "./generated/direct-commands";

export type LocalChatLaunchKind = "llamacpp-sidecar" | "engine-manager" | "unavailable";
export const LOCAL_CHAT_RUNTIME_RESTART_EVENT =
    "thinclaw-local-chat-runtime-restart";

export function requestLocalChatRuntimeRestart(): void {
    window.dispatchEvent(new Event(LOCAL_CHAT_RUNTIME_RESTART_EVENT));
}

export function localChatLaunchKind(
    engine: Pick<EngineInfo, "id" | "available"> | null,
): LocalChatLaunchKind {
    if (!engine?.available || engine.id === "none") return "unavailable";
    return engine.id === "llamacpp" ? "llamacpp-sidecar" : "engine-manager";
}

/**
 * Ollama owns its model store and accepts model identifiers such as
 * `qwen3:8b`. Every bundled runtime instead receives an inventory-backed path
 * under ThinClaw's managed model directory.
 */
export function localChatUsesManagedModelPath(
    engine: Pick<EngineInfo, "id" | "available"> | null,
): boolean {
    return localChatLaunchKind(engine) !== "unavailable"
        && engine?.id !== "ollama";
}

export interface StartLocalChatRuntimeOptions {
    engine: Pick<EngineInfo, "id" | "available"> | null;
    modelPath: string;
    contextSize: number;
    template?: string | null;
    mmproj?: string | null;
    mlock?: boolean;
    quantizeKv?: boolean;
}

/**
 * Stop both possible owners of the local chat endpoint.
 *
 * Build variants still register both commands, and attempting both prevents a
 * provider switch from leaving an old model resident because the frontend
 * guessed the compile-time owner incorrectly.
 */
export async function stopLocalChatRuntime(): Promise<void> {
    const [sidecar, engine] = await Promise.allSettled([
        directCommands.directRuntimeStopChatServer(""),
        directCommands.directRuntimeStopEngine(),
    ]);
    const errors: string[] = [];
    if (sidecar.status === "rejected") {
        errors.push(
            sidecar.reason instanceof Error
                ? sidecar.reason.message
                : String(sidecar.reason),
        );
    }
    if (engine.status === "rejected") {
        errors.push(
            engine.reason instanceof Error
                ? engine.reason.message
                : String(engine.reason),
        );
    }
    if (errors.length > 0) {
        throw new Error(errors.join("; "));
    }
}

/**
 * Start the compiled local runtime through its authoritative launcher.
 * GGUF uses the bundled llama.cpp sidecar; directory runtimes and Ollama are
 * owned by EngineManager.
 */
export async function startLocalChatRuntime({
    engine,
    modelPath,
    contextSize,
    template = null,
    mmproj = null,
    mlock = false,
    quantizeKv = false,
}: StartLocalChatRuntimeOptions): Promise<void> {
    if (!modelPath) throw new Error("No local chat model is selected");

    const launchKind = localChatLaunchKind(engine);
    if (launchKind === "unavailable") {
        throw new Error("This build has no available local inference runtime");
    }
    if (launchKind === "llamacpp-sidecar") {
        await directCommands.directRuntimeStartChatServer(
            modelPath,
            contextSize,
            template,
            mmproj,
            false,
            mlock,
            quantizeKv,
        );
        return;
    }

    // Ollama is an external daemon and owns its own model store. The
    // provisioning command intentionally rejects engines with a configured
    // base URL, so only bundled directory runtimes should pass through it.
    if (localChatUsesManagedModelPath(engine)) {
        await directCommands.directRuntimeEnsureEngineReady();
    }
    await directCommands.directRuntimeStartEngine(modelPath, contextSize);
}
