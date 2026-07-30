import { describe, expect, it, vi } from "vitest";
import {
    chooseInstalledOllamaModel,
    isInstalledOllamaModel,
    loadInstalledOllamaModels,
} from "./ollama-models";

describe("Ollama model workflow", () => {
    it("loads, sorts, and deduplicates backend-authoritative identifiers", async () => {
        const client = {
            directRuntimeListOllamaModels: vi.fn().mockResolvedValue({
                status: "ok",
                data: ["qwen3:8b", "gemma3:4b", "qwen3:8b"],
            }),
        };

        await expect(loadInstalledOllamaModels(client)).resolves.toEqual([
            "gemma3:4b",
            "qwen3:8b",
        ]);
    });

    it("propagates an actionable backend listing failure", async () => {
        const client = {
            directRuntimeListOllamaModels: vi.fn().mockResolvedValue({
                status: "error",
                error: { message: "Start Ollama, then refresh." },
            }),
        };

        await expect(loadInstalledOllamaModels(client))
            .rejects.toThrow("Start Ollama, then refresh.");
    });

    it("keeps a valid configured model and replaces a stale one deterministically", () => {
        const installed = ["gemma3:4b", "qwen3:8b"];

        expect(chooseInstalledOllamaModel(installed, "qwen3:8b")).toBe("qwen3:8b");
        expect(chooseInstalledOllamaModel(installed, "removed:latest")).toBe("gemma3:4b");
        expect(chooseInstalledOllamaModel([], "removed:latest")).toBeNull();
        expect(isInstalledOllamaModel(installed, "gemma3:4b")).toBe(true);
        expect(isInstalledOllamaModel(installed, "removed:latest")).toBe(false);
    });
});
