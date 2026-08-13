import { describe, expect, it, vi } from "vitest";
import { reconcileSummarizerRuntime } from "./summarizer-runtime";

describe("summarizer runtime reconciliation", () => {
    it("persists the path only after the backend accepts and readies it", async () => {
        const events: string[] = [];
        const start = vi.fn(async () => {
            events.push("started");
            return null;
        });
        const persistSelection = vi.fn(() => {
            events.push("persisted");
        });

        await reconcileSummarizerRuntime({
            modelPath: "/managed/LLM/model/model.gguf",
            contextSize: 32768,
            start,
            persistSelection,
        });

        expect(start).toHaveBeenCalledWith(
            "/managed/LLM/model/model.gguf",
            32768,
        );
        expect(persistSelection).toHaveBeenCalledWith(
            "/managed/LLM/model/model.gguf",
        );
        expect(events).toEqual(["started", "persisted"]);
    });

    it("does not persist when the command client rejects a backend error", async () => {
        const persistSelection = vi.fn();

        await expect(reconcileSummarizerRuntime({
            modelPath: "/managed/LLM/model/model.gguf",
            contextSize: 32768,
            start: vi.fn().mockRejectedValue(new Error("model rejected")),
            persistSelection,
        })).rejects.toThrow(/model rejected/);

        expect(persistSelection).not.toHaveBeenCalled();
    });

    it("does not persist after a rejected command transport", async () => {
        const persistSelection = vi.fn();

        await expect(reconcileSummarizerRuntime({
            modelPath: "/managed/LLM/model/model.gguf",
            contextSize: 32768,
            start: vi.fn().mockRejectedValue(new Error("IPC unavailable")),
            persistSelection,
        })).rejects.toThrow("IPC unavailable");

        expect(persistSelection).not.toHaveBeenCalled();
    });
});
