import { describe, expect, it } from "vitest";
import {
    buildModelDeactivationPlan,
    buildModelLibraryCategories,
    getVisionSelectionState,
    isModelPathAffectedByRemoval,
    isLocalSummarizerCandidate,
    isModelOnDiskInLibrary,
    modelRemovalPaths,
    normalizeModelLibraryCategory,
    selectedModelRolesForRemoval,
    selectedRolesForModelRemoval,
    shouldIncludeCuratedEntryInMyModels,
    supportsLocalSummarizer,
} from "./model-library-view";

describe("My Models inventory boundary", () => {
    it("excludes curated local catalog entries regardless of download claims", () => {
        expect(shouldIncludeCuratedEntryInMyModels({ category: "LLM" })).toBe(false);
        expect(shouldIncludeCuratedEntryInMyModels({ category: "Embedding" })).toBe(false);
    });

    it("retains cloud catalog entries because they have no local inventory record", () => {
        expect(shouldIncludeCuratedEntryInMyModels({ category: "Cloud" })).toBe(true);
    });

    it("does not route cloud catalog cards through local file actions", () => {
        expect(isModelOnDiskInLibrary({ isLocal: false })).toBe(false);
        expect(isModelOnDiskInLibrary({ isLocal: true })).toBe(true);
    });
});

describe("My Models category navigation", () => {
    it("derives runtime-owned categories and llama.cpp-only surfaces", () => {
        expect(buildModelLibraryCategories({
            hasAnyCloud: true,
            isLlamaCpp: true,
            summarizerSupported: true,
            supportedCapabilities: ["chat", "embedding", "tts"],
        })).toEqual([
            "All",
            "Cloud Brains",
            "Chat",
            "Summarizer",
            "Embedding",
            "TTS",
            "Standard",
        ]);
    });

    it("returns to All when a runtime switch removes the active category", () => {
        const categories = buildModelLibraryCategories({
            hasAnyCloud: false,
            isLlamaCpp: false,
            summarizerSupported: false,
            supportedCapabilities: ["chat"],
        });

        expect(normalizeModelLibraryCategory("Standard", categories)).toBe("All");
        expect(normalizeModelLibraryCategory("Chat", categories)).toBe("Chat");
    });
});

describe("local summarizer acceptance", () => {
    const llamaCppEngine = {
        id: "llamacpp",
        available: true,
        single_file_model: true,
    };
    const llamaCppRuntime = {
        kind: "llama_cpp",
        supportedCapabilities: ["chat", "embedding"],
    };

    it("requires matching, available llama.cpp engine and runtime capability", () => {
        expect(supportsLocalSummarizer(llamaCppEngine, llamaCppRuntime)).toBe(true);
        expect(supportsLocalSummarizer(
            { ...llamaCppEngine, available: false },
            llamaCppRuntime,
        )).toBe(false);
        expect(supportsLocalSummarizer(
            { ...llamaCppEngine, id: "mlx", single_file_model: false },
            { ...llamaCppRuntime, kind: "mlx" },
        )).toBe(false);
        expect(supportsLocalSummarizer(
            { ...llamaCppEngine, id: "vllm", single_file_model: false },
            { ...llamaCppRuntime, kind: "vllm" },
        )).toBe(false);
        expect(supportsLocalSummarizer(
            { ...llamaCppEngine, id: "ollama", single_file_model: false },
            { ...llamaCppRuntime, kind: "ollama" },
        )).toBe(false);
        expect(supportsLocalSummarizer(
            llamaCppEngine,
            { ...llamaCppRuntime, supportedCapabilities: ["embedding"] },
        )).toBe(false);
    });

    it("offers only exact compatible LLM inventory entries", () => {
        const model = {
            isLocal: true,
            localPath: "/managed/LLM/model/model.gguf",
            managedCategory: "LLM",
            compatible: true,
        };

        expect(isLocalSummarizerCandidate(model, true)).toBe(true);
        expect(isLocalSummarizerCandidate({ ...model, compatible: false }, true))
            .toBe(false);
        expect(isLocalSummarizerCandidate(
            { ...model, managedCategory: "Embedding" },
            true,
        )).toBe(false);
        expect(isLocalSummarizerCandidate({ ...model, localPath: null }, true))
            .toBe(false);
        expect(isLocalSummarizerCandidate(model, false)).toBe(false);
    });
});

describe("model selection and deletion state", () => {
    it("keeps a stale vision preference deactivatable without marking it operational", () => {
        expect(getVisionSelectionState(
            "/models/vision.gguf",
            "/models/chat.gguf",
            "/models/vision.gguf",
        )).toEqual({
            selected: true,
            operational: false,
        });
    });

    it("marks vision operational only when chat and vision use the same model", () => {
        expect(getVisionSelectionState(
            "/models/vision.gguf",
            "/models/vision.gguf",
            "/models/vision.gguf",
        )).toEqual({
            selected: true,
            operational: true,
        });
    });

    it("finds every role that would retain a deleted primary or companion path", () => {
        const removedPaths = modelRemovalPaths({
            path: "/models/install/model.gguf",
            companion_path: "/models/install/mmproj.gguf",
            relative_path: "LLM/install/model.gguf",
            install_root: "LLM/install",
        });

        expect(selectedRolesForModelRemoval(removedPaths, {
            chat: "/models/install/model.gguf",
            embedding: "",
            vision: "/models/install/mmproj.gguf",
            stt: "",
            diffusion: "",
            summarizer: "LLM/install/model.gguf",
        })).toEqual(["chat", "vision", "summarizer"]);
        expect(selectedModelRolesForRemoval(removedPaths, {
            chat: "/models/install/model.gguf",
            embedding: "",
            vision: "/models/install/mmproj.gguf",
            stt: "",
            diffusion: "",
            summarizer: "LLM/install/model.gguf",
        })).toEqual({
            chat: true,
            embedding: false,
            vision: true,
            stt: false,
            image: false,
            summarizer: true,
        });
    });

    it("treats canonical and relative children of an invalid install root as affected", () => {
        const removedPaths = modelRemovalPaths({
            path: "/models/LLM/invalid-install",
            relative_path: "LLM/invalid-install",
            install_root: "LLM/invalid-install",
        });
        const selections = {
            chat: "/models/LLM/invalid-install/model.gguf",
            embedding: "",
            vision: "",
            stt: "",
            diffusion: "",
            summarizer: "LLM/invalid-install/model.gguf",
        };

        expect(selectedRolesForModelRemoval(removedPaths, selections))
            .toEqual(["chat", "summarizer"]);
        expect(selectedModelRolesForRemoval(removedPaths, selections)).toMatchObject({
            chat: true,
            summarizer: true,
        });
    });

    it("does not affect sibling paths that only share the install-root prefix", () => {
        const removedPaths = new Set([
            "/models/LLM/model",
            "LLM/model",
            "C:\\models\\LLM\\model",
        ]);

        expect(isModelPathAffectedByRemoval(
            "/models/LLM/model-backup/model.gguf",
            removedPaths,
        )).toBe(false);
        expect(isModelPathAffectedByRemoval(
            "LLM/model-backup/model.gguf",
            removedPaths,
        )).toBe(false);
        expect(isModelPathAffectedByRemoval(
            "C:\\models\\LLM\\model-backup\\model.gguf",
            removedPaths,
        )).toBe(false);
        expect(isModelPathAffectedByRemoval(
            "C:\\models\\LLM\\model\\model.gguf",
            removedPaths,
        )).toBe(true);
    });
});

describe("model deactivation planning", () => {
    const noRoles = {
        chat: false,
        embedding: false,
        vision: false,
        summarizer: false,
        stt: false,
        image: false,
    };

    it("stops only the chat sidecar for a single-file chat model", () => {
        expect(buildModelDeactivationPlan(
            { ...noRoles, chat: true, vision: true },
            { id: "llamacpp", single_file_model: true },
        )).toEqual({
            hasSelection: true,
            stopEngine: false,
            deactivateServices: true,
            services: {
                chat: true,
                embedding: false,
                summarizer: false,
                stt: false,
                image: false,
            },
            clearSelections: {
                ...noRoles,
                chat: true,
                vision: true,
            },
        });
    });

    it("also stops a directory-backed engine only when its chat model is selected", () => {
        expect(buildModelDeactivationPlan(
            { ...noRoles, chat: true },
            { id: "mlx", single_file_model: false },
        ).stopEngine).toBe(true);
        expect(buildModelDeactivationPlan(
            { ...noRoles, embedding: true },
            { id: "mlx", single_file_model: false },
        ).stopEngine).toBe(false);
    });

    it("never treats Ollama or a cloud-only build as an owned engine process", () => {
        expect(buildModelDeactivationPlan(
            { ...noRoles, chat: true },
            { id: "ollama", single_file_model: false },
        ).stopEngine).toBe(false);
        expect(buildModelDeactivationPlan(
            { ...noRoles, chat: true },
            { id: "none", single_file_model: false },
        ).stopEngine).toBe(false);
    });

    it("clears a stale vision-only preference without stopping unrelated services", () => {
        expect(buildModelDeactivationPlan(
            { ...noRoles, vision: true },
            { id: "llamacpp", single_file_model: true },
        )).toEqual({
            hasSelection: true,
            stopEngine: false,
            deactivateServices: false,
            services: {
                chat: false,
                embedding: false,
                summarizer: false,
                stt: false,
                image: false,
            },
            clearSelections: {
                ...noRoles,
                vision: true,
            },
        });
    });

    it("preserves exact role isolation for auxiliary services", () => {
        const plan = buildModelDeactivationPlan(
            {
                ...noRoles,
                embedding: true,
                summarizer: true,
                image: true,
            },
            { id: "vllm", single_file_model: false },
        );

        expect(plan.services).toEqual({
            chat: false,
            embedding: true,
            summarizer: true,
            stt: false,
            image: true,
        });
        expect(plan.deactivateServices).toBe(true);
        expect(plan.stopEngine).toBe(false);
    });
});
