import { beforeEach, describe, expect, it, vi } from "vitest";

const runtimeCommands = vi.hoisted(() => ({
    ensureEngineReady: vi.fn(),
    startEngine: vi.fn(),
    launchChatSidecar: vi.fn(),
    stopChatServer: vi.fn(),
    stopEngine: vi.fn(),
}));

vi.mock("./generated/direct-commands", () => ({
    directCommands: {
        directRuntimeEnsureEngineReady: runtimeCommands.ensureEngineReady,
        directRuntimeStartEngine: runtimeCommands.startEngine,
        directRuntimeStartChatServer: runtimeCommands.launchChatSidecar,
        directRuntimeStopChatServer: runtimeCommands.stopChatServer,
        directRuntimeStopEngine: runtimeCommands.stopEngine,
    },
}));

import {
    localChatLaunchKind,
    localChatUsesManagedModelPath,
    startLocalChatRuntime,
    stopLocalChatRuntime,
} from "./local-runtime-start";

describe("local chat runtime dispatch", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        runtimeCommands.ensureEngineReady.mockResolvedValue({
            status: "ok",
            data: null,
        });
        runtimeCommands.startEngine.mockResolvedValue({
            status: "ok",
            data: { port: 11434, token: "" },
        });
        runtimeCommands.launchChatSidecar.mockResolvedValue({
            status: "ok",
            data: null,
        });
        runtimeCommands.stopChatServer.mockResolvedValue({
            status: "ok",
            data: null,
        });
        runtimeCommands.stopEngine.mockResolvedValue({
            status: "ok",
            data: null,
        });
    });

    it("routes only llama.cpp through the GGUF sidecar", () => {
        expect(localChatLaunchKind({ id: "llamacpp", available: true }))
            .toBe("llamacpp-sidecar");
    });

    it.each(["mlx", "vllm", "ollama"])(
        "routes %s through EngineManager",
        id => {
            expect(localChatLaunchKind({ id, available: true }))
                .toBe("engine-manager");
        },
    );

    it("rejects cloud-only and unavailable runtimes", () => {
        expect(localChatLaunchKind({ id: "none", available: true }))
            .toBe("unavailable");
        expect(localChatLaunchKind({ id: "mlx", available: false }))
            .toBe("unavailable");
        expect(localChatLaunchKind(null)).toBe("unavailable");
    });

    it("does not treat Ollama model identifiers as managed filesystem paths", () => {
        expect(localChatUsesManagedModelPath({ id: "llamacpp", available: true }))
            .toBe(true);
        expect(localChatUsesManagedModelPath({ id: "mlx", available: true }))
            .toBe(true);
        expect(localChatUsesManagedModelPath({ id: "vllm", available: true }))
            .toBe(true);
        expect(localChatUsesManagedModelPath({ id: "ollama", available: true }))
            .toBe(false);
        expect(localChatUsesManagedModelPath({ id: "none", available: true }))
            .toBe(false);
        expect(localChatUsesManagedModelPath(null)).toBe(false);
    });

    it("starts Ollama without invoking bundled-runtime provisioning", async () => {
        await startLocalChatRuntime({
            engine: { id: "ollama", available: true },
            modelPath: "qwen3:8b",
            contextSize: 8192,
        });

        expect(runtimeCommands.ensureEngineReady).not.toHaveBeenCalled();
        expect(runtimeCommands.startEngine).toHaveBeenCalledWith("qwen3:8b", 8192);
    });

    it("provisions bundled directory runtimes before starting them", async () => {
        await startLocalChatRuntime({
            engine: { id: "mlx", available: true },
            modelPath: "/managed/LLM/model",
            contextSize: 16384,
        });

        expect(runtimeCommands.ensureEngineReady).toHaveBeenCalledOnce();
        expect(runtimeCommands.startEngine)
            .toHaveBeenCalledWith("/managed/LLM/model", 16384);
        expect(runtimeCommands.ensureEngineReady.mock.invocationCallOrder[0])
            .toBeLessThan(runtimeCommands.startEngine.mock.invocationCallOrder[0]);
    });

    it("stops both possible local chat runtime owners", async () => {
        await stopLocalChatRuntime();

        expect(runtimeCommands.stopChatServer).toHaveBeenCalledWith("");
        expect(runtimeCommands.stopEngine).toHaveBeenCalledOnce();
    });

    it("attempts both stops and reports every failure", async () => {
        runtimeCommands.stopChatServer.mockResolvedValue({
            status: "error",
            error: { kind: "runtime", message: "sidecar failed" },
        });
        runtimeCommands.stopEngine.mockRejectedValue(new Error("engine failed"));

        await expect(stopLocalChatRuntime()).rejects.toThrow(
            /sidecar failed.*engine failed/,
        );
        expect(runtimeCommands.stopChatServer).toHaveBeenCalledOnce();
        expect(runtimeCommands.stopEngine).toHaveBeenCalledOnce();
    });
});
