import { describe, expect, it } from "vitest";
import { isVisionCapable } from "./vision";

describe("isVisionCapable", () => {
    it("uses managed task and companion metadata instead of filenames", () => {
        expect(isVisionCapable(
            "/models/LLM/unusual-name/model.gguf",
            {
                source: "huggingface",
                task: "vision",
                companion_path: "/models/LLM/unusual-name/mmproj.gguf",
                compatible: true,
            },
        )).toBe(true);
        expect(isVisionCapable(
            "/models/LLM/gemma-chat/model.gguf",
            {
                source: "huggingface",
                task: "chat",
                companion_path: null,
                compatible: true,
            },
        )).toBe(false);
        expect(isVisionCapable(
            "/models/LLM/vision/model.gguf",
            {
                source: "huggingface",
                task: "vision",
                companion_path: null,
                compatible: false,
            },
        )).toBe(false);
    });

    it("retains filename inference only for legacy installs", () => {
        expect(isVisionCapable(
            "/models/LLM/qwen3-vl/model.gguf",
            { source: "legacy", task: null },
        )).toBe(true);
        expect(isVisionCapable(
            "/models/LLM/plain/model.gguf",
            { source: "legacy", task: null },
        )).toBe(false);
    });
});
