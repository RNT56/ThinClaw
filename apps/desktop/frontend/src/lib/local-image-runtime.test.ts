import { describe, expect, it, vi } from "vitest";
import {
    requireLocalImageModelPath,
    startLocalImageRuntime,
} from "./local-image-runtime";

describe("local image runtime", () => {
    it("rejects local generation before startup when no compatible path resolved", () => {
        expect(() => requireLocalImageModelPath("local", undefined))
            .toThrow("No compatible local image generation model");
        expect(requireLocalImageModelPath("nano-banana", undefined))
            .toBeUndefined();
    });

    it("unwraps backend startup failures and does not report readiness", async () => {
        const start = vi.fn().mockResolvedValue({
            status: "error",
            error: {
                kind: "runtime",
                message: "diffusion server failed",
            },
        });

        await expect(startLocalImageRuntime({
            modelPath: "/models/diffusion",
            start,
        })).rejects.toThrow("diffusion server failed");
        expect(start).toHaveBeenCalledWith("/models/diffusion");
    });

    it("resolves only after the backend accepts startup", async () => {
        const start = vi.fn().mockResolvedValue({
            status: "ok",
            data: null,
        });

        await expect(startLocalImageRuntime({
            modelPath: "/models/diffusion",
            start,
        })).resolves.toBeUndefined();
    });
});
