import { describe, expect, it, vi } from "vitest";
import { loadInitialModelState } from "./model-initialization";

describe("loadInitialModelState", () => {
    it("starts inventory loading before hardware telemetry and preserves both results", async () => {
        const calls: string[] = [];
        const refreshInventory = vi.fn(async () => {
            calls.push("inventory");
            return [{ path: "/models/chat" }];
        });
        const getSystemSpecs = vi.fn(async () => {
            calls.push("specs");
            return { totalMemory: 16 };
        });

        const result = await loadInitialModelState({
            refreshInventory,
            getSystemSpecs,
        });

        expect(calls).toEqual(["inventory", "specs"]);
        expect(result).toEqual({
            inventory: [{ path: "/models/chat" }],
            specs: { totalMemory: 16 },
            specsError: null,
        });
    });

    it.each(["null", "failure"] as const)(
        "still loads inventory when hardware telemetry returns %s",
        async outcome => {
            const refreshInventory = vi.fn().mockResolvedValue([
                { path: "/models/chat" },
            ]);
            const hardwareError = new Error("telemetry unavailable");
            const getSystemSpecs = outcome === "null"
                ? vi.fn().mockResolvedValue(null)
                : vi.fn().mockRejectedValue(hardwareError);

            const result = await loadInitialModelState({
                refreshInventory,
                getSystemSpecs,
            });

            expect(refreshInventory).toHaveBeenCalledTimes(1);
            expect(result.inventory).toEqual([{ path: "/models/chat" }]);
            expect(result.specs).toBeNull();
            expect(result.specsError).toBe(
                outcome === "failure" ? hardwareError : null,
            );
        },
    );
});
